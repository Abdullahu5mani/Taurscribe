//! Playback copy of a meeting recording: one small Opus-in-WebM file.
//!
//! The raw capture is a two-channel 48 kHz float WAV (~1.4 GB per hour). It is
//! only needed while the meeting is processed (diarization, transcription,
//! voice samples); afterwards the user just listens to it. This mixes the two
//! channels to mono, levelling each so a quiet caller is as audible as the local
//! mic, and encodes 16 kbps Opus (~7 MB per hour).
//!
//! WebM rather than Ogg: every webview Taurscribe runs in plays WebM/Opus
//! (WebKit since macOS 13 / Safari 16, WebView2, WebKitGTK); Ogg only reached
//! Safari in 2025.

use std::path::{Path, PathBuf};

pub const PLAYBACK_BITRATE: i32 = 16_000;
const OPUS_RATE: u32 = 48_000;
const FRAME_MS: u64 = 20;
const CLUSTER_MS: u64 = 5_000;

/// What processing measured on the raw channels before they were discarded.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct ProcessingDiagnostics {
    pub raw_channels: u16,
    pub raw_seconds: f64,
    pub raw_bytes: u64,
    pub channel_rms: Vec<f32>,
    /// Pearson correlation of mic vs call channel (1.0 = one signal recorded twice).
    pub channel_correlation: Option<f32>,
    pub playback_path: Option<String>,
    pub playback_bytes: u64,
}

static LAST: std::sync::Mutex<Option<ProcessingDiagnostics>> = std::sync::Mutex::new(None);

/// Diagnostics of the most recently processed meeting (exposed on /api/status).
/// Tests that compress audio hold this: `last_processing_diagnostics` is global,
/// so parallel tests would read each other's results.
#[cfg(test)]
pub(crate) static COMPRESS_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn last_processing_diagnostics() -> Option<ProcessingDiagnostics> {
    LAST.lock().ok().and_then(|d| d.clone())
}

/// Writes `<wav stem>.webm` next to the WAV and returns its path. The WAV is
/// left in place; the caller removes it once the new file is recorded.
pub fn compress_for_playback(wav: &Path) -> Result<PathBuf, String> {
    let (spec, gains, channel_rms, channel_correlation, frames) = measure_wav(wav)?;

    let mut diag = ProcessingDiagnostics {
        raw_channels: spec.channels,
        raw_seconds: frames as f64 / spec.sample_rate as f64,
        raw_bytes: std::fs::metadata(wav).map(|m| m.len()).unwrap_or(0),
        channel_rms,
        channel_correlation,
        ..Default::default()
    };

    let out = wav.with_extension("webm");
    let bytes = encode_wav_stream(wav, &gains, frames, spec.sample_rate)?;
    let temp = wav.with_extension("webm.tmp");
    if let Err(error) = std::fs::write(&temp, &bytes)
        .and_then(|()| std::fs::rename(&temp, &out))
    {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("write {}: {}", out.display(), error));
    }

    diag.playback_path = Some(out.to_string_lossy().to_string());
    diag.playback_bytes = bytes.len() as u64;
    if let Ok(mut last) = LAST.lock() {
        *last = Some(diag);
    }
    Ok(out)
}

/// Swaps a processed meeting's raw WAV for its playback copy; returns the path to
/// store. On failure the WAV is kept so the meeting stays playable.
pub fn keep_playback_copy(wav: &Path) -> PathBuf {
    match compress_for_playback(wav) {
        Ok(webm) => {
            let _ = std::fs::remove_file(wav);
            webm
        }
        Err(e) => {
            eprintln!("[WARN] Keeping the raw meeting WAV, playback copy failed: {}", e);
            wav.to_path_buf()
        }
    }
}

