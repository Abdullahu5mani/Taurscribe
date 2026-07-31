//! Parity gate for the Rust MLX encoder against the Python reference.
//!
//! Run the capture side first (it writes the .npy files this reads):
//!   scripts/granite_mlx/  python -c "...capture..."   # see repo docs
//!   cargo run --bin granite_mlx_parity -- <model-dir>
//!
//! Passes when the worst relative difference is at fp32 noise level. Anything
//! larger means the port drifted from the reference and would decode to
//! plausible-but-wrong text.

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("macOS only");
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::path::PathBuf;

    use mlx_rs::ops::indexing::IndexOp;
    use mlx_rs::{Array, Dtype};
    use taurscribe_lib::granite_mlx::{
        editor::{add_insertion_slots, EditorConfig},
        encoder::EncoderConfig,
        load_weights, resolve_layer_indices, CtcEncoder, Editor, Projector, ProjectorConfig,
    };

    let model_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: granite_mlx_parity <model-dir>")?;

    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(model_dir.join("config.json"))?)?;
    let cfg = EncoderConfig::from_json(&config)?;
    println!(
        "encoder: {} layers, hidden {}, ctx {}, self-cond @ {}",
        cfg.num_layers, cfg.hidden_dim, cfg.context_size, cfg.self_conditioning_layer
    );

    // fp32 so the comparison measures the port, not rounding.
    let t = std::time::Instant::now();
    let proj_cfg = ProjectorConfig::from_json(&config)?;
    let ed_cfg = EditorConfig::from_json(&config)?;
    let weights = load_weights(&model_dir, &cfg, &proj_cfg, &ed_cfg, Dtype::Float32)?;
    let encoder = CtcEncoder::load(&weights, cfg.clone())?;
    let projector = Projector::load(&weights, proj_cfg.clone())?;
    let editor = Editor::load(&weights, ed_cfg.clone())?;
    println!("loaded {} tensors in {:.2}s", weights.len(), t.elapsed().as_secs_f32());

    let feats = load_npy_f32("/tmp/parity_feats.npy")?;
    let features = Array::from_slice(&feats.1, &feats.0.iter().map(|&d| d as i32).collect::<Vec<_>>());

    let raw: Vec<i64> = config["encoder_layer_indices"]
        .as_array()
        .ok_or("encoder_layer_indices missing")?
        .iter()
        .map(|v| v.as_i64().unwrap_or(-1))
        .collect();
    let wanted = resolve_layer_indices(&raw, cfg.num_layers);
    println!("layer indices {raw:?} -> {wanted:?}");

    let t = std::time::Instant::now();
    let (bpe, hidden) = encoder.forward(&features, &wanted)?;
    let bpe_v: Vec<f32> = bpe.as_slice::<f32>().to_vec();
    println!(
        "forward in {:.3}s -> bpe {:?}, {} hidden states",
        t.elapsed().as_secs_f32(),
        bpe.shape(),
        hidden.len()
    );

    let (ref_shape, ref_bpe) = load_npy_f32("/tmp/parity_bpe.npy")?;
    println!("reference bpe shape {ref_shape:?}");
    compare("bpe_logits", &flat(&bpe)?, &ref_bpe)?;

    println!("first 4 logits  rust: {:?}", &bpe_v[..4]);
    println!("first 4 logits python: {:?}", &ref_bpe[..4]);

    // ---- projector ----
    let multilayer = mlx_rs::ops::concatenate_axis(&hidden, -1)?;
    if let Ok((_, ref_multi)) = load_npy_f32("/tmp/parity_multi.npy") {
        println!("\nmultilayer -> {:?}", multilayer.shape());
        compare("multilayer (projector input)", &flat(&multilayer)?, &ref_multi)?;
    }
    let (s1, s2) = projector.debug_stages(&multilayer)?;
    if let Ok((_, r1)) = load_npy_f32("/tmp/pj_step1.npy") {
        compare("  pj step1 norms+concat", &flat(&s1)?, &r1)?;
    }
    if let Ok((_, r2)) = load_npy_f32("/tmp/pj_step2.npy") {
        compare("  pj step2 proj+gelu", &flat(&s2)?, &r2)?;
    }
    let (dh, denc, dl0) = projector.debug_windows(&s2)?;
    if let Ok((_, r)) = load_npy_f32("/tmp/pj_h.npy") {
        compare("  pj queries (h)", &flat(&dh)?, &r)?;
    }
    if let Ok((_, r)) = load_npy_f32("/tmp/pj_enc.npy") {
        compare("  pj window ctx (enc)", &flat(&denc)?, &r)?;
    }
    if let Ok((_, r)) = load_npy_f32("/tmp/pj_l0.npy") {
        compare("  pj qformer layer0", &flat(&dl0)?, &r)?;
    }
    // Weight sanity: does the tensor the loader produced match the reference?
    if let Some(w) = weights.get("projector.layers.0.q_proj.weight") {
        let v = w.as_slice::<f32>();
        let sum: f32 = v.iter().sum();
        println!("\n  q_proj.weight shape {:?} sum {sum:.6} first3 {:?}", w.shape(), &v[..3]);
    }
    if let Some(b) = weights.get("projector.layers.0.q_proj.bias") {
        let sum: f32 = b.as_slice::<f32>().iter().sum();
        println!("  q_proj.bias sum {sum:.6}");
    }
    let (dq, dattn, dpost) = projector.debug_attn(&dh, &denc)?;
    if let Ok((rs, r)) = load_npy_f32("/tmp/pj_q.npy") {
        let g = dq.as_slice::<f32>();
        println!("\n  q shape rust {:?} vs ref {:?}  (len {} vs {})", dq.shape(), rs, g.len(), r.len());
        println!("  q rust[..6]  {:?}", &g[..6]);
        println!("  q ref [..6]  {:?}", &r[..6]);
        // is it a permutation? check whether ref[0] appears early in rust
        if let Some(pos) = g.iter().position(|v| (v - r[0]).abs() < 1e-4) {
            println!("  ref[0]={:.5} found at rust index {pos}", r[0]);
        }
    }
    for (label, got, file) in [
        ("  pj q heads", &dq, "/tmp/pj_q.npy"),
        ("  pj attn out", &dattn, "/tmp/pj_attn.npy"),
        ("  pj post-attn", &dpost, "/tmp/pj_postattn.npy"),
    ] {
        if let Ok((_, r)) = load_npy_f32(file) {
            compare(label, &flat(got)?, &r)?;
        }
    }
    let audio = projector.forward(&multilayer)?;
    println!("\nprojector -> {:?}", audio.shape());
    let (_, ref_audio) = load_npy_f32("/tmp/parity_audio.npy")?;
    compare("projector", &flat(&audio)?, &ref_audio)?;

    // ---- editor ----
    let blank = config["blank_token_id"].as_i64().unwrap_or(100257);
    let min_len = config["min_edit_sequence_length"].as_i64().unwrap_or(8) as usize;
    let scale_proj = config["scale_projected_embeddings"].as_bool().unwrap_or(true);

    let ctc_ids = taurscribe_lib::granite_mlx::editor::ctc_greedy(
        &taurscribe_lib::granite_mlx::editor::argmax_rows(&bpe.index(0))?,
        blank,
    );
    let audio_len = features.shape()[1] / proj_cfg.downsample_rate;
    let audio_e = if scale_proj {
        audio.divide(Array::from_f32(ed_cfg.embedding_multiplier))?
    } else {
        audio.clone()
    };
    let audio_e = audio_e.index((.., ..audio_len));

    let slots = add_insertion_slots(&ctc_ids, blank, min_len);
    println!("ctc tokens {}, slots {}, audio_len {audio_len}", ctc_ids.len(), slots.len());
    let slot_arr = Array::from_slice(
        &slots.iter().map(|&v| v as i32).collect::<Vec<_>>(),
        &[slots.len() as i32],
    );
    let text_e = editor.embed(&slot_arr);
    let text_e = text_e.reshape(&[1, slots.len() as i32, -1])?;
    let embeds = mlx_rs::ops::concatenate_axis(&[audio_e, text_e], 1)?;
    let logits = editor.forward(&embeds, audio_len)?;
    println!("editor -> {:?}", logits.shape());
    let (_, ref_ed) = load_npy_f32("/tmp/parity_editor.npy")?;
    compare("editor_logits", &flat(&logits)?, &ref_ed)?;

    println!("\nALL STAGES PASS");
    Ok(())
}

