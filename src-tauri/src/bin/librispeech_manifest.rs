//! Build JSONL manifest for LibriSpeech `test-clean` (utt_id, flac_path, ref_text).
//!
//! Usage:
//!   cargo run --bin librispeech_manifest -- --root "%LOCALAPPDATA%\Taurscribe\...\LibriSpeech\test-clean" --out manifest.jsonl
//! Optional: `--limit N` [--shuffle-seed U64] for a reproducible subset (shuffled then truncated).

use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
struct ManifestRow {
    utt_id: String,
    flac_path: String,
    ref_text: String,
}

fn usage() -> ! {
    eprintln!(
        "Usage: librispeech_manifest --root <path-to-test-clean> [--out manifest.jsonl] [--limit N] [--shuffle-seed U64]"
    );
    std::process::exit(2);
}

fn parse_args() -> (PathBuf, PathBuf, Option<usize>, Option<u64>) {
    let mut args = std::env::args().skip(1);
    let mut root: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut limit: Option<usize> = None;
    let mut shuffle_seed: Option<u64> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--root" => root = Some(PathBuf::from(args.next().unwrap_or_else(|| usage()))),
            "--out" => out = Some(PathBuf::from(args.next().unwrap_or_else(|| usage()))),
            "--limit" => {
                limit = Some(
                    args.next()
                        .unwrap_or_else(|| usage())
                        .parse()
                        .unwrap_or_else(|_| usage()),
                );
            }
            "--shuffle-seed" => {
                shuffle_seed = Some(
                    args.next()
                        .unwrap_or_else(|| usage())
                        .parse()
                        .unwrap_or_else(|_| usage()),
                );
            }
            "-h" | "--help" => usage(),
            _ => usage(),
        }
    }
    let root = root.unwrap_or_else(|| usage());
    let out = out.unwrap_or_else(|| PathBuf::from("eval_manifest.jsonl"));
    (root, out, limit, shuffle_seed)
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn shuffle_in_place<T>(items: &mut [T], seed: u64) {
    let n = items.len();
    for i in (1..n).rev() {
        let j = (splitmix64(seed.wrapping_add(i as u64)) as usize) % (i + 1);
        items.swap(i, j);
    }
}

fn visit_trans_txt(dir: &Path, rows: &mut Vec<ManifestRow>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            visit_trans_txt(&path, rows)?;
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".trans.txt"))
        {
            parse_trans_txt(&path, rows)?;
        }
    }
    Ok(())
}

fn parse_trans_txt(path: &Path, rows: &mut Vec<ManifestRow>) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "trans.txt has no parent")
    })?;
    let text = std::fs::read_to_string(path)?;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((utt_id, ref_text)) = line.split_once(' ') else {
            continue;
        };
        let flac = parent.join(format!("{utt_id}.flac"));
        if !flac.is_file() {
            eprintln!("[manifest] skip (no .flac): {}", flac.display());
            continue;
        }
        rows.push(ManifestRow {
            utt_id: utt_id.to_string(),
            flac_path: flac.to_string_lossy().into_owned(),
            ref_text: ref_text.trim().to_string(),
        });
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (root, out_path, limit, shuffle_seed) = parse_args();
    if !root.is_dir() {
        return Err(format!("--root is not a directory: {}", root.display()).into());
    }

    let mut rows: Vec<ManifestRow> = Vec::new();
    visit_trans_txt(&root, &mut rows)?;
    rows.sort_by(|a, b| a.utt_id.cmp(&b.utt_id));

    if let Some(seed) = shuffle_seed {
        shuffle_in_place(&mut rows, seed);
    }

    if let Some(n) = limit {
        rows.truncate(n);
    }

    let mut w = std::io::BufWriter::new(std::fs::File::create(&out_path)?);
    for row in &rows {
        serde_json::to_writer(&mut w, row)?;
        w.write_all(b"\n")?;
    }
    w.flush()?;

    eprintln!(
        "Wrote {} utterances to {}",
        rows.len(),
        out_path.display()
    );
    Ok(())
}
