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
pub fn decode_audio_mono_stream<F>(path: &Path, mut on_samples: F) -> Result<(u64, u32), String>
where
    F: FnMut(&[f32], u32, u64, Option<u64>) -> Result<(), String>,
{
    let (frames, rate, _) =
        decode_audio_packets(path, |samples, channels, rate, decoded, expected| {
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
    let (_, sample_rate, channels) = decode_audio_packets(path, |samples, channels, _, _, _| {
        push_samples(&mut all_samples, samples, channels);
        Ok(())
    })?;
    Ok((all_samples, sample_rate, channels))
}

fn decode_audio_packets<F>(path: &Path, mut on_packet: F) -> Result<(u64, u32, u32), String>
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
                total_frames += (buf.samples().len() / channels) as u64;
                on_packet(
                    buf.samples(),
                    channels,
                    rate,
                    total_frames,
                    declared_frames,
                )?;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

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
