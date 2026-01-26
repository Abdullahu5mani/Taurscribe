use tauri::State;
use crate::state::AudioState;
use crate::types::SampleFile;

/// List default sample files for testing
#[tauri::command]
pub fn list_sample_files() -> Result<Vec<SampleFile>, String> {
    let mut files = Vec::new();

    let possible_paths = [
        "taurscribe-runtime/samples",
        "../taurscribe-runtime/samples",
        "../../taurscribe-runtime/samples",
    ];

    let mut target_dir = std::path::PathBuf::new();
    let mut found = false;

    for path in possible_paths {
        if let Ok(p) = std::fs::canonicalize(path) {
            if p.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&p) {
                    for entry in entries.flatten() {
                        if let Some(name) = entry.file_name().to_str() {
                            if name.to_lowercase().ends_with(".wav") {
                                target_dir = p;
                                found = true;
                                break;
                            }
                        }
                    }
                }
                if found {
                    break;
                }
            }
        }
    }

    if !found {
        return Ok(vec![]);
    }

    let entries = std::fs::read_dir(target_dir)
        .map_err(|e| format!("Failed to read samples directory: {}", e))?;

    for entry in entries {
        if let Ok(entry) = entry {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext.to_string_lossy().to_lowercase() == "wav" {
                        if let Some(name) = path.file_name() {
                            files.push(SampleFile {
                                name: name.to_string_lossy().to_string(),
                                path: path.to_string_lossy().to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(files)
}

/// RUN A PERFORMANCE TEST
#[tauri::command]
pub fn benchmark_test(state: State<AudioState>, file_path: String) -> Result<String, String> {
    use std::time::Instant;

    println!("[BENCHMARK] Starting REALISTIC benchmark on: {}", file_path);

    let absolute_path = std::fs::canonicalize(&file_path)
        .or_else(|_| std::fs::canonicalize(format!("../{}", file_path)))
        .or_else(|_| std::fs::canonicalize(format!("../../{}", file_path)))
        .map_err(|e| format!("Could not find file at '{}'. Error: {}", file_path, e))?;

    println!("[BENCHMARK] Step 1: Loading WAV file...");
    let mut reader = hound::WavReader::open(&absolute_path)
        .map_err(|e| format!("Failed to open WAV file: {}", e))?;
    let spec = reader.spec();
    let sample_count = reader.len();

    let audio_duration_secs = sample_count as f32 / spec.sample_rate as f32 / spec.channels as f32;

    println!(
        "[BENCHMARK] Audio: {:.2}s, {}Hz, {} channels",
        audio_duration_secs, spec.sample_rate, spec.channels
    );

    let mut samples: Vec<f32> = Vec::with_capacity(sample_count as usize);
    if spec.sample_format == hound::SampleFormat::Float {
        samples.extend(reader.samples::<f32>().map(|s| s.unwrap_or(0.0)));
    } else {
        samples.extend(
            reader
                .samples::<i16>()
                .map(|s| s.unwrap_or(0) as f32 / 32768.0),
        );
    }

    let mono_samples = if spec.channels == 2 {
        samples
            .chunks(2)
            .map(|chunk| (chunk[0] + chunk[1]) / 2.0)
            .collect::<Vec<f32>>()
    } else {
        samples
    };

    let sample_rate = spec.sample_rate;
    let chunk_duration_secs = 6;
    let chunk_size = (sample_rate * chunk_duration_secs) as usize;
    let num_chunks = (mono_samples.len() + chunk_size - 1) / chunk_size;

    println!(
        "[BENCHMARK] Processing {} chunks of {}s each...",
        num_chunks, chunk_duration_secs
    );

    // Test Whisper
    state.whisper.lock().unwrap().clear_context();
    let start_whisper_naive = Instant::now();
    for chunk in mono_samples.chunks(chunk_size) {
        state
            .whisper
            .lock()
            .unwrap()
            .transcribe_chunk(chunk, sample_rate)
            .ok();
    }
    state
        .whisper
        .lock()
        .unwrap()
        .transcribe_file(absolute_path.to_str().unwrap())
        .ok();
    let time_whisper_naive = start_whisper_naive.elapsed();

    state.whisper.lock().unwrap().clear_context();
    let start_whisper_vad = Instant::now();
    let mut chunks_skipped = 0;
    for chunk in mono_samples.chunks(chunk_size) {
        let is_speech = state.vad.lock().unwrap().is_speech(chunk).unwrap_or(0.6);
        if is_speech > 0.5 {
            state
                .whisper
                .lock()
                .unwrap()
                .transcribe_chunk(chunk, sample_rate)
                .ok();
        } else {
            chunks_skipped += 1;
        }
    }
    {
        let mut whisper = state.whisper.lock().unwrap();
        let audio_data = whisper.load_audio(absolute_path.to_str().unwrap()).unwrap();
        let mut vad = state.vad.lock().unwrap();
        let timestamps = vad.get_speech_timestamps(&audio_data, 500).unwrap();
        let mut clean = Vec::new();
        for (s, e) in timestamps {
            let start = (s * 16000.0) as usize;
            let end = (e * 16000.0) as usize;
            clean.extend_from_slice(
                &audio_data[start.min(audio_data.len())..end.min(audio_data.len())],
            );
        }
        if !clean.is_empty() {
            whisper.transcribe_audio_data(&clean).ok();
        }
    }
    let time_whisper_vad = start_whisper_vad.elapsed();

    // Test Parakeet
    let parakeet_chunk_size = (sample_rate as f32 * 1.12) as usize;
    let parakeet_manager = state.parakeet.clone();

    let start_parakeet = Instant::now();
    for chunk in mono_samples.chunks(parakeet_chunk_size) {
        parakeet_manager
            .lock()
            .unwrap()
            .transcribe_chunk(chunk, sample_rate)
            .ok();
    }
    let time_parakeet = start_parakeet.elapsed();

    let factor_whisper = audio_duration_secs / time_whisper_vad.as_secs_f32();
    let factor_parakeet = audio_duration_secs / time_parakeet.as_secs_f32();

    let winner = if time_whisper_vad < time_parakeet {
        "Whisper AI"
    } else {
        "NVIDIA Parakeet"
    };

    Ok(format!(
        "📊 EXTENSIVE CUDA BENCHMARK RESULTS\n\
        ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\
        🎙️ WHISPER AI:\n\
        - Baseline (No VAD): {:.2}s\n\
        - Optimized (With VAD): {:.2}s\n\
        - Speed Factor: {:.1}x Real-time\n\n\
        🦜 NVIDIA PARAKEET:\n\
        - Streaming (No VAD): {:.2}s\n\
        - Speed Factor: {:.1}x Real-time\n\
        ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\
        🏆 WINNER: {} is faster on your system!\n\
        📉 Resource Usage: Whisper skipped {}/{} chunks",
        time_whisper_naive.as_secs_f32(),
        time_whisper_vad.as_secs_f32(),
        factor_whisper,
        time_parakeet.as_secs_f32(),
        factor_parakeet,
        winner,
        chunks_skipped,
        num_chunks
    ))
}
