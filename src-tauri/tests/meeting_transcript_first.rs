//! Installed-model regression for timestamp extraction and bounded import chunks.
//! This public single-speaker fixture is not a multi-speaker accuracy benchmark.
use taurscribe_lib::{
    gguf_asr::GgufAsrManager,
    meeting_transcript::{chunk_ranges, TimedTranscript},
    whisper::WhisperManager,
};
#[path = "support/asr_reference.rs"]
mod asr_reference;
#[test]
#[ignore = "Requires installed Whisper and Granite models"]
fn installed_models_preserve_public_reference_with_import_chunks() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../taurscribe-runtime/samples/jfk.wav");
    let (audio, rate, channels) =
        taurscribe_lib::audio_decode::decode_audio_interleaved_strict(&fixture).unwrap();
    let mono: Vec<_> = audio
        .chunks_exact(channels as usize)
        .map(|f| f.iter().sum::<f32>() / channels as f32)
        .collect();
    let pcm = taurscribe_lib::audio_preprocess::resample_mono_to_16k(&mono, rate).unwrap();
    let repeated: Vec<_> = (0..4)
        .flat_map(|_| pcm.iter().copied().chain(std::iter::repeat_n(0.0, 16000)))
        .collect();
    let reference = std::iter::repeat_n(asr_reference::JFK_REFERENCE, 4)
        .collect::<Vec<_>>()
        .join(" ");
    let mut whisper = WhisperManager::new();
    whisper.initialize(Some("base.en-q5_1"), false).unwrap();
    let mut granite = GgufAsrManager::granite();
    granite
        .initialize(Some("granite-speech-5-nc"), false)
        .unwrap();
    for name in ["Whisper", "Granite"] {
        let baseline = if name == "Whisper" {
            whisper.transcribe_audio_data(&repeated, None).unwrap()
        } else {
            granite.transcribe_chunk(&repeated, 16000, None).unwrap()
        };
        let mut result = TimedTranscript::default();
        let begin = std::time::Instant::now();
        for (index, (start, end)) in chunk_ranges(&repeated).into_iter().enumerate() {
            let part = if name == "Whisper" {
                whisper
                    .transcribe_audio_data_timed(&repeated[start..end], None)
                    .unwrap()
            } else {
                granite.transcribe_timed(&repeated[start..end]).unwrap()
            };
            if index == 0 {
                println!("{name}: first text {:.2}s", begin.elapsed().as_secs_f64());
            }
            result.append(part, start as u64 / 16);
        }
        let before = asr_reference::word_error_rate(&reference, &baseline);
        let after = asr_reference::word_error_rate(&reference, &result.text);
        println!("{name}: baseline WER {before:.4}, draft WER {after:.4}, timestamp coverage {}, total {:.2}s",result.covers_text(),begin.elapsed().as_secs_f64());
        assert!(
            after <= before + 0.03,
            "{name} chunking regression: {after} vs {before}: {}",
            result.text
        );
        assert!(after < 0.15, "{name} failed reference coverage");
    }
}

