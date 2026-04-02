//! Integration memory regression test for ASR engines.
//!
//! Exercises the same engine managers the app uses today across:
//! - standalone load/transcribe/unload cycles
//! - repeated transcriptions on the same loaded engine
//! - cross-engine switching sequences
//! - explicit unload verification after each sequence
//!
//! Requires:
//! - `jfk.wav` at `tests/fixtures/jfk.wav`, repo root, or `JFK_WAV`
//! - Whisper / Parakeet / Cohere models installed under `%LOCALAPPDATA%\Taurscribe\models`
//!
//! Run with:
//!   cargo test memory_engine_regression -- --ignored --nocapture
//!
//! Optional env vars:
//! - `TAURSCRIBE_ASR_SMOKE_SKIP=1` skip the test
//! - `TAURSCRIBE_MEMORY_REPORT=path.json` write a JSON report
//! - `TAURSCRIBE_MEMORY_FORCE_CPU=1` force CPU loads where supported

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;
use taurscribe_lib::cohere::CohereManager;
use taurscribe_lib::memory::{process_memory_stats, ProcessMemoryStats};
use taurscribe_lib::parakeet::ParakeetManager;
use taurscribe_lib::parakeet_loaders::ParakeetLoadPath;
use taurscribe_lib::whisper::WhisperManager;

#[derive(Debug, Serialize)]
struct MemorySnapshot {
    label: String,
    working_set_bytes: u64,
    private_bytes: Option<u64>,
    virtual_bytes: Option<u64>,
    peak_working_set_bytes: Option<u64>,
    source: String,
}

impl MemorySnapshot {
    fn ws_i64(&self) -> i64 {
        self.working_set_bytes as i64
    }
}

#[derive(Debug, Serialize)]
struct ScenarioReport {
    name: String,
    duration_ms: u128,
    snapshots: Vec<MemorySnapshot>,
    notes: Vec<String>,
}

#[derive(Debug, Serialize)]
struct MemoryRegressionReport {
    audio_fixture: String,
    force_cpu: bool,
    sample_count_16k: usize,
    scenarios: Vec<ScenarioReport>,
}

fn snapshot(label: impl Into<String>) -> MemorySnapshot {
    let ProcessMemoryStats {
        working_set_bytes,
        private_bytes,
        virtual_bytes,
        peak_working_set_bytes,
        source,
    } = process_memory_stats();

    MemorySnapshot {
        label: label.into(),
        working_set_bytes,
        private_bytes,
        virtual_bytes,
        peak_working_set_bytes,
        source,
    }
}

fn bytes_to_mb(v: u64) -> f64 {
    v as f64 / (1024.0 * 1024.0)
}

fn maybe_write_report(report: &MemoryRegressionReport) {
    if let Ok(path) = std::env::var("TAURSCRIBE_MEMORY_REPORT") {
        let json = serde_json::to_string_pretty(report)
            .unwrap_or_else(|e| panic!("failed to serialize memory report: {e}"));
        fs::write(&path, json)
            .unwrap_or_else(|e| panic!("failed to write memory report to {path}: {e}"));
        eprintln!("[memory-test] wrote report to {path}");
    }
}

fn resolve_jfk_wav() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("JFK_WAV") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for rel in ["tests/fixtures/jfk.wav", "../jfk.wav"] {
        let p = manifest.join(rel);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn load_wav_mono_f32(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let mut reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    let rate = spec.sample_rate;
    let ch = spec.channels.max(1) as usize;

    let mono: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => {
            let interleaved: Vec<f32> = reader
                .samples::<f32>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            if ch <= 1 {
                interleaved
            } else {
                interleaved
                    .chunks(ch)
                    .map(|fr| fr.iter().sum::<f32>() / ch as f32)
                    .collect()
            }
        }
        hound::SampleFormat::Int => {
            if spec.bits_per_sample != 16 {
                return Err(format!(
                    "test helper supports only 16-bit int WAV, got {} bits",
                    spec.bits_per_sample
                ));
            }
            let interleaved: Vec<i16> = reader
                .samples::<i16>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            let g = 1.0 / 32768.0;
            if ch <= 1 {
                interleaved.iter().map(|&x| x as f32 * g).collect()
            } else {
                interleaved
                    .chunks(ch)
                    .map(|fr| fr.iter().map(|&x| x as f32).sum::<f32>() / ch as f32 * g)
                    .collect()
            }
        }
    };

    Ok((mono, rate))
}