fn for_each_wav_frame(
    path: &Path,
    mut visit: impl FnMut(&[f32]) -> Result<(), String>,
) -> Result<(hound::WavSpec, u64), String> {
    let mut r = hound::WavReader::open(path).map_err(|e| format!("open {}: {}", path.display(), e))?;
    let spec = r.spec();
    if spec.channels == 0 || spec.sample_rate == 0 {
        return Err("WAV has an invalid channel count or sample rate".into());
    }
    let channels = spec.channels as usize;
    let expected_samples = r.len() as u64;
    let mut frame = vec![0.0f32; channels];
    let mut samples = 0u64;
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for sample in r.samples::<f32>() {
                let value = sample.map_err(|e| format!("Read {} sample {}: {e}", path.display(), samples))?;
                if !value.is_finite() {
                    return Err(format!("Non-finite audio sample in {}", path.display()));
                }
                frame[samples as usize % channels] = value;
                samples += 1;
                if samples as usize % channels == 0 {
                    visit(&frame)?;
                }
            }
        }
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample.max(1) - 1)) as f32;
            for sample in r.samples::<i32>() {
                let value = sample.map_err(|e| format!("Read {} sample {}: {e}", path.display(), samples))?;
                frame[samples as usize % channels] = value as f32 / max;
                samples += 1;
                if samples as usize % channels == 0 {
                    visit(&frame)?;
                }
            }
        }
    }
    if samples != expected_samples || samples as usize % channels != 0 {
        return Err(format!("Incomplete WAV data in {}", path.display()));
    }
    Ok((spec, samples / channels as u64))
}

fn measure_wav(path: &Path) -> Result<(hound::WavSpec, Vec<f32>, Vec<f32>, Option<f32>, u64), String> {
    let spec = hound::WavReader::open(path).map_err(|e| e.to_string())?.spec();
    let channels = spec.channels as usize;
    if channels == 0 || spec.sample_rate == 0 {
        return Err("WAV has an invalid channel count or sample rate".into());
    }
    let mut sum = vec![0.0f64; channels];
    let mut sum_sq = vec![0.0f64; channels];
    let mut window_sq = vec![0.0f64; channels];
    let mut levels = vec![Vec::<f32>::new(); channels];
    let mut cross = 0.0f64;
    let mut window_count = 0u64;
    let frame_samples = (spec.sample_rate / 50).max(1) as u64;
    let (_, frames) = for_each_wav_frame(path, |frame| {
        for (i, &value) in frame.iter().enumerate() {
            let v = value as f64;
            sum[i] += v;
            sum_sq[i] += v * v;
            window_sq[i] += v * v;
        }
        if channels >= 2 {
            cross += frame[0] as f64 * frame[1] as f64;
        }
        window_count += 1;
        if window_count == frame_samples {
            for i in 0..channels {
                levels[i].push((window_sq[i] / window_count as f64).sqrt() as f32);
                window_sq[i] = 0.0;
            }
            window_count = 0;
        }
        Ok(())
    })?;
    if window_count > 0 {
        for i in 0..channels {
            levels[i].push((window_sq[i] / window_count as f64).sqrt() as f32);
        }
    }
    let channel_rms = sum_sq.iter().map(|v| (v / frames.max(1) as f64).sqrt() as f32).collect();
    let correlation = if channels >= 2 && frames > 0 {
        let n = frames as f64;
        let covariance = cross - sum[0] * sum[1] / n;
        let a = sum_sq[0] - sum[0] * sum[0] / n;
        let b = sum_sq[1] - sum[1] * sum[1] / n;
        Some(if a > 0.0 && b > 0.0 { (covariance / (a * b).sqrt()) as f32 } else { 0.0 })
    } else {
        None
    };
    let gains = levels.iter_mut().map(|channel| {
        channel.sort_by(|a, b| a.total_cmp(b));
        let level = channel.get((channel.len().saturating_sub(1)) * 95 / 100).copied().unwrap_or(0.0);
        if level < 1e-4 { 1.0 } else { (0.08 / level).clamp(0.25, 12.0) }
    }).collect();
    Ok((spec, gains, channel_rms, correlation, frames))
}

fn soft_limit(x: f32) -> f32 {
    const KNEE: f32 = 0.8;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        x.signum() * (KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh())
    }
}

// ── Opus + WebM ──────────────────────────────────────────────────────────────

