//! Times the ASR and LLM engines on the backends this build has.
//!
//! Usage:
//!   cargo run --release --features dev-tools --bin engine_bench -- whisper <model_id> <audio> [--cpu] [--runs N]
//!   cargo run --release --features dev-tools --bin engine_bench -- llm [--cpu] [--runs N]
//!   cargo run --release --features dev-tools --bin engine_bench -- gpu   (what gpu.rs detects)
//!   cargo run --release --features dev-tools --bin engine_bench -- models-dir
//!
//! Prints the backend each engine reports, then the time per run and the
//! real-time factor (audio seconds / processing second) for ASR.

use std::time::Instant;
use taurscribe_lib::llm::{FlowRequest, LLMEngine};
use taurscribe_lib::whisper::WhisperManager;

const LLM_TEXT: &str = "so um basically what I wanted to say is that the the quarterly numbers look \
    pretty good but we need to uh double check the marketing budget before friday because \
    sarah mentioned that it might be off by like ten percent or something";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cpu = args.iter().any(|a| a == "--cpu");
    let runs = args
        .iter()
        .position(|a| a == "--runs")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(3usize);
    match args.first().map(String::as_str) {
        Some("whisper") => {
            let (Some(model), Some(audio)) = (args.get(1), args.get(2)) else {
                eprintln!("usage: engine_bench whisper <model_id> <audio> [--cpu] [--runs N]");
                std::process::exit(2);
            };
            let mut w = WhisperManager::new();
            let t = Instant::now();
            let backend = w.initialize(Some(model), cpu).expect("whisper init");
            println!("[bench] whisper {model}: {backend} · load {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
            let pcm = w.load_audio(audio).expect("audio");
            let secs = pcm.len() as f64 / 16_000.0;
            for i in 0..runs {
                let t = Instant::now();
                let text = w.transcribe_audio_data(&pcm, None).expect("transcribe");
                let dt = t.elapsed().as_secs_f64();
                println!("[bench] run {i}: {:.0} ms · {:.1}x real time · {} words", dt * 1e3, secs / dt, text.split_whitespace().count());
                if i + 1 == runs {
                    println!("[bench] text: {}", text.trim());
                }
            }
        }
        Some("llm") => {
            let t = Instant::now();
            let mut e = LLMEngine::new(!cpu).expect("llm init");
            println!("[bench] flowscribe gpu={} · load {:.0} ms", !cpu, t.elapsed().as_secs_f64() * 1e3);
            let req = FlowRequest { engine: "whisper".into(), level: "clean".into(), app: "other".into(), vocab: vec![], prev: None };
            for i in 0..runs {
                let t = Instant::now();
                let out = e.clean_transcript(LLM_TEXT, &req).expect("llm");
                println!("[bench] run {i}: {:.0} ms · {} chars out", t.elapsed().as_secs_f64() * 1e3, out.len());
            }
        }
        Some("models-dir") => {
            println!("{}", taurscribe_lib::utils::get_models_dir().expect("models dir").display());
        }
        Some("gpu") => {
            println!("{}", serde_json::to_string_pretty(&taurscribe_lib::gpu::report()).unwrap());
        }
        _ => {
            eprintln!("usage: engine_bench whisper <model_id> <audio> [--cpu] | llm [--cpu]");
            std::process::exit(2);
        }
    }
}
