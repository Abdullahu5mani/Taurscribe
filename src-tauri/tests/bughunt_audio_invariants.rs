//! Audio boundary and state invariants. No microphone, model downloads or app DB.
use std::path::{Path, PathBuf};
use taurscribe_lib::audio_decode::{decode_audio_interleaved_f32, decode_audio_mono_f32, decode_audio_mono_stream};
use taurscribe_lib::audio_preprocess::{resample_mono_to_16k, StreamingResampler};
use taurscribe_lib::neural_diarizer::{tidy_turns, SpeakerTurn};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self { Self(std::env::temp_dir().join(format!("taurscribe-adversarial-{}.wav", rand::random::<u64>()))) }
    fn path(&self) -> &Path { &self.0 }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); } }

fn write_pcm(path: &Path, channels: u16, bits: u16, floating: bool, frames: usize) {
    let mut writer = hound::WavWriter::create(path, hound::WavSpec {
        channels, sample_rate: 22_050, bits_per_sample: bits,
        sample_format: if floating { hound::SampleFormat::Float } else { hound::SampleFormat::Int },
    }).unwrap();
    for n in 0..frames {
        for c in 0..channels {
            let value = ((n * 13 + c as usize * 17) % 127) as i32 - 63;
            if floating { writer.write_sample(value as f32 / 128.0).unwrap(); }
            else { writer.write_sample(value << (bits - 8)).unwrap(); }
        }
    }
    writer.finalize().unwrap();
}

#[test]
fn wav_bit_depth_and_channel_matrix_agrees_across_streamed_and_full_decoders() {
    for channels in [1, 2, 6] {
        for (bits, float) in [(8, false), (16, false), (24, false), (32, false), (32, true)] {
            let fixture = Fixture::new(); write_pcm(fixture.path(), channels, bits, float, 4099);
            let (interleaved, rate, decoded_channels) = decode_audio_interleaved_f32(fixture.path()).unwrap();
            let (mono, mono_rate) = decode_audio_mono_f32(fixture.path()).unwrap();
            let mut streamed = Vec::new();
            let mut previous = 0;
            let (frames, stream_rate) = decode_audio_mono_stream(fixture.path(), |pcm, rate, decoded, _| {
                assert_eq!(rate, 22_050);
                assert!(decoded > previous); previous = decoded;
                streamed.extend_from_slice(pcm); Ok(())
            }).unwrap();
            assert_eq!((rate, mono_rate, stream_rate, decoded_channels), (22_050, 22_050, 22_050, channels as u32));
            assert_eq!(frames, 4099); assert_eq!(mono.len(), 4099); assert_eq!(streamed, mono);
            assert_eq!(interleaved.len(), 4099 * channels as usize);
            for (n, sample) in mono.iter().enumerate() {
                let expected = (0..channels).map(|c| (((n * 13 + c as usize * 17) % 127) as f32 - 63.0) / 128.0).sum::<f32>() / channels as f32;
                assert!((sample - expected).abs() < 1e-6, "bits={bits}, float={float}, channels={channels}, frame={n}: {sample} != {expected}");
            }
        }
    }
}

#[test]
fn stream_callback_cancellation_aborts_immediately_and_preserves_source() {
    let fixture = Fixture::new(); write_pcm(fixture.path(), 2, 16, false, 20_000);
    let original = std::fs::read(fixture.path()).unwrap();
    let mut calls = 0;
    let result = decode_audio_mono_stream(fixture.path(), |_, _, _, _| {
        calls += 1; Err("user cancelled".into())
    });
    assert_eq!(calls, 1);
    assert_eq!(result.unwrap_err(), "user cancelled");
    assert_eq!(std::fs::read(fixture.path()).unwrap(), original);
}