fn jfk_pcm16_preprocessed_for_asr() -> Result<(PathBuf, Vec<f32>), String> {
    let path = resolve_jfk_wav()
        .ok_or("jfk.wav not found (tests/fixtures/jfk.wav, ../jfk.wav, or JFK_WAV)")?;
    let (mono, rate) = load_wav_mono_f32(&path)?;
    if mono.is_empty() {
        return Err("jfk.wav has no samples".into());
    }
    let mut pcm16 = taurscribe_lib::audio_preprocess::resample_mono_to_16k(&mono, rate)?;
    taurscribe_lib::audio_preprocess::trim_file_buffer_edges_16k(&mut pcm16);
    if pcm16.is_empty() {
        return Err("edge trim emptied jfk buffer".into());
    }
    taurscribe_lib::audio_preprocess::preprocess_assembled_speech_16k(&mut pcm16);
    if pcm16.is_empty() {
        return Err("preprocess emptied jfk buffer".into());
    }
    Ok((path, pcm16))
}

fn signed_mb(delta_bytes: i64) -> String {
    let mb = delta_bytes as f64 / (1024.0 * 1024.0);
    if delta_bytes >= 0 {
        format!("+{:.1}", mb)
    } else {
        format!("{:.1}", mb)
    }
}

fn summarize_report(report: &MemoryRegressionReport) {
    eprintln!(
        "\n[memory-test] ═══════════════════════════════════════════════════════"
    );
    eprintln!("[memory-test]  MEMORY REGRESSION SUMMARY");
    eprintln!(
        "[memory-test]  fixture : {} ({} samples, {:.1}s @ 16 kHz)",
        report.audio_fixture,
        report.sample_count_16k,
        report.sample_count_16k as f64 / 16_000.0,
    );
    eprintln!("[memory-test]  force_cpu: {}", report.force_cpu);
    eprintln!(
        "[memory-test] ═══════════════════════════════════════════════════════"
    );

    let mut all_pass = true;

    for scenario in &report.scenarios {
        eprintln!(
            "\n[memory-test] ── {} ({} ms) ──────────────────────────",
            scenario.name, scenario.duration_ms,
        );

        if scenario.snapshots.is_empty() {
            for note in &scenario.notes {
                eprintln!("  note: {note}");
            }
            continue;
        }

        let baseline_ws = scenario.snapshots[0].working_set_bytes as i64;

        // Per-snapshot table
        eprintln!(
            "  {:<48} {:>9}  {:>10}  {:>10}  {:>12}",
            "snapshot", "WS (MB)", "Δ prev", "Δ baseline", "private (MB)",
        );
        eprintln!("  {}", "─".repeat(96));

        let baseline_private = scenario.snapshots[0].private_bytes.map(|v| v as i64);

        // First pass: collect row data and find the single biggest positive jump
        struct RowData {
            label: String,
            ws: i64,
            delta_prev: i64,
            delta_baseline: i64,
            private_str: String,
        }
        let mut rows: Vec<RowData> = Vec::with_capacity(scenario.snapshots.len());
        let mut prev_ws = baseline_ws;
        let mut peak_ws: i64 = 0;
        let mut after_unload_ws: Option<i64> = None;
        let mut after_unload_private: Option<i64> = None;

        for snap in &scenario.snapshots {
            let ws = snap.ws_i64();
            let delta_prev = ws - prev_ws;
            let delta_baseline = ws - baseline_ws;
            let private_str = snap
                .private_bytes
                .map(|b| format!("{:.1}", bytes_to_mb(b)))
                .unwrap_or_else(|| "—".to_string());

            if ws > peak_ws {
                peak_ws = ws;
            }
            if snap.label.contains("after unload") || snap.label.contains("after final unload") {
                after_unload_ws = Some(ws);
                after_unload_private = snap.private_bytes.map(|b| b as i64);
            }

            rows.push(RowData { label: snap.label.clone(), ws, delta_prev, delta_baseline, private_str });
            prev_ws = ws;
        }

        // Find index of the single largest positive jump (for spike marker)
        let spike_idx = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.delta_prev > 100 * 1024 * 1024)
            .max_by_key(|(_, r)| r.delta_prev)
            .map(|(i, _)| i);

        let biggest_jump = spike_idx.map(|i| rows[i].delta_prev).unwrap_or(0);
        let biggest_jump_label = spike_idx.map(|i| rows[i].label.clone()).unwrap_or_default();

        // Second pass: print table with spike marker only on the single spike row
        for (i, row) in rows.iter().enumerate() {
            let spike_marker = if Some(i) == spike_idx { " ← SPIKE" } else { "" };
            eprintln!(
                "  {:<48} {:>9.1}  {:>10}  {:>10}  {:>12}{}",
                row.label,
                bytes_to_mb(row.ws as u64),
                signed_mb(row.delta_prev),
                signed_mb(row.delta_baseline),
                row.private_str,
                spike_marker,
            );
        }

        eprintln!("  {}", "─".repeat(96));

        // Spike annotation
        if biggest_jump > 100 * 1024 * 1024 {
            let is_first_inference = biggest_jump_label.contains("first transcription")
                || biggest_jump_label.contains("first transcription");
            let interpretation = if is_first_inference {
                "likely CUDA/ORT memory-mapped at first inference — not additional system RAM"
            } else {
                "review snapshot above"
            };
            eprintln!(
                "  SPIKE     +{:.0} MB at \"{}\"",
                bytes_to_mb(biggest_jump as u64),
                biggest_jump_label,
            );
            eprintln!("            interpretation: {}", interpretation);
            eprintln!(
                "            working_set vs private: WS spike reflects GPU VRAM pages mapped \
                into process address space — compare private_bytes for true RAM pressure"
            );
        }

        // Leak check
        // Threshold: allow up to 200 MB above baseline for CUDA/ORT page residuals
        const LEAK_THRESHOLD_BYTES: i64 = 200 * 1024 * 1024;
        if let Some(unload_ws) = after_unload_ws {
            let retained = unload_ws - baseline_ws;
            let pass = retained <= LEAK_THRESHOLD_BYTES;
            if !pass {
                all_pass = false;
            }
            eprintln!(
                "  LEAK WS   baseline={:.1} MB  after-unload={:.1} MB  retained={}  {}",
                bytes_to_mb(baseline_ws as u64),
                bytes_to_mb(unload_ws as u64),
                signed_mb(retained),
                if pass { "✓ CLEAN" } else { "✗ POSSIBLE LEAK" },
            );
            // Private bytes: expect them to stay elevated after CUDA init (ORT holds pages)
            if let (Some(unload_priv), Some(base_priv)) = (after_unload_private, baseline_private) {
                let priv_retained = unload_priv - base_priv;
                // Private bytes leak threshold is 1 GB (ORT/CUDA virtual commit is expected)
                let priv_pass = priv_retained <= 1024 * 1024 * 1024;
                if !priv_pass {
                    all_pass = false;
                }
                eprintln!(
                    "  LEAK PRIV baseline={:.1} MB  after-unload={:.1} MB  retained={}  {} \
                    (ORT keeps virtual commit after first CUDA init)",
                    bytes_to_mb(base_priv as u64),
                    bytes_to_mb(unload_priv as u64),
                    signed_mb(priv_retained),
                    if priv_pass { "✓ expected" } else { "✗ EXCESSIVE" },
                );
            }
        }

        // Notes
        if !scenario.notes.is_empty() {
            eprint!("  notes     ");
            eprintln!("{}", scenario.notes.join(" | "));
        }
    }

    // Roll-up
    eprintln!(
        "\n[memory-test] ═══════════════════════════════════════════════════════"
    );
    eprintln!(
        "[memory-test]  OVERALL: {}",
        if all_pass { "✓ PASS — no leaks detected" } else { "✗ FAIL — possible leak(s) above" }
    );
    eprintln!(
        "[memory-test] ═══════════════════════════════════════════════════════"
    );
}

