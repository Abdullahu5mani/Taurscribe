//! Explicit playback acceptance check. Requires ffmpeg and ffprobe on PATH.
//! Run: cargo test --test meeting_playback_stress -- --ignored --nocapture
use std::{path::Path, process::Command};
use taurscribe_lib::meeting_audio::compress_for_playback;

fn decode(path: &Path) -> Vec<f32> {
    let decoded = Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(path)
        .args(["-f", "f32le", "-ac", "1", "-ar", "48000", "pipe:1"])
        .output().expect("ffmpeg is required for this explicit playback test");
    assert!(decoded.status.success(), "{}", String::from_utf8_lossy(&decoded.stderr));
    assert_eq!(decoded.stdout.len() % 4, 0);
    decoded.stdout.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect()
}

fn rms(samples: &[f32]) -> f64 {
    (samples.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / samples.len() as f64).sqrt()
}

fn tone_fraction(samples: &[f32], hz: f64) -> f64 {
    let (mut sin, mut cos) = (0.0, 0.0);
    for (i, &sample) in samples.iter().enumerate() {
        let phase = i as f64 * hz * std::f64::consts::TAU / 48000.0;
        sin += sample as f64 * phase.sin(); cos += sample as f64 * phase.cos();
    }
    2.0 * (sin * sin + cos * cos) / (samples.len() as f64).powi(2) / rms(samples).powi(2)
}

#[test]
#[ignore = "Requires ffmpeg/ffprobe; encodes three 65-second multi-cluster fixtures"]
fn long_playback_preserves_timing_quiet_remote_audio_and_final_partial_frame() {
    let dir = std::env::temp_dir().join(format!("taurscribe_playback_stress_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for rate in [16000, 44100, 48000] {
        let wav = dir.join(format!("capture-{rate}.wav"));
        let spec = hound::WavSpec { channels: 2, sample_rate: rate, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        let mut w = hound::WavWriter::create(&wav, spec).unwrap();
        let frames = (rate as f64 * 65.013).round() as usize;
        for frame in 0..frames {
            let t = frame as f64 / rate as f64;
            let mic = if t < 10.0 || t >= 60.0 { 0.2 * (t * 220.0 * std::f64::consts::TAU).sin() } else { 0.0 };
            let remote = if (10.0..30.0).contains(&t) { 0.02 * (t * 660.0 * std::f64::consts::TAU).sin() } else { 0.0 };
            w.write_sample(mic as f32).unwrap(); w.write_sample(remote as f32).unwrap();
        }
        w.finalize().unwrap();
        let original = std::fs::read(&wav).unwrap();
        let playback = compress_for_playback(&wav).unwrap();
        assert_eq!(std::fs::read(&wav).unwrap(), original, "compression changed raw PCM");
        let decoded = decode(&playback);
        assert!(decoded.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "non-finite or clipped playback");
        let expected = frames as f64 / rate as f64;
        let duration = decoded.len() as f64 / 48000.0;
        assert!((duration - expected).abs() <= 0.021, "rate={rate}, decoded={duration}, expected={expected}");
        let head = &decoded[48000..4 * 48000];
        let caller = &decoded[20 * 48000..23 * 48000];
        let tail = &decoded[62 * 48000..64 * 48000];
        assert!(rms(head) > 0.03 && rms(tail) > 0.03, "missing head/tail");
        let ratio = rms(caller) / rms(head);
        assert!((0.7..1.4).contains(&ratio), "quiet remote level ratio={ratio}");
        assert!(tone_fraction(head, 220.0) > 0.85, "mic waveform distorted");
        assert!(tone_fraction(caller, 660.0) > 0.85, "remote waveform distorted");
        assert!(rms(&decoded[40 * 48000..50 * 48000]) < 0.002, "silence acquired noise");
        let probe = Command::new("ffprobe").args(["-v", "error", "-show_entries", "format=duration:stream=codec_name", "-of", "json"])
            .arg(&playback).output().expect("ffprobe is required for this explicit playback test");
        assert!(probe.status.success(), "{}", String::from_utf8_lossy(&probe.stderr));
        let metadata: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
        assert_eq!(metadata["streams"][0]["codec_name"], "opus");
        let container_duration: f64 = metadata["format"]["duration"].as_str().unwrap().parse().unwrap();
        assert!((container_duration - expected).abs() < 0.03);
        let size = std::fs::metadata(&playback).unwrap().len();
        assert!(size < 180_000, "unexpected playback size {size}");
        assert!(!wav.with_extension("webm.tmp").exists());
        eprintln!("rate={rate}: {duration:.3}s decoded, caller/mic={ratio:.3}, {size} bytes");
    }
    std::fs::remove_dir_all(dir).unwrap();
}