fn encode_wav_stream(path: &Path, gains: &[f32], expected_frames: u64, rate: u32) -> Result<Vec<u8>, String> {
    let mut enc = opus::Encoder::new(OPUS_RATE, opus::Channels::Mono, opus::Application::Voip)
        .map_err(|e| format!("opus encoder: {}", e))?;
    enc.set_bitrate(opus::Bitrate::Bits(PLAYBACK_BITRATE)).map_err(|e| format!("opus bitrate: {}", e))?;
    let pre_skip = enc.get_lookahead().unwrap_or(312).max(0) as u16;

    let frame = (OPUS_RATE as u64 * FRAME_MS / 1000) as usize;
    let mut packets: Vec<Vec<u8>> = Vec::new();
    let mut buf = vec![0u8; 4000];
    let mut pcm = Vec::with_capacity(frame);
    let mut emit = |sample: f32| -> Result<(), String> {
        pcm.push(sample);
        if pcm.len() == frame {
            let n = enc.encode_float(&pcm, &mut buf).map_err(|e| format!("opus encode: {e}"))?;
            packets.push(buf[..n].to_vec());
            pcm.clear();
        }
        Ok(())
    };
    let mut source_index = 0u64;
    let mut output_index = 0u64;
    let mut previous = 0.0f32;
    let target_len = expected_frames.saturating_mul(OPUS_RATE as u64) / rate as u64;
    let (observed_spec, observed_frames) = for_each_wav_frame(path, |channels| {
        let mixed = soft_limit(channels.iter().zip(gains).map(|(v, gain)| v * gain).sum());
        if source_index == 0 {
            if target_len > 0 {
                emit(mixed)?;
                output_index = 1;
            }
        } else {
            while output_index < target_len
                && (output_index as f64 * rate as f64 / OPUS_RATE as f64) <= source_index as f64
            {
                let pos = output_index as f64 * rate as f64 / OPUS_RATE as f64;
                let t = (pos - (source_index - 1) as f64) as f32;
                emit(previous + (mixed - previous) * t)?;
                output_index += 1;
            }
        }
        previous = mixed;
        source_index += 1;
        Ok(())
    })?;
    if observed_frames != expected_frames
        || observed_spec.sample_rate != rate
        || observed_spec.channels as usize != gains.len()
    {
        return Err(format!("WAV changed during conversion: {}", path.display()));
    }
    while output_index < target_len {
        emit(previous)?;
        output_index += 1;
    }
    // The decoder drops `pre_skip` samples; pad so the tail is not cut off.
    for _ in 0..pre_skip {
        emit(0.0)?;
    }
    drop(emit);
    if !pcm.is_empty() {
        pcm.resize(frame, 0.0);
        let n = enc.encode_float(&pcm, &mut buf).map_err(|e| format!("opus encode: {e}"))?;
        packets.push(buf[..n].to_vec());
    }
    let duration_ms = target_len as f64 * 1000.0 / OPUS_RATE as f64;
    Ok(webm(&packets, pre_skip, duration_ms))
}

/// EBML variable-length size.
fn vint_size(n: u64) -> Vec<u8> {
    for len in 1..=8u32 {
        if n < (1u64 << (7 * len)) - 1 {
            let mut v = n | (1u64 << (7 * len));
            let mut out = vec![0u8; len as usize];
            for i in (0..len as usize).rev() {
                out[i] = (v & 0xFF) as u8;
                v >>= 8;
            }
            return out;
        }
    }
    unreachable!("EBML size too large")
}

fn id_bytes(id: u32) -> Vec<u8> {
    let b = id.to_be_bytes();
    let skip = b.iter().position(|&x| x != 0).unwrap_or(3);
    b[skip..].to_vec()
}

fn el(id: u32, body: &[u8]) -> Vec<u8> {
    let mut out = id_bytes(id);
    out.extend(vint_size(body.len() as u64));
    out.extend_from_slice(body);
    out
}

fn uint(id: u32, v: u64) -> Vec<u8> {
    let b = v.to_be_bytes();
    let skip = b.iter().position(|&x| x != 0).unwrap_or(7);
    el(id, &b[skip..])
}

/// Fixed 8-byte unsigned: keeps an element's size independent of its value.
fn uint8(id: u32, v: u64) -> Vec<u8> {
    el(id, &v.to_be_bytes())
}

fn float(id: u32, v: f64) -> Vec<u8> {
    el(id, &v.to_be_bytes())
}

fn string(id: u32, s: &str) -> Vec<u8> {
    el(id, s.as_bytes())
}