#[test]
#[ignore = "Requires public-room fixture, Granite and Nemotron models"]
fn two_public_speakers_preserve_words_and_attribution() {
    let path = std::env::var("MEETING_ROOM_FIXTURE")
        .unwrap_or_else(|_| "/tmp/taurscribe-transcript-first-qa/public-room.wav".into());
    let truth_path = std::path::Path::new(&path).with_extension("json");
    #[derive(serde::Deserialize)]
    struct Truth {
        speaker: String,
        start_ms: u64,
        end_ms: u64,
        text: String,
    }
    let truth: Vec<Truth> = serde_json::from_slice(&std::fs::read(truth_path).unwrap()).unwrap();
    let reference = truth
        .iter()
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let (samples, rate) =
        taurscribe_lib::diarization::read_room_audio(std::path::Path::new(&path)).unwrap();
    let pcm = taurscribe_lib::audio_preprocess::resample_mono_to_16k(&samples, rate).unwrap();
    let mut model = GgufAsrManager::granite();
    model
        .initialize(Some("granite-speech-5-nc"), false)
        .unwrap();
    let mut cached = TimedTranscript::default();
    for (start, end) in chunk_ranges(&pcm) {
        cached.append(
            model.transcribe_timed(&pcm[start..end]).unwrap(),
            start as u64 / 16,
        );
    }
    let neural = taurscribe_lib::neural_diarizer::diarize_16k(&pcm).unwrap();
    let snippets =
        std::env::temp_dir().join(format!("meeting-import-quality-{}", std::process::id()));
    let mut calls = 0;
    let mut transcribe = |audio: &[f32], rate| {
        calls += 1;
        model.transcribe_chunk(audio, rate, None)
    };
    let baseline = taurscribe_lib::diarization::finish_room_import(
        &samples,
        rate,
        &snippets,
        1,
        &mut transcribe,
        Some(2),
        &TimedTranscript::default(),
        Some(Ok(neural.clone())),
    )
    .unwrap();
    let old_calls = calls;
    calls = 0;
    let mut transcribe = |audio: &[f32], rate| {
        calls += 1;
        model.transcribe_chunk(audio, rate, None)
    };
    let result = taurscribe_lib::diarization::finish_room_import(
        &samples,
        rate,
        &snippets,
        2,
        &mut transcribe,
        Some(2),
        &cached,
        Some(Ok(neural)),
    )
    .unwrap();
    let joined = |turns: &[taurscribe_lib::diarization::DiarizedTurn]| {
        turns
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let before = asr_reference::word_error_rate(&reference, &joined(&baseline));
    let after = asr_reference::word_error_rate(&reference, &joined(&result));
    println!("Two-speaker fixture: baseline WER {before:.4}, cached WER {after:.4}, baseline ASR calls {old_calls}, fallback calls {calls}");
    assert!(
        after <= before + 0.01,
        "speaker cache degraded transcription: {}",
        joined(&result)
    );
    let labels = |turns: &[taurscribe_lib::diarization::DiarizedTurn]| {
        truth
            .iter()
            .map(|t| {
                let label = turns
                    .iter()
                    .max_by_key(|turn| {
                        turn.end_ms
                            .min(t.end_ms)
                            .saturating_sub(turn.start_ms.max(t.start_ms))
                    })
                    .unwrap();
                (t.speaker.clone(), label.speaker_id.clone())
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(labels(&baseline), labels(&result));
    assert!(calls <= old_calls, "cache increased per-speaker ASR calls");
    assert!(
        !cached.covers_text(),
        "Granite Speech 5 must not claim real timestamps"
    );
    let _ = std::fs::remove_dir_all(snippets);
}

#[test]
#[ignore = "Requires Windows GPU with enough free VRAM and public-room fixture"]
fn gpu_overlap_preserves_transcription_and_diarization() {
    let path = std::env::var("MEETING_ROOM_FIXTURE").expect("MEETING_ROOM_FIXTURE");
    let (samples, rate) =
        taurscribe_lib::diarization::read_room_audio(std::path::Path::new(&path)).unwrap();
    let pcm = taurscribe_lib::audio_preprocess::resample_mono_to_16k(&samples, rate).unwrap();
    let mut model = GgufAsrManager::granite();
    model
        .initialize(Some("granite-speech-5-nc"), false)
        .unwrap();
    assert!(
        taurscribe_lib::meeting_transcript::can_overlap_diarization(),
        "GPU headroom insufficient or unreported; parallel path cannot be validated"
    );
    let start = std::time::Instant::now();
    let mut sequential = TimedTranscript::default();
    for (a, b) in chunk_ranges(&pcm) {
        sequential.append(model.transcribe_timed(&pcm[a..b]).unwrap(), a as u64 / 16);
    }
    let serial_turns = taurscribe_lib::neural_diarizer::diarize_16k(&pcm).unwrap();
    let serial_time = start.elapsed();
    let start = std::time::Instant::now();
    let (parallel, parallel_turns) = std::thread::scope(|scope| {
        let mut result = TimedTranscript::default();
        let mut worker = None;
        for (index, (a, b)) in chunk_ranges(&pcm).into_iter().enumerate() {
            result.append(model.transcribe_timed(&pcm[a..b]).unwrap(), a as u64 / 16);
            if index == 0 {
                let audio = &pcm;
                worker =
                    Some(scope.spawn(move || taurscribe_lib::neural_diarizer::diarize_16k(audio)));
            }
        }
        (result, worker.unwrap().join().unwrap().unwrap())
    });
    println!(
        "GPU sequential {:.2}s, overlapped {:.2}s",
        serial_time.as_secs_f64(),
        start.elapsed().as_secs_f64()
    );
    assert_eq!(
        parallel.text, sequential.text,
        "concurrent inference changed ASR output"
    );
    assert_eq!(
        parallel_turns, serial_turns,
        "concurrent inference changed speaker intervals"
    );
}

#[test]
#[ignore = "Requires installed Whisper and Granite models"]
fn native_cancellation_aborts_and_the_same_models_can_retry() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../taurscribe-runtime/samples/jfk.wav");
    let (audio, rate, channels) =
        taurscribe_lib::audio_decode::decode_audio_interleaved_strict(&fixture).unwrap();
    let mono: Vec<_> = audio
        .chunks_exact(channels as usize)
        .map(|f| f.iter().sum::<f32>() / channels as f32)
        .collect();
    let pcm = taurscribe_lib::audio_preprocess::resample_mono_to_16k(&mono, rate).unwrap();
    let long: Vec<_> = (0..40).flat_map(|_| pcm.iter().copied()).collect();
    let mut whisper = WhisperManager::new();
    whisper.initialize(Some("base.en-q5_1"), true).unwrap();
    let mut granite = GgufAsrManager::granite();
    granite
        .initialize(Some("granite-speech-5-nc"), false)
        .unwrap();
    for name in ["Whisper", "Granite"] {
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let setter = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            signal.store(true, Ordering::Relaxed);
        });
        let begin = std::time::Instant::now();
        let result = if name == "Whisper" {
            whisper.transcribe_audio_data_timed_cancellable(&long, None, Some(cancel.clone()))
        } else {
            granite.transcribe_timed_cancellable(&long, &cancel)
        };
        setter.join().unwrap();
        assert!(result.is_err(), "{name} ignored cancellation");
        assert!(
            begin.elapsed().as_secs_f64() < 10.0,
            "{name} took too long to stop"
        );
        println!("{name} cancelled in {:.3}s", begin.elapsed().as_secs_f64());
        let fresh = Arc::new(AtomicBool::new(false));
        let result = if name == "Whisper" {
            whisper.transcribe_audio_data_timed_cancellable(&pcm, None, Some(fresh))
        } else {
            granite.transcribe_timed_cancellable(&pcm, &fresh)
        }
        .unwrap();
        assert!(
            !result.text.trim().is_empty(),
            "{name} cannot retry after cancellation"
        );
    }
}