fn skipped_scenario(name: &str, reason: impl Into<String>) -> ScenarioReport {
    ScenarioReport {
        name: name.to_string(),
        duration_ms: 0,
        snapshots: vec![snapshot("skipped")],
        notes: vec![reason.into()],
    }
}

fn run_whisper_cycle(
    pcm: &[f32],
    whisper_model_id: &str,
    force_cpu: bool,
) -> Result<ScenarioReport, String> {
    let started = Instant::now();
    let mut snapshots = vec![snapshot("baseline")];
    let mut notes = Vec::new();

    let mut w = WhisperManager::new();
    w.initialize(Some(whisper_model_id), force_cpu)?;
    snapshots.push(snapshot("whisper after initialize"));

    let text1 = w.transcribe_audio_data(pcm, None)?;
    notes.push(format!("first transcript chars={}", text1.len()));
    snapshots.push(snapshot("whisper after first transcription"));

    let text2 = w.transcribe_audio_data(pcm, None)?;
    notes.push(format!("second transcript chars={}", text2.len()));
    snapshots.push(snapshot("whisper after second transcription"));

    w.unload();
    snapshots.push(snapshot("whisper after unload"));
    assert!(
        w.get_current_model().is_none(),
        "whisper should be unloaded"
    );

    Ok(ScenarioReport {
        name: "whisper_cycle".to_string(),
        duration_ms: started.elapsed().as_millis(),
        snapshots,
        notes,
    })
}

