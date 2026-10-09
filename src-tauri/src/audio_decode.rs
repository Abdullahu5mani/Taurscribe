//! Decode arbitrary audio files to interleaved f32 via Symphonia (shared by file transcription and eval).

use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decode an audio file to interleaved f32 samples.
/// Returns `(samples, sample_rate_hz, channel_count)`.
pub fn decode_audio_interleaved_f32(path: &Path) -> Result<(Vec<f32>, u32, u32), String> {
    decode_audio_with(path, |acc, samples, channels| {
        let _ = channels;
        acc.extend_from_slice(samples);
    })
}

/// Decode an audio file directly to mono f32 samples.
///
/// This avoids materializing the full interleaved buffer before immediately
/// downmixing it, which is the common path for ASR.
pub fn decode_audio_mono_f32(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let (samples, sample_rate, _channels) = decode_audio_with(path, |acc, samples, channels| {
        if channels <= 1 {
            acc.extend_from_slice(samples);
            return;
        }

        for frame in samples.chunks(channels) {
            if frame.is_empty() {
                continue;
            }
            acc.push(frame.iter().copied().sum::<f32>() / frame.len() as f32);
        }
    })?;

    Ok((samples, sample_rate))
}

/// Decode one packet at a time without retaining the whole file in memory.
/// The callback receives mono samples at the source sample rate. Returns the
/// decoded frame count and sample rate for duration reporting. The callback
/// also receives decoded frames so far and the optional duration from metadata.
pub fn decode_audio_mono_stream<F>(path: &Path, on_samples: F) -> Result<(u64, u32), String>
where
    F: FnMut(&[f32], u32, u64, Option<u64>) -> Result<(), String>,
{
    decode_audio_mono_stream_with(path, on_samples, false)
}

/// Import must never silently save a partial recording as a complete meeting.
pub fn decode_audio_mono_stream_strict<F>(path: &Path, on_samples: F) -> Result<(u64, u32), String>
where
    F: FnMut(&[f32], u32, u64, Option<u64>) -> Result<(), String>,
{
    decode_audio_mono_stream_with(path, on_samples, true)
}

fn decode_audio_mono_stream_with<F>(path: &Path, mut on_samples: F, strict: bool) -> Result<(u64, u32), String>
where
    F: FnMut(&[f32], u32, u64, Option<u64>) -> Result<(), String>,
{
    let (frames, rate, _) =
        decode_audio_packets(path, strict, |samples, channels, rate, decoded, expected| {
            if channels <= 1 {
                on_samples(samples, rate, decoded, expected)
            } else {
                let mono: Vec<f32> = samples
                    .chunks(channels)
                    .map(|frame| frame.iter().copied().sum::<f32>() / frame.len() as f32)
                    .collect();
                on_samples(&mono, rate, decoded, expected)
            }
        })?;
    Ok((frames, rate))
}

fn decode_audio_with<F>(path: &Path, mut push_samples: F) -> Result<(Vec<f32>, u32, u32), String>
where
    F: FnMut(&mut Vec<f32>, &[f32], usize),
{
    let mut all_samples = Vec::new();
    let (_, sample_rate, channels) = decode_audio_packets(path, false, |samples, channels, _, _, _| {
        push_samples(&mut all_samples, samples, channels);
        Ok(())
    })?;
    Ok((all_samples, sample_rate, channels))
}