fn opus_head(pre_skip: u16) -> Vec<u8> {
    let mut h = b"OpusHead".to_vec();
    h.push(1); // version
    h.push(1); // channels
    h.extend(pre_skip.to_le_bytes());
    h.extend(OPUS_RATE.to_le_bytes());
    h.extend(0i16.to_le_bytes()); // output gain
    h.push(0); // mapping family
    h
}

fn webm(packets: &[Vec<u8>], pre_skip: u16, duration_ms: f64) -> Vec<u8> {
    let ebml_header = el(
        0x1A45DFA3,
        &[
            uint(0x4286, 1),
            uint(0x42F7, 1),
            uint(0x42F2, 4),
            uint(0x42F3, 8),
            string(0x4282, "webm"),
            uint(0x4287, 4),
            uint(0x4285, 2),
        ]
        .concat(),
    );

    let info = el(
        0x1549A966,
        &[
            uint(0x2AD7B1, 1_000_000), // TimecodeScale: 1 ms
            float(0x4489, duration_ms),
            string(0x4D80, "Taurscribe"),
            string(0x5741, "Taurscribe"),
        ]
        .concat(),
    );

    let tracks = el(
        0x1654AE6B,
        &el(
            0xAE,
            &[
                uint(0xD7, 1),
                uint(0x73C5, 1),
                uint(0x83, 2), // audio
                string(0x86, "A_OPUS"),
                el(0x63A2, &opus_head(pre_skip)),
                uint(0x56AA, pre_skip as u64 * 1_000_000_000 / OPUS_RATE as u64), // CodecDelay (ns)
                uint(0x56BB, 80_000_000),                                          // SeekPreRoll (ns)
                el(0xE1, &[float(0xB5, OPUS_RATE as f64), uint(0x9F, 1)].concat()),
            ]
            .concat(),
        ),
    );

    // Clusters of CLUSTER_MS, each opening with a keyframe-flagged block.
    let per_cluster = (CLUSTER_MS / FRAME_MS) as usize;
    let clusters: Vec<(u64, Vec<u8>)> = packets
        .chunks(per_cluster)
        .enumerate()
        .map(|(ci, chunk)| {
            let t0 = ci as u64 * CLUSTER_MS;
            let mut body = uint(0xE7, t0);
            for (i, p) in chunk.iter().enumerate() {
                let rel = (i as u64 * FRAME_MS) as i16;
                let mut block = vec![0x81]; // track 1
                block.extend(rel.to_be_bytes());
                block.push(0x80); // keyframe (every Opus packet is independently decodable)
                block.extend_from_slice(p);
                body.extend(el(0xA3, &block));
            }
            (t0, el(0x1F43B675, &body))
        })
        .collect();

    // Cues sit before the clusters so a player can seek without scanning; their
    // positions use fixed-width numbers, so the cues' size is known up front.
    let cue_point = |t: u64, pos: u64| {
        el(0xBB, &[uint8(0xB3, t), el(0xB7, &[uint(0xF7, 1), uint8(0xF1, pos)].concat())].concat())
    };
    let cues_len = el(0x1C53BB6B, &clusters.iter().map(|(t, _)| cue_point(*t, 0)).collect::<Vec<_>>().concat()).len();
    let mut pos = (info.len() + tracks.len() + cues_len) as u64;
    let mut points = Vec::new();
    for (t, c) in &clusters {
        points.push(cue_point(*t, pos));
        pos += c.len() as u64;
    }
    let cues = el(0x1C53BB6B, &points.concat());
    debug_assert_eq!(cues.len(), cues_len);

    let mut segment_body = Vec::with_capacity(pos as usize + 64);
    segment_body.extend(info);
    segment_body.extend(tracks);
    segment_body.extend(cues);
    for (_, c) in clusters {
        segment_body.extend(c);
    }
    let mut out = ebml_header;
    out.extend(el(0x18538067, &segment_body));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vint_sizes() {
        assert_eq!(vint_size(0), vec![0x80]);
        assert_eq!(vint_size(126), vec![0xFE]);
        assert_eq!(vint_size(127), vec![0x40, 0x7F]);
        assert_eq!(vint_size(1000), vec![0x43, 0xE8]);
    }

    #[test]
    fn test_playback_levels_quiet_caller() {
        let dir = std::env::temp_dir().join(format!("taurscribe_playback_levels_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("levels.wav");
        let spec = hound::WavSpec { channels: 2, sample_rate: 48_000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        let mut writer = hound::WavWriter::create(&wav, spec).unwrap();
        for i in 0..48_000 * 10 {
            let tone = (i as f32 * 220.0 * std::f32::consts::TAU / 48_000.0).sin();
            writer.write_sample(if i < 48_000 * 5 { 0.3 * tone } else { 0.0 }).unwrap();
            writer.write_sample(if i >= 48_000 * 5 { 0.01 * tone } else { 0.0 }).unwrap();
        }
        writer.finalize().unwrap();
        let webm = compress_for_playback(&wav).unwrap();
        if let Ok(decoded) = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&webm)
            .args(["-f", "f32le", "-ac", "1", "-ar", "48000", "pipe:1"])
            .output()
        {
            assert!(decoded.status.success(), "ffmpeg: {}", String::from_utf8_lossy(&decoded.stderr));
            let samples: Vec<f32> = decoded.stdout.chunks_exact(4)
                .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
                .collect();
            let rms = |region: &[f32]| (region.iter().map(|v| v * v).sum::<f32>() / region.len() as f32).sqrt();
            let mic = rms(&samples[48_000..48_000 * 4]);
            let caller = rms(&samples[48_000 * 6..48_000 * 9]);
            assert!((caller / mic) > 0.7 && (caller / mic) < 1.4, "caller {caller} vs mic {mic}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_compress_writes_small_webm() {
        let _serial = COMPRESS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("taurscribe_meeting_audio_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("meeting_test.wav");
        let spec = hound::WavSpec { channels: 2, sample_rate: 48_000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        let mut w = hound::WavWriter::create(&wav, spec).unwrap();
        for i in 0..48_000 * 10 {
            let t = i as f32 / 48_000.0;
            w.write_sample(0.2 * (t * 220.0 * 6.283).sin()).unwrap();
            w.write_sample(0.02 * (t * 330.0 * 6.283).sin()).unwrap();
        }
        w.finalize().unwrap();

        let out = compress_for_playback(&wav).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(&bytes[..4], &[0x1A, 0x45, 0xDF, 0xA3]);
        assert!(bytes.windows(6).any(|w| w == b"A_OPUS"));
        if let Ok(probe) = std::process::Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "stream=codec_name", "-show_entries", "format=duration", "-of", "json"])
            .arg(&out)
            .output()
        {
            assert!(probe.status.success(), "ffprobe: {}", String::from_utf8_lossy(&probe.stderr));
            let metadata: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
            assert_eq!(metadata["streams"][0]["codec_name"], "opus");
            let duration: f64 = metadata["format"]["duration"].as_str().unwrap().parse().unwrap();
            assert!((duration - 10.0).abs() < 0.05, "duration {duration}");
        }
        let raw = std::fs::metadata(&wav).unwrap().len();
        assert!(bytes.len() < 40_000 && (raw as usize) > bytes.len() * 50, "{} bytes from {}", bytes.len(), raw);
        let d = last_processing_diagnostics().unwrap();
        assert_eq!(d.raw_channels, 2);
        assert!((d.raw_seconds - 10.0).abs() < 0.01);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncated_wav_never_replaces_the_source() {
        let dir = std::env::temp_dir().join(format!("taurscribe_truncated_wav_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("truncated.wav");
        let spec = hound::WavSpec { channels: 2, sample_rate: 48_000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        let mut writer = hound::WavWriter::create(&wav, spec).unwrap();
        for _ in 0..960 {
            writer.write_sample(0.2f32).unwrap();
            writer.write_sample(0.1f32).unwrap();
        }
        writer.finalize().unwrap();
        let size = std::fs::metadata(&wav).unwrap().len();
        std::fs::OpenOptions::new().write(true).open(&wav).unwrap().set_len(size - 16).unwrap();
        assert!(compress_for_playback(&wav).is_err());
        assert!(wav.exists());
        assert!(!wav.with_extension("webm").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