fn run_parakeet_cycle(
    pcm: &[f32],
    parakeet_model_id: &str,
    force_cpu: bool,
) -> Result<ScenarioReport, String> {
    run_parakeet_load_path_cycle(
        "parakeet_cycle",
        pcm,
        parakeet_model_id,
        if force_cpu {
            ParakeetLoadPath::Cpu
        } else {
            ParakeetLoadPath::FallbackGpu
        },
    )
}

fn run_parakeet_load_path_cycle(
    name: &str,
    pcm: &[f32],
    parakeet_model_id: &str,
    load_path: ParakeetLoadPath,
) -> Result<ScenarioReport, String> {
    let started = Instant::now();
    let mut snapshots = vec![snapshot("baseline")];
    let mut notes = Vec::new();

    let mut p = ParakeetManager::new();
    notes.push(format!("load_path={load_path}"));
    p.initialize_with_load_path(
        Some(parakeet_model_id),
        load_path == ParakeetLoadPath::Cpu,
        load_path,
    )?;
    snapshots.push(snapshot(format!("parakeet after initialize ({load_path})")));

    let text1 = p.transcribe_chunk(pcm, 16000)?;
    notes.push(format!("first transcript chars={}", text1.len()));
    snapshots.push(snapshot(format!(
        "parakeet after first transcription ({load_path})"
    )));

    let text2 = p.transcribe_chunk(pcm, 16000)?;
    notes.push(format!("second transcript chars={}", text2.len()));
    snapshots.push(snapshot(format!(
        "parakeet after second transcription ({load_path})"
    )));

    p.unload();
    snapshots.push(snapshot(format!("parakeet after unload ({load_path})")));
    assert!(!p.get_status().loaded, "parakeet should be unloaded");

    Ok(ScenarioReport {
        name: name.to_string(),
        duration_ms: started.elapsed().as_millis(),
        snapshots,
        notes,
    })
}

fn run_cohere_cycle(pcm: &[f32], force_cpu: bool) -> Result<ScenarioReport, String> {
    let started = Instant::now();
    let mut snapshots = vec![snapshot("baseline")];
    let mut notes = Vec::new();

    let mut g = CohereManager::new();
    g.initialize(None, force_cpu)?;
    snapshots.push(snapshot("cohere after initialize"));

    let text1 = g.transcribe_chunk(pcm, 16000)?;
    notes.push(format!("first transcript chars={}", text1.len()));
    snapshots.push(snapshot("cohere after first transcription"));

    let text2 = g.transcribe_chunk(pcm, 16000)?;
    notes.push(format!("second transcript chars={}", text2.len()));
    snapshots.push(snapshot("cohere after second transcription"));

    g.unload();
    snapshots.push(snapshot("cohere after unload"));
    assert!(!g.get_status().loaded, "cohere should be unloaded");

    Ok(ScenarioReport {
        name: "cohere_cycle".to_string(),
        duration_ms: started.elapsed().as_millis(),
        snapshots,
        notes,
    })
}

