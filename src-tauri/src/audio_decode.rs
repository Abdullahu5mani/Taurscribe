//! Decode arbitrary audio files to interleaved f32 via Symphonia (shared by file transcription and eval).

use std::path::Path;

/// Decode an audio file to interleaved f32 samples.
/// Returns `(samples, sample_rate_hz, channel_count)`.
pub fn decode_audio_interleaved_f32(path: &Path) -> Result<(Vec<f32>, u32, u32), String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::errors::Error as SymphError;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path).map_err(|e| format!("Cannot open file: {}", e))?;

    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
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

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Cannot create audio decoder: {}", e))?;

    let mut all_samples: Vec<f32> = Vec::new();
    let mut actual_channels: u32 = hint_channels;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(_) => break,
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                if actual_channels == 0 {
                    actual_channels = spec.channels.count() as u32;
                }
                let capacity = decoded.capacity() as u64;
                if capacity == 0 {
                    continue;
                }
                let mut buf = SampleBuffer::<f32>::new(capacity, spec);
                buf.copy_interleaved_ref(decoded);
                all_samples.extend_from_slice(buf.samples());
            }
            Err(SymphError::IoError(_)) => continue,
            Err(SymphError::DecodeError(_)) => continue,
            Err(_) => break,
        }
    }

    if all_samples.is_empty() {
        return Err("Audio file is empty or could not be decoded".to_string());
    }

    if actual_channels == 0 {
        actual_channels = 1;
    }

    Ok((all_samples, sample_rate, actual_channels))
}