fn decode_audio_packets<F>(path: &Path, strict: bool, mut on_packet: F) -> Result<(u64, u32, u32), String>
where
    F: FnMut(&[f32], usize, u32, u64, Option<u64>) -> Result<(), String>,
{
    let file = std::fs::File::open(path).map_err(|e| format!("Cannot open file: {}", e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("Cannot probe audio format: {}", e))?;

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or("No audio track found in file")?;

    if track.codec_params.codec == symphonia::core::codecs::CODEC_TYPE_OPUS {
        // Symphonia demuxes Ogg Opus but has no Opus decoder. Reuse the
        // libopus already bundled for meeting playback, with gapless trimming.
        drop(format);
        return decode_opus_packets(path, on_packet);
    }

    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or("File has unknown sample rate")?;
    let hint_channels = track
        .codec_params
        .channels
        .map(|c| c.count() as u32)
        .unwrap_or(0);
    let declared_frames = track.codec_params.n_frames;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Cannot create audio decoder: {}", e))?;

    let mut actual_channels: u32 = hint_channels;
    let mut total_frames = 0u64;
    // The decoder's rate, which wins over the container's (they can differ, e.g.
    // for HE-AAC). It must not change partway through.
    let mut decoded_rate: Option<u32> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            // Reading the file itself failed: not the end of the audio.
            Err(SymphError::IoError(e)) => {
                return Err(format!(
                    "Audio read failed before the end of {}: {e}",
                    path.display()
                ))
            }
            Err(e) if strict => return Err(format!("Audio read failed: {e}")),
            // A reset or a malformed tail ends the stream with what decoded so far.
            Err(_) => break,
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                let rate = *decoded_rate.get_or_insert(spec.rate);
                if spec.rate != rate {
                    return Err(format!(
                        "Audio sample rate changed while decoding {}",
                        path.display()
                    ));
                }
                let channels = spec.channels.count().max(1);
                actual_channels = channels as u32;
                let capacity = decoded.capacity() as u64;
                if capacity == 0 {
                    continue;
                }
                let mut buf = SampleBuffer::<f32>::new(capacity, spec);
                buf.copy_interleaved_ref(decoded);
                if buf.samples().iter().any(|s| !s.is_finite()) {
                    return Err("Audio contains nonfinite samples".into());
                }
                total_frames += (buf.samples().len() / channels) as u64;
                on_packet(
                    buf.samples(),
                    channels,
                    rate,
                    total_frames,
                    declared_frames,
                )?;
            }
            Err(e) if strict => return Err(format!("Audio packet could not be decoded: {e}")),
            // A damaged packet leaves a short gap instead of failing the whole file.
            Err(SymphError::DecodeError(_) | SymphError::IoError(_)) => continue,
            Err(_) => break,
        }
    }

    if total_frames == 0 {
        return Err("Audio file is empty or could not be decoded".to_string());
    }
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"))
        && declared_frames.is_some_and(|expected| total_frames < expected)
    {
        if strict { return Err(format!("WAV ended early: decoded {total_frames} of {} frames", declared_frames.unwrap_or(0))); }
        // A crashed recorder, or a WAV streamed through a pipe, declares more
        // data than it holds. Keep what is there: a streaming caller has already
        // transcribed it by now.
        eprintln!(
            "[AUDIO] {} ended before its declared length ({} of {} frames); using what was decoded",
            path.display(),
            total_frames,
            declared_frames.unwrap_or(0)
        );
    }

    if actual_channels == 0 {
        actual_channels = 1;
    }

    Ok((total_frames, decoded_rate.unwrap_or(sample_rate), actual_channels))
}