fn run_switch_sequence(
    name: &str,
    pcm: &[f32],
    whisper_model_id: &str,
    parakeet_model_id: &str,
    force_cpu: bool,
    steps: &[&str],
) -> Result<ScenarioReport, String> {
    let started = Instant::now();
    let mut snapshots = vec![snapshot("baseline")];
    let mut notes = Vec::new();

    let mut w = WhisperManager::new();
    let mut p = ParakeetManager::new();
    let mut g = CohereManager::new();

    for step in steps {
        match *step {
            "whisper" => {
                p.unload();
                g.unload();
                snapshots.push(snapshot("after outgoing unloads before whisper init"));
                w.initialize(Some(whisper_model_id), force_cpu)?;
                snapshots.push(snapshot("after whisper init"));
                let text = w.transcribe_audio_data(pcm, None)?;
                notes.push(format!("whisper transcript chars={}", text.len()));
                snapshots.push(snapshot("after whisper transcription"));
            }
            "parakeet" => {
                w.unload();
                g.unload();
                snapshots.push(snapshot("after outgoing unloads before parakeet init"));
                p.initialize(Some(parakeet_model_id), force_cpu)?;
                snapshots.push(snapshot("after parakeet init"));
                let text = p.transcribe_chunk(pcm, 16000)?;
                notes.push(format!("parakeet transcript chars={}", text.len()));
                snapshots.push(snapshot("after parakeet transcription"));
            }
            "cohere" => {
                w.unload();
                p.unload();
                snapshots.push(snapshot("after outgoing unloads before cohere init"));
                g.initialize(None, force_cpu)?;
                snapshots.push(snapshot("after cohere init"));
                let text = g.transcribe_chunk(pcm, 16000)?;
                notes.push(format!("cohere transcript chars={}", text.len()));
                snapshots.push(snapshot("after cohere transcription"));
            }
            other => return Err(format!("unknown switch step: {other}")),
        }
    }

    w.unload();
    p.unload();
    g.unload();
    snapshots.push(snapshot("after final unloads"));
    assert!(
        w.get_current_model().is_none(),
        "whisper should be unloaded at end"
    );
    assert!(!p.get_status().loaded, "parakeet should be unloaded at end");
    assert!(!g.get_status().loaded, "cohere should be unloaded at end");

    Ok(ScenarioReport {
        name: name.to_string(),
        duration_ms: started.elapsed().as_millis(),
        snapshots,
        notes,
    })
}

