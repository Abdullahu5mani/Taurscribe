//! Latency / accuracy benchmark for the current GGUF ASR engines.
//! One model per process so memory numbers are clean.
//!
//! usage: cargo run --release --example asr_latency_bench -- \
//!          <granite|qwen3> <model.gguf> <librispeech_test_clean_dir> <n_utts> <out.json>
//!
//! Per utterance it reports whole-utterance inference latency, WER, and memory.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn main() {
    use std::path::{Path, PathBuf};
    use std::time::Instant;
    use taurscribe_lib::librispeech_wer::{normalize_for_wer, word_error_rate};

    const STOP_SILENCE: usize = 6400; // 400 ms appended on release, as the app does

    let a: Vec<String> = std::env::args().collect();
    let (engine, model_dir, libri, n, out) = (a[1].as_str(), Path::new(&a[2]), Path::new(&a[3]), a[4].parse::<usize>().unwrap(), &a[5]);
    assert!(matches!(engine, "granite" | "qwen3"), "expected granite or qwen3");

    // Deterministic utterance pick: every k-th file of the sorted corpus.
    let mut all: Vec<(PathBuf, String)> = Vec::new();
    for spk in std::fs::read_dir(libri).unwrap().flatten() {
        for ch in std::fs::read_dir(spk.path()).unwrap().flatten() {
            for f in std::fs::read_dir(ch.path()).unwrap().flatten() {
                let p = f.path();
                if p.extension().is_some_and(|e| e == "txt") {
                    for line in std::fs::read_to_string(&p).unwrap().lines() {
                        let (id, text) = line.split_once(' ').unwrap();
                        all.push((ch.path().join(format!("{id}.flac")), text.to_string()));
                    }
                }
            }
        }
    }
    all.sort();
    let step = (all.len() / n).max(1);
    let picks: Vec<_> = all.iter().step_by(step).take(n).cloned().collect();

    let mem = || taurscribe_lib::memory::process_memory_stats().private_bytes.unwrap_or(0) as f64 / 1e6;
    let base_mb = mem();

    let t = Instant::now();
    let mut eng = taurscribe_lib::gguf_asr::GgufAsr::load(model_dir).unwrap();
    let load_s = t.elapsed().as_secs_f64();
    // Warm-up, as the app does after loading.
    let t = Instant::now();
    eng.transcribe(&vec![0.0; 16_000]).unwrap();
    let warm_s = t.elapsed().as_secs_f64();
    let loaded_mb = mem();

    let (mut audio_s, mut compute_s, mut errs, mut words) = (0.0, 0.0, 0.0, 0usize);
    let mut release = Vec::new();
    let mut worst_step: f64 = 0.0;
    let mut peak_mb = loaded_mb;
    let mut rows = Vec::new();
    for (path, reference) in &picks {
        let (mut x, rate) = taurscribe_lib::audio_decode::decode_audio_mono_f32(path).unwrap();
        assert_eq!(rate, 16_000);
        let secs = x.len() as f64 / 16_000.0;
        x.extend(std::iter::repeat_n(0.0, STOP_SILENCE));
        let t = Instant::now();
        let text = eng.transcribe(&x).unwrap();
        let total = t.elapsed().as_secs_f64();
        let rel = total;
        let step_max = total;
        peak_mb = peak_mb.max(mem());
        let r = normalize_for_wer(reference);
        let h = normalize_for_wer(&text);
        let wer = word_error_rate(&r, &h);
        errs += wer * r.len() as f64;
        words += r.len();
        audio_s += secs;
        compute_s += total;
        worst_step = worst_step.max(step_max);
        release.push(rel);
        rows.push(serde_json::json!({"file": path.file_name().unwrap().to_string_lossy(), "secs": secs,
            "compute_ms": total * 1e3, "release_ms": rel * 1e3, "wer": wer, "text": text.trim(), "ref": reference}));
    }
    release.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |p: f64| release[((release.len() - 1) as f64 * p).round() as usize] * 1e3;
    let summary = serde_json::json!({
        "engine": engine, "utterances": picks.len(), "audio_s": audio_s,
        "load_s": load_s, "warmup_s": warm_s,
        "rtfx": audio_s / compute_s,
        "release_ms_p50": pct(0.5), "release_ms_p90": pct(0.9), "release_ms_max": pct(1.0),
        "worst_single_step_ms": worst_step * 1e3,
        "wer": errs / words as f64,
        "memory_mb_loaded": loaded_mb - base_mb, "memory_mb_peak": peak_mb - base_mb,
    });
    println!("{}", serde_json::to_string_pretty(&summary).unwrap());
    std::fs::write(out, serde_json::to_string_pretty(&serde_json::json!({"summary": summary, "rows": rows})).unwrap()).unwrap();
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn main() {}