#[test]
fn resampling_short_tails_is_independent_of_input_packet_boundaries() {
    for rate in [8_000, 11_025, 16_000, 22_050, 44_100, 48_000, 96_000] {
        for len in [1, 17, 159, 160, 1000, 1023, 4095, 4096, 4097, 11026] {
            let source: Vec<_> = (0..len).map(|i| 0.3 * (i as f32 * 0.11).sin()).collect();
            let expected = resample_mono_to_16k(&source, rate).unwrap();
            for packet in [1, 137, 1024, 4096] {
                let mut resampler = StreamingResampler::new(rate).unwrap();
                let mut actual = Vec::new();
                for input in source.chunks(packet) { actual.extend(resampler.push(input).unwrap()); }
                actual.extend(resampler.finish().unwrap());
                assert_eq!(actual.len(), expected.len(), "rate={rate}, len={len}, packet={packet}");
                assert!(actual.iter().zip(&expected).all(|(a, b)| a.is_finite() && (a - b).abs() < 1e-6), "rate={rate}, len={len}, packet={packet}");
                assert!(resampler.finish().unwrap().is_empty(), "finishing twice cannot add more samples");
            }
        }
    }
}

#[test]
fn invalid_stream_resampler_configurations_fail_without_panicking() {
    for config in [(0, 16_000, 1024), (48_000, 0, 1024), (48_000, 16_000, 0)] {
        assert!(StreamingResampler::with_target(config.0, config.1, config.2).is_err());
    }
}

#[test]
fn diarizer_cleanup_is_idempotent_ordered_and_uses_contiguous_speaker_ids() {
    let mut seed = 7u64;
    let mut next = || { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1); seed };
    for _ in 0..100 {
        let raw: Vec<_> = (0..60).map(|i| {
            let start = i * 300 + next() % 200;
            SpeakerTurn { start_ms: start, end_ms: start + next() % 2000 + 1, speaker: (next() % 8) as usize }
        }).collect();
        let maximum = raw.iter().map(|t| t.end_ms).max().unwrap();
        let clean = tidy_turns(raw);
        assert_eq!(tidy_turns(clean.clone()), clean);
        assert!(clean.iter().all(|t| t.end_ms > t.start_ms && t.end_ms <= maximum));
        assert!(clean.windows(2).all(|pair| pair[0].end_ms <= pair[1].start_ms));
        let mut ids: Vec<_> = clean.iter().map(|t| t.speaker).collect(); ids.sort_unstable(); ids.dedup();
        assert_eq!(ids, (0..ids.len()).collect::<Vec<_>>());
    }
}

#[test]
fn malformed_wav_headers_return_errors_and_keep_the_file() {
    for content in [&b""[..], &b"RIFF\0\0\0\0WAVE"[..], &b"definitely not audio"[..]] {
        let fixture = Fixture::new(); std::fs::write(fixture.path(), content).unwrap();
        assert!(decode_audio_mono_f32(fixture.path()).is_err());
        assert_eq!(std::fs::read(fixture.path()).unwrap(), content);
    }
}

#[test]
fn nonfinite_float_audio_must_not_enter_the_asr_pipeline() {
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let fixture = Fixture::new();
        let mut writer = hound::WavWriter::create(fixture.path(), hound::WavSpec {
            channels: 1, sample_rate: 16_000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float,
        }).unwrap();
        for n in 0..16_000 { writer.write_sample(if n == 8000 { invalid } else { 0.3 * (n as f32 * 0.11).sin() }).unwrap(); }
        writer.finalize().unwrap();
        let original = std::fs::read(fixture.path()).unwrap();
        assert!(decode_audio_mono_f32(fixture.path()).unwrap_err().contains("nonfinite"));
        assert!(decode_audio_interleaved_f32(fixture.path()).unwrap_err().contains("nonfinite"));
        let error = decode_audio_mono_stream(fixture.path(), |pcm, _, _, _| {
            assert!(pcm.iter().all(|s| s.is_finite()), "invalid audio cannot reach inference"); Ok(())
        }).unwrap_err();
        assert!(error.contains("nonfinite"));
        assert_eq!(std::fs::read(fixture.path()).unwrap(), original);
    }
}