#[test]
#[ignore = "Needs JFK fixture + installed Whisper, Parakeet, and Cohere models. Run with --ignored --nocapture."]
fn memory_engine_regression() {
    if std::env::var("TAURSCRIBE_ASR_SMOKE_SKIP").as_deref() == Ok("1") {
        eprintln!("SKIP memory_engine_regression (TAURSCRIBE_ASR_SMOKE_SKIP=1)");
        return;
    }

    let force_cpu = std::env::var("TAURSCRIBE_MEMORY_FORCE_CPU").as_deref() == Ok("1");
    let (fixture_path, pcm) =
        jfk_pcm16_preprocessed_for_asr().unwrap_or_else(|e| panic!("fixture error: {e}"));
    assert!(
        pcm.len() > 8000,
        "preprocessed fixture too short: {} samples",
        pcm.len()
    );

    let whisper_models = WhisperManager::list_available_models()
        .unwrap_or_else(|e| panic!("Whisper list models failed: {e}"));
    assert!(
        !whisper_models.is_empty(),
        "Whisper model required for memory regression test"
    );
    let whisper_model_id = whisper_models[0].id.clone();

    let parakeet_models = ParakeetManager::list_available_models()
        .unwrap_or_else(|e| panic!("Parakeet list models failed: {e}"));
    assert!(
        !parakeet_models.is_empty(),
        "Parakeet model required for memory regression test"
    );
    let parakeet_model_id = parakeet_models[0].id.clone();

    let mut scenarios = Vec::new();
    scenarios.push(
        match run_whisper_cycle(&pcm, &whisper_model_id, force_cpu) {
            Ok(report) => report,
            Err(e) => panic!("whisper cycle failed: {e}"),
        },
    );
    scenarios.push(
        match run_parakeet_cycle(&pcm, &parakeet_model_id, force_cpu) {
            Ok(report) => report,
            Err(e) => panic!("parakeet cycle failed: {e}"),
        },
    );
    if !force_cpu {
        scenarios.push(
            match run_parakeet_load_path_cycle(
                "parakeet_strict_gpu_cycle",
                &pcm,
                &parakeet_model_id,
                ParakeetLoadPath::StrictGpu,
            ) {
                Ok(report) => report,
                Err(e) => skipped_scenario(
                    "parakeet_strict_gpu_cycle",
                    format!("strict GPU unavailable: {e}"),
                ),
            },
        );
        scenarios.push(
            match run_parakeet_load_path_cycle(
                "parakeet_fallback_gpu_cycle",
                &pcm,
                &parakeet_model_id,
                ParakeetLoadPath::FallbackGpu,
            ) {
                Ok(report) => report,
                Err(e) => skipped_scenario(
                    "parakeet_fallback_gpu_cycle",
                    format!("fallback GPU unavailable: {e}"),
                ),
            },
        );
    }
    scenarios.push(
        match run_parakeet_load_path_cycle(
            "parakeet_cpu_cycle",
            &pcm,
            &parakeet_model_id,
            ParakeetLoadPath::Cpu,
        ) {
            Ok(report) => report,
            Err(e) => skipped_scenario("parakeet_cpu_cycle", format!("cpu-only run failed: {e}")),
        },
    );
    scenarios.push(match run_cohere_cycle(&pcm, force_cpu) {
        Ok(report) => report,
        Err(e) => skipped_scenario("cohere_cycle", format!("skip: {e}")),
    });
    scenarios.push(
        match run_switch_sequence(
            "switch_whisper_parakeet_whisper",
            &pcm,
            &whisper_model_id,
            &parakeet_model_id,
            force_cpu,
            &["whisper", "parakeet", "whisper"],
        ) {
            Ok(report) => report,
            Err(e) => panic!("whisper→parakeet→whisper failed: {e}"),
        },
    );
    scenarios.push(
        match run_switch_sequence(
            "switch_whisper_cohere_whisper",
            &pcm,
            &whisper_model_id,
            &parakeet_model_id,
            force_cpu,
            &["whisper", "cohere", "whisper"],
        ) {
            Ok(report) => report,
            Err(e) => skipped_scenario("switch_whisper_cohere_whisper", format!("skip: {e}")),
        },
    );
    scenarios.push(
        match run_switch_sequence(
            "switch_parakeet_cohere_parakeet",
            &pcm,
            &whisper_model_id,
            &parakeet_model_id,
            force_cpu,
            &["parakeet", "cohere", "parakeet"],
        ) {
            Ok(report) => report,
            Err(e) => skipped_scenario("switch_parakeet_cohere_parakeet", format!("skip: {e}")),
        },
    );

    let report = MemoryRegressionReport {
        audio_fixture: fixture_path.display().to_string(),
        force_cpu,
        sample_count_16k: pcm.len(),
        scenarios,
    };

    summarize_report(&report);
    maybe_write_report(&report);

    // Hard leak assertion: after-unload working set must be within 200 MB of baseline
    // for every scenario that completed (non-skipped, has an unload snapshot).
    const LEAK_THRESHOLD_BYTES: u64 = 200 * 1024 * 1024;
    for scenario in &report.scenarios {
        if scenario.duration_ms == 0 {
            continue; // skipped
        }
        let baseline_ws = scenario
            .snapshots
            .first()
            .map(|s| s.working_set_bytes)
            .unwrap_or(0);
        if let Some(unload_snap) = scenario.snapshots.iter().find(|s| {
            s.label.contains("after unload") || s.label.contains("after final unload")
        }) {
            let after_unload_ws = unload_snap.working_set_bytes;
            let retained = after_unload_ws.saturating_sub(baseline_ws);
            assert!(
                retained <= LEAK_THRESHOLD_BYTES,
                "[memory-test] LEAK DETECTED in scenario '{}': \
                after-unload WS={:.1} MB, baseline={:.1} MB, retained={:.1} MB (threshold={:.0} MB). \
                Check engine unload paths.",
                scenario.name,
                bytes_to_mb(after_unload_ws),
                bytes_to_mb(baseline_ws),
                bytes_to_mb(retained),
                bytes_to_mb(LEAK_THRESHOLD_BYTES),
            );
        }
    }
}
