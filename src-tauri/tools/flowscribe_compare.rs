//! Runs FlowScribe on transcripts exactly as the app does (same engine code,
//! prompts, guards and settings).
//!
//! Usage:
//!   cargo run --release --features dev-tools --bin flowscribe_compare -- in.jsonl out.jsonl
//! Input lines: {"text": ..., optional "engine", "level", "app", "vocab", "prev"}.
//! Output adds "output" and "ms". FLOWSCRIBE_V3_GGUF can point at another GGUF.

use std::io::{BufRead, Write};
use taurscribe_lib::llm::{FlowRequest, LLMEngine};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (inp, outp) = (&args[1], &args[2]);
    let mut engine = LLMEngine::new(true)?;
    let mut out = std::fs::File::create(outp)?;
    for line in std::io::BufReader::new(std::fs::File::open(inp)?).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let mut row: serde_json::Value = serde_json::from_str(&line)?;
        let s = |k: &str, d: &str| row[k].as_str().unwrap_or(d).to_string();
        let text = s("text", "");
        let req = FlowRequest {
            engine: s("engine", "whisper"),
            level: s("level", "clean"),
            app: s("app", "generic"),
            vocab: row["vocab"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default(),
            prev: row["prev"].as_str().map(String::from),
        };
        let t0 = std::time::Instant::now();
        let output = engine.clean_transcript(&text, &req)?;
        row["output"] = output.into();
        row["ms"] = (t0.elapsed().as_millis() as u64).into();
        writeln!(out, "{row}")?;
    }
    Ok(())
}