/// `as_slice` on a transposed view exposes the raw strided buffer, so flatten
/// through a reshape first — that forces MLX to lay the data out logically.
#[cfg(target_os = "macos")]
fn flat(a: &mlx_rs::Array) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    Ok(a.reshape(&[-1])?.as_slice::<f32>().to_vec())
}

#[cfg(target_os = "macos")]
fn compare(name: &str, got: &[f32], want: &[f32]) -> Result<(), String> {
    if got.len() != want.len() {
        return Err(format!("{name}: length {} vs {}", got.len(), want.len()));
    }
    let scale = want.iter().fold(0f32, |m, v| m.max(v.abs())).max(1e-6);
    let worst = got
        .iter()
        .zip(want)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    let rel = worst / scale;
    let bad = got
        .iter()
        .zip(want)
        .filter(|(a, b)| (*a - *b).abs() > 1e-3 * scale)
        .count();
    let worst_at = got
        .iter()
        .zip(want)
        .enumerate()
        .max_by(|a, b| {
            ((a.1 .0 - a.1 .1).abs())
                .partial_cmp(&((b.1 .0 - b.1 .1).abs()))
                .unwrap()
        })
        .map(|(i, _)| i)
        .unwrap_or(0);
    let verdict = if rel < 1e-4 { "PASS" } else { "FAIL" };
    if rel >= 1e-4 {
        println!(
            "      {bad}/{} elems differ; worst at index {worst_at} (of {})",
            got.len(),
            got.len()
        );
    }
    // Report every stage rather than stopping at the first failure: knowing which
    // later stages also drift is what localises the bug.
    println!("{verdict}  {name}: worst |delta| {worst:.3e}, relative {rel:.3e}");
    Ok(())
}

/// Minimal .npy reader for the C-order float32 arrays the capture writes.
#[cfg(target_os = "macos")]
fn load_npy_f32(path: &str) -> Result<(Vec<usize>, Vec<f32>), Box<dyn std::error::Error>> {
    let raw = std::fs::read(path)?;
    if &raw[..6] != b"\x93NUMPY" {
        return Err(format!("{path}: not a .npy file").into());
    }
    let header_len = u16::from_le_bytes([raw[8], raw[9]]) as usize;
    let header = std::str::from_utf8(&raw[10..10 + header_len])?;
    if !header.contains("'<f4'") && !header.contains("'|f4'") {
        return Err(format!("{path}: expected float32, header {header}").into());
    }
    let shape_str = header
        .split("'shape':")
        .nth(1)
        .and_then(|s| s.split('(').nth(1))
        .and_then(|s| s.split(')').next())
        .ok_or("no shape in npy header")?;
    let shape: Vec<usize> = shape_str
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let body = &raw[10 + header_len..];
    let values: Vec<f32> = body
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    Ok((shape, values))
}