/// Decode Ogg Opus through the existing demuxer and bundled multistream decoder.
/// Header pre-skip and the final granule count remove encoder delay/padding.
/// Symphonia's first buffered page can lack packet trims, especially in short
/// files, so apply those bounds to decoded positions rather than packet flags.
fn decode_opus_packets<F>(path: &Path, mut on_packet: F) -> Result<(u64, u32, u32), String>
where
    F: FnMut(&[f32], usize, u32, u64, Option<u64>) -> Result<(), String>,
{
    let file = std::fs::File::open(path).map_err(|e| format!("Cannot open file: {e}"))?;
    let source = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("ogg");
    let options = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };
    let mut format = symphonia::default::get_probe()
        .format(&hint, source, &options, &MetadataOptions::default())
        .map_err(|e| format!("Cannot probe Opus audio: {e}"))?
        .format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec == symphonia::core::codecs::CODEC_TYPE_OPUS)
        .ok_or("No Opus track found")?;
    let track_id = track.id;
    let granule_frames = track.codec_params.n_frames;
    let header = track
        .codec_params
        .extra_data
        .as_deref()
        .ok_or("Missing Opus header")?;
    if header.len() < 19 || &header[..8] != b"OpusHead" || header[8] > 15 {
        return Err("Invalid Opus identification header".into());
    }
    let pre_skip = u16::from_le_bytes([header[10], header[11]]) as u64;
    let expected_frames = granule_frames
        .and_then(|frames| frames.checked_sub(pre_skip))
        .ok_or("Cannot determine Opus audio length after pre-skip")?;
    let channels = header[9] as usize;
    let (streams, coupled, mapping) = match header[18] {
        0 if channels == 1 => (1, 0, vec![0]),
        0 if channels == 2 => (1, 1, vec![0, 1]),
        1 if (1..=8).contains(&channels) && header.len() >= 21 + channels => {
            // Opus family 1 uses Vorbis speaker order. Present the same
            // channel-mask order as other Symphonia decoders.
            let order: &[usize] = match channels {
                1 => &[0],
                2 => &[0, 1],
                3 => &[0, 2, 1],
                4 => &[0, 1, 2, 3],
                5 => &[0, 2, 1, 3, 4],
                6 => &[0, 2, 1, 5, 3, 4],
                7 => &[0, 2, 1, 6, 5, 3, 4],
                8 => &[0, 2, 1, 7, 5, 6, 3, 4],
                _ => unreachable!(),
            };
            (
                header[19],
                header[20],
                order.iter().map(|&i| header[21 + i]).collect(),
            )
        }
        _ => {
            return Err(format!(
                "Unsupported Opus channel mapping for {channels} channels"
            ))
        }
    };
    let gain = i16::from_le_bytes([header[16], header[17]]) as i32;
    let mut decoder = opus::MSDecoder::new(48_000, streams, coupled, &mapping)
        .map_err(|e| format!("Cannot create Opus decoder: {e}"))?;
    decoder
        .set_gain(gain)
        .map_err(|e| format!("Cannot apply Opus output gain: {e}"))?;
    // An Opus packet can contain at most 120 ms, regardless of encoder rate.
    let mut samples = vec![0.0f32; 5_760 * channels];
    let mut total = 0u64;
    let mut decoded_position = 0u64;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("Opus audio read failed: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        // An empty packet would ask libopus to synthesize packet-loss audio.
        // A file import must not silently manufacture that data.
        if packet.data.is_empty() {
            return Err("Empty Opus audio packet".into());
        }
        let frames = decoder
            .decode_float(&packet.data, &mut samples, false)
            .map_err(|e| format!("Opus audio packet could not be decoded: {e}"))?;
        let start = pre_skip.saturating_sub(decoded_position).min(frames as u64) as usize;
        let end = (pre_skip + expected_frames)
            .saturating_sub(decoded_position)
            .min(frames as u64) as usize;
        decoded_position += frames as u64;
        if end <= start {
            continue;
        }
        total += (end - start) as u64;
        on_packet(
            &samples[start * channels..end * channels],
            channels,
            48_000,
            total,
            Some(expected_frames),
        )?;
    }
    if total == 0 {
        return Err("Opus audio file contains no decoded samples".into());
    }
    if total != expected_frames {
        return Err(format!(
            "Opus audio ended early: decoded {total} of {expected_frames} frames"
        ));
    }
    Ok((total, 48_000, channels as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ogg_opus_preserves_gapless_bounds_and_channel_order() {
        for (name, channels, expected_frames) in [
            ("opus-1ch.ogg", 1usize, 10003),
            ("opus-2ch.ogg", 2, 10003),
            ("opus-6ch.ogg", 6, 10003),
            ("opus-8ch.ogg", 8, 10003),
            ("opus-long.ogg", 2, 100003),
        ] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name);
            let mut interleaved = Vec::new();
            let (frames, rate, decoded_channels) =
                decode_audio_packets(&path, false, |samples, count, sr, _, _| {
                    assert_eq!(count, channels);
                    assert_eq!(sr, 48000);
                    interleaved.extend_from_slice(samples);
                    Ok(())
                })
                .unwrap();
            assert_eq!(
                (frames, rate, decoded_channels),
                (expected_frames as u64, 48000, channels as u32)
            );
            assert_eq!(interleaved.len(), expected_frames * channels);
            assert!(interleaved.iter().all(|v| v.is_finite()));
            // Each channel has its own frequency in standard speaker order.
            // A mapping error changes which frequency dominates that channel.
            for c in 0..channels {
                let energy = |freq: f64| {
                    let (mut re, mut im) = (0.0, 0.0);
                    for n in 1000..9000 {
                        let phase = 2.0 * std::f64::consts::PI * freq * n as f64 / 48000.0;
                        let v = interleaved[n * channels + c] as f64;
                        re += v * phase.cos();
                        im += v * phase.sin();
                    }
                    re * re + im * im
                };
                let wanted = energy((200 + 100 * c) as f64);
                assert!(wanted > 1000.0);
                for other in 0..channels {
                    if other != c {
                        assert!(
                            wanted > 20.0 * energy((200 + 100 * other) as f64),
                            "channel {c} mapped to {other}"
                        );
                    }
                }
            }
            let (batch, _) = decode_audio_mono_f32(&path).unwrap();
            let mut stream = Vec::new();
            decode_audio_mono_stream(&path, |samples, _, _, _| {
                stream.extend_from_slice(samples);
                Ok(())
            })
            .unwrap();
            assert_eq!(batch, stream);
            assert_eq!(batch.len(), expected_frames);
            let cancelled = decode_audio_mono_stream(&path, |_, _, _, _| Err("cancelled".into()));
            assert_eq!(cancelled.unwrap_err(), "cancelled");
        }
    }

    #[test]
    fn ogg_opus_applies_header_gain() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let (base, _) = decode_audio_mono_f32(&fixtures.join("opus-1ch.ogg")).unwrap();
        let (gain, _) = decode_audio_mono_f32(&fixtures.join("opus-gain.ogg")).unwrap();
        assert_eq!(base.len(), gain.len());
        let factor = 10.0_f32.powf(6.0 / 20.0);
        assert!(base
            .iter()
            .zip(&gain)
            .all(|(a, b)| (a * factor - b).abs() < 0.00001));
    }

    #[test]
    fn streamed_mono_matches_full_decode_and_reports_source_duration() {
        let path =
            std::env::temp_dir().join(format!("taurscribe-stream-{}.wav", rand::random::<u64>()));
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..32_000 {
            writer.write_sample::<i16>((i % 100) as i16).unwrap();
            writer.write_sample::<i16>(-((i % 100) as i16)).unwrap();
        }
        writer.finalize().unwrap();
        let (full, rate) = decode_audio_mono_f32(&path).unwrap();
        let mut streamed = Vec::new();
        let (frames, streamed_rate) = decode_audio_mono_stream(&path, |samples, _, _, _| {
            assert!(samples.len() < 32_000);
            streamed.extend_from_slice(samples);
            Ok(())
        })
        .unwrap();
        assert_eq!(rate, streamed_rate);
        assert_eq!(frames as usize, full.len());
        assert_eq!(streamed, full);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn truncated_wav_decodes_the_audio_it_has() {
        let path = std::env::temp_dir().join(format!(
            "taurscribe-truncated-{}.wav",
            rand::random::<u64>()
        ));
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..16_000 {
            writer.write_sample::<i16>(123).unwrap();
        }
        writer.finalize().unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        let original_len = file.metadata().unwrap().len();
        file.set_len(original_len - 8_000).unwrap();
        let (frames, _) = decode_audio_mono_stream(&path, |_, _, _, _| Ok(())).unwrap();
        assert!(frames > 0 && frames < 16_000, "{frames}");
        let _ = std::fs::remove_file(path);
    }
}
