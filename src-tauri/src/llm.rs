//! LLM engine for transcript clean-up (FlowScribe).
//!
//! FlowScribe v3 (Qwen3.5-0.8B, F16, models/flowscribe_v3) takes tags for the
//! speech engine, clean-up level, target app and the user's dictionary.
//! n_gpu_layers=0 forces CPU; change to -1 or layer count for GPU.

use anyhow::{Error, Result};
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex, OnceLock};

pub const V3_DIR: &str = "flowscribe_v3";
pub const V3_FILENAME: &str = "flowscribe-v3-f16.gguf";

/// Must match the system prompt used in training (scripts/flowscribe_train/prepare_data.py).
const V3_SYSTEM: &str = "You are FlowScribe. Rewrite the dictation in <text> as the speaker meant it, following the tags. Output only the result.";

/// What v3 needs to know besides the transcript.
#[derive(Debug, Clone)]
pub struct FlowRequest {
    /// "whisper" | "granite" | "qwen3"
    pub engine: String,
    /// "verbatim" | "clean" | "formatted"
    pub level: String,
    /// App category, see `context::app_category_for`.
    pub app: String,
    pub vocab: Vec<String>,
    /// Text already in the document right before this dictation, if known.
    pub prev: Option<String>,
}

/// Maps the saved style setting onto a v3 clean-up level. Older tone styles
/// (Casual, Professional, ...) map to "clean".
pub fn level_for_style(style: Option<&str>) -> &'static str {
    match style.map(|s| s.to_ascii_lowercase()) {
        Some(s) if s == "verbatim" => "verbatim",
        Some(s) if s == "formatted" => "formatted",
        _ => "clean",
    }
}

pub fn v3_model_path() -> Option<std::path::PathBuf> {
    // Dev/test override: FLOWSCRIBE_V3_GGUF=/path/to/model.gguf
    if let Ok(p) = std::env::var("FLOWSCRIBE_V3_GGUF") {
        let p = std::path::PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    let path = crate::utils::get_models_dir().ok()?.join(V3_DIR).join(V3_FILENAME);
    path.exists().then_some(path)
}

/// True when the tail of `tokens` is one short sequence repeated 4+ times
/// (small models under greedy decoding can fall into "I, I, I, ..." loops).
fn is_looping(tokens: &[LlamaToken]) -> bool {
    (1..=8).any(|n| {
        let reps = 4;
        tokens.len() >= n * reps && {
            let tail = &tokens[tokens.len() - n * reps..];
            tail.chunks(n).all(|c| c == &tail[..n])
        }
    })
}

/// Output far longer than the input, or a word sequence repeated 4+ times in
/// a row, means the generation went wrong.
pub fn output_looks_broken(output: &str, input: &str) -> bool {
    if output.len() > input.len() * 8 / 5 + 40 {
        return true;
    }
    let words: Vec<&str> = output.split_whitespace().collect();
    (1..=6).any(|n| {
        words.len() >= 4 * n
            && (0..=words.len() - 4 * n).any(|i| (1..4).all(|k| words[i + k * n..i + (k + 1) * n] == words[i..i + n]))
    })
}

// ── Number guard ─────────────────────────────────────────────────────────────
// Every number FlowScribe writes must be traceable to what was said: digits in
// the transcript, a spoken number ("two thousand three hundred and forty five"),
// a time ("three thirty" -> 3, 30) or a year/code ("twenty twenty five" ->
// 2025). A 0.8B model occasionally rewrites 25% as 20% or $2,345.60 as $2,300;
// pasting the transcript is safer than a wrong amount.

fn number_word(w: &str) -> Option<(u64, bool)> {
    // (value, is_tens)
    const UNITS: [&str; 20] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
        "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen"];
    const ORD_UNITS: [&str; 20] = ["zeroth", "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth",
        "ninth", "tenth", "eleventh", "twelfth", "thirteenth", "fourteenth", "fifteenth", "sixteenth", "seventeenth",
        "eighteenth", "nineteenth"];
    const TENS: [&str; 8] = ["twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
    const ORD_TENS: [&str; 8] = ["twentieth", "thirtieth", "fortieth", "fiftieth", "sixtieth", "seventieth", "eightieth", "ninetieth"];
    if let Some(i) = UNITS.iter().position(|u| *u == w).or_else(|| ORD_UNITS.iter().position(|u| *u == w)) {
        return Some((i as u64, false));
    }
    TENS.iter().position(|t| *t == w).or_else(|| ORD_TENS.iter().position(|t| *t == w)).map(|i| (20 + 10 * i as u64, true))
}

fn scale_word(w: &str) -> Option<u64> {
    match w {
        "hundred" => Some(100),
        "thousand" => Some(1_000),
        "million" => Some(1_000_000),
        "billion" => Some(1_000_000_000),
        _ => None,
    }
}

/// Numbers in `text` in order: digit groups as written ("$5,000" -> 5000,
/// "2,345.60" -> 2345, 60) and number phrases by their standard reading.
fn ordered_numbers(text: &str) -> Vec<u64> {
    let lower = text.to_lowercase().replace('-', " ");
    let mut tokens: Vec<String> = Vec::new();
    let mut chars = lower.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            let mut t = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() || d == ',' || d == '.' {
                    t.push(d);
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push(t.trim_end_matches([',', '.']).to_string());
        } else if c.is_ascii_alphabetic() {
            let mut t = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_alphabetic() {
                    t.push(d);
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push(t);
        } else {
            chars.next();
        }
    }

    let mut values = Vec::new();
    let (mut cur, mut total, mut active) = (0u64, 0u64, false);
    let mut last_tens = false;
    let mut last_scale = false;
    let flush = |values: &mut Vec<u64>, cur: &mut u64, total: &mut u64, active: &mut bool| {
        if *active {
            values.push(*total + *cur);
        }
        *cur = 0;
        *total = 0;
        *active = false;
    };
    for (i, w) in tokens.iter().enumerate() {
        let next = tokens.get(i + 1).map(String::as_str).unwrap_or("");
        if w.starts_with(|c: char| c.is_ascii_digit()) {
            flush(&mut values, &mut cur, &mut total, &mut active);
            let cleaned = w.replace(',', "");
            let (int_part, frac) = cleaned.split_once('.').unwrap_or((&cleaned, ""));
            if let Ok(v) = int_part.parse() {
                values.push(v);
            }
            if let Ok(v) = frac.parse() {
                values.push(v);
            }
            last_tens = false;
            last_scale = false;
            continue;
        }
        if w == "a" && !active && scale_word(next).is_some() {
            cur = 1;
            active = true;
            last_tens = false;
            last_scale = false;
            continue;
        }
        if w == "and" && active && number_word(next).is_some() {
            continue;
        }
        if let (Some(s), true) = (scale_word(w), active) {
            if s == 100 {
                cur = cur.max(1) * 100;
            } else {
                total += cur.max(1) * s;
                cur = 0;
            }
            last_scale = true;
            last_tens = false;
            continue;
        }
        let Some((val, is_tens)) = number_word(w) else {
            flush(&mut values, &mut cur, &mut total, &mut active);
            last_tens = false;
            last_scale = false;
            continue;
        };
        let continues = active && ((!is_tens && val < 10 && last_tens) || last_scale);
        if !continues {
            flush(&mut values, &mut cur, &mut total, &mut active);
            active = true;
        }
        cur += val;
        last_tens = is_tens;
        last_scale = false;
    }
    flush(&mut values, &mut cur, &mut total, &mut active);
    values
}

fn allowed_numbers(input: &str) -> std::collections::HashSet<String> {
    let seq: Vec<String> = ordered_numbers(input).iter().map(u64::to_string).collect();
    let mut allowed: std::collections::HashSet<String> = seq.iter().cloned().collect();
    allowed.insert("0".into()); // "4:00" from "four"
    for n in 2..=4 {
        for w in seq.windows(n) {
            allowed.insert(w.concat()); // years, codes, times written as one number
        }
    }
    allowed
}

/// True when every number in `output` can be traced to `input`.
pub fn numbers_supported(output: &str, input: &str) -> bool {
    let allowed = allowed_numbers(input);
    let mut rest = output;
    while let Some(start) = rest.find(|c: char| c.is_ascii_digit()) {
        let tail = &rest[start..];
        let end = tail.find(|c: char| !(c.is_ascii_digit() || c == ',' || c == '.')).unwrap_or(tail.len());
        let token = tail[..end].trim_end_matches([',', '.']).replace(',', "");
        rest = &tail[end.max(1)..];
        let (int_part, frac) = token.split_once('.').unwrap_or((&token, ""));
        let int_norm = int_part.trim_start_matches('0');
        let int_norm = if int_norm.is_empty() { "0" } else { int_norm };
        if !allowed.contains(int_norm) {
            return false;
        }
        if !frac.is_empty() && !allowed.contains(frac) && !allowed.contains(frac.trim_start_matches('0')) {
            return false;
        }
    }
    true
}

/// Builds the v3 prompt exactly as the training data renders it (Qwen3.5 chat
/// template with an empty think block).
pub fn build_v3_prompt(text: &str, req: &FlowRequest) -> String {
    let clean = |s: &str| s.replace(['<', '>'], "").replace('\n', " ");
    let mut tags = format!("<engine={}> <level={}> <app={}>", req.engine, req.level, req.app);
    let vocab: Vec<String> = req.vocab.iter().map(|v| clean(v)).filter(|v| !v.trim().is_empty()).take(40).collect();
    if !vocab.is_empty() {
        tags.push_str(&format!(" <vocab={}>", vocab.join("; ")));
    }
    let mut user = tags;
    if let Some(prev) = req.prev.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        user.push_str(&format!("\n<prev>{}</prev>", clean(prev)));
    }
    user.push_str(&format!("\n<text>{}</text>", text.trim()));
    format!(
        "<|im_start|>system\n{V3_SYSTEM}<|im_end|>\n<|im_start|>user\n{user}<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n"
    )
}
const GRAMMAR_CONTEXT_TOKENS: u32 = 2048;

/// Global backend instance (initialized once)
static BACKEND: OnceLock<Arc<LlamaBackend>> = OnceLock::new();

// Internal structure that holds model and context together
struct ModelContext {
    model: LlamaModel,
    context: llama_cpp_2::context::LlamaContext<'static>,
}

unsafe impl Send for ModelContext {}
unsafe impl Sync for ModelContext {}

pub struct LLMEngine {
    #[allow(dead_code)] // kept alive so backend outlives model/context
    backend: Arc<LlamaBackend>,
    model_context: Mutex<ModelContext>,
    eos_token_id: LlamaToken,
    eos_im_end_id: LlamaToken,
}

impl LLMEngine {
    /// Load FlowScribe v3 from models/flowscribe_v3.
    /// Uses CUDA when available (via llama-cpp-2 features) and use_gpu is true.
    pub fn new(use_gpu: bool) -> Result<Self> {
        let model_path = v3_model_path().ok_or_else(|| {
            Error::msg("FlowScribe model not found. Download FlowScribe V3 from the Models tab.")
        })?;

        println!("[LLM] Loading FlowScribe v3 from: {:?}", model_path);

        // Initialize backend (once, shared across instances)
        let backend = BACKEND.get_or_init(|| {
            Arc::new(LlamaBackend::init().expect("Failed to initialize llama backend"))
        });
        let backend = Arc::clone(backend);

        // Load model: n_gpu_layers=99 for GPU, 0 for CPU
        // On macOS, we force CPU only (0 layers) per user request, ignoring the use_gpu flag's "true" intent for layers.
        let requested_layers = if use_gpu {
            #[cfg(target_os = "macos")]
            {
                println!("[LLM] macOS detected: Forcing CPU only (0 layers) as requested.");
                0
            }
            #[cfg(not(target_os = "macos"))]
            99
        } else {
            0
        };
        println!(
            "[LLM] Wrapper backend config: use_gpu={}, layers={}",
            use_gpu, requested_layers
        );

        let model_params = LlamaModelParams::default().with_n_gpu_layers(requested_layers);

        let (model, loaded_layers) =
            match LlamaModel::load_from_file(&backend, &model_path, &model_params) {
                Ok(m) => (m, requested_layers),
                Err(e) => {
                    if use_gpu {
                        eprintln!("[LLM] GPU load failed: {}. Falling back to CPU only.", e);
                        let cpu_params = LlamaModelParams::default().with_n_gpu_layers(0);
                        let m = LlamaModel::load_from_file(&backend, &model_path, &cpu_params)
                            .map_err(|e2| {
                                Error::msg(format!(
                                    "Failed to load GGUF model (CPU fallback also failed): {}",
                                    e2
                                ))
                            })?;
                        (m, 0)
                    } else {
                        return Err(Error::msg(format!("Failed to load GGUF model: {}", e)));
                    }
                }
            };

        println!(
            "[LLM] Model loaded successfully. GPU Layers: {}",
            loaded_layers
        );

        // Get EOS tokens
        let eos_token_id = model.token_eos();

        // Try to find <|im_end|> token by searching through tokens
        let eos_im_end_id = model
            .str_to_token("<|im_end|>", AddBos::Never)
            .ok()
            .and_then(|tokens| tokens.first().copied())
            .unwrap_or_else(|| {
                // Fallback: try to find it via token search
                model
                    .tokens(true)
                    .find_map(|(token, result)| {
                        result
                            .ok()
                            .and_then(|s| if s == "<|im_end|>" { Some(token) } else { None })
                    })
                    .unwrap_or(eos_token_id)
            });

        println!(
            "[LLM] EOS tokens: <|endoftext|>={:?}, <|im_end|>={:?}",
            eos_token_id, eos_im_end_id
        );

        // Grammar correction prompts are short; cap KV cache instead of allocating
        // the model's full train context.
        let context_params = llama_cpp_2::context::params::LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(GRAMMAR_CONTEXT_TOKENS));
        let context = model
            .new_context(&backend, context_params)
            .map_err(|e| Error::msg(format!("Failed to create context: {}", e)))?;

        // Transmute lifetime to 'static - safe because model lives as long as the struct
        let context = unsafe { std::mem::transmute(context) };
        let model_context = ModelContext { model, context };

        Ok(Self {
            backend,
            model_context: Mutex::new(model_context),
            eos_token_id,
            eos_im_end_id,
        })
    }

    /// Run generation. `max_gen_tokens` caps output length; lower = faster for short tasks.
    /// `temperature` 0.0–1.0; lower = more deterministic, often stops sooner (e.g. 0.3 for correction).
    pub fn run_with_options(
        &mut self,
        prompt: &str,
        max_gen_tokens: usize,
        temperature: f64,
    ) -> Result<String> {
        use std::io::Write;

        let total_start = std::time::Instant::now();

        let mut mc = self.model_context.lock().unwrap();

        // CRITICAL: Clear KV cache to ensure fresh context for every request
        // This prevents "inconsistent sequence positions" errors on subsequent runs.
        mc.context.clear_kv_cache();

        // Encode prompt using model's built-in tokenizer
        let prompt_tokens = mc
            .model
            .str_to_token(prompt, AddBos::Never)
            .map_err(|e| Error::msg(format!("Failed to tokenize prompt: {}", e)))?;
        let prompt_tokens_len = prompt_tokens.len();

        println!("[LLM] Prompt tokens: {}", prompt_tokens_len);
        let max_context_tokens = GRAMMAR_CONTEXT_TOKENS as usize;
        if prompt_tokens_len + max_gen_tokens + 8 > max_context_tokens {
            return Err(Error::msg(format!(
                "Grammar prompt too long for {GRAMMAR_CONTEXT_TOKENS}-token context: prompt={prompt_tokens_len}, max_gen={max_gen_tokens}"
            )));
        }

        // Create sampler chain: temperature -> top_p -> greedy
        let mut sampler = if temperature <= 0.0 {
            LlamaSampler::greedy()
        } else {
            LlamaSampler::chain_simple([
                LlamaSampler::temp(temperature as f32),
                LlamaSampler::top_p(0.95, 1),
                LlamaSampler::greedy(),
            ])
        };

        // UTF-8 decoder for token_to_piece
        let mut decoder = encoding_rs::UTF_8.new_decoder();

        // Prefill: process all prompt tokens at once
        let prefill_start = std::time::Instant::now();
        let mut batch = LlamaBatch::new(prompt_tokens_len.max(512), 1);

        // Add all prompt tokens to batch (pos is i32)
        let last_index = prompt_tokens_len as i32 - 1;
        for (i, &token) in (0_i32..).zip(prompt_tokens.iter()) {
            batch
                .add(token, i, &[0], i == last_index)
                .map_err(|e| Error::msg(format!("Failed to add token to batch: {:?}", e)))?;
        }

        // Decode the prompt
        mc.context
            .decode(&mut batch)
            .map_err(|e| Error::msg(format!("Failed to decode prompt: {}", e)))?;

        // Sample first token
        let mut next_token = sampler.sample(&mc.context, batch.n_tokens() - 1);
        sampler.accept(next_token);

        let mut generated_tokens = vec![next_token];
        let prefill_time = prefill_start.elapsed();
        let mut n_cur = batch.n_tokens();

        println!(
            "[LLM] Prefill: {} tokens in {:?}",
            prompt_tokens_len, prefill_time
        );
        print!("[LLM] Generating: ");
        std::io::stdout().flush().ok();

        // Decode loop: generate one token at a time
        let gen_start = std::time::Instant::now();
        for i in 0..max_gen_tokens {
            if is_looping(&generated_tokens) {
                println!(" [stopped: repetition loop at token {}]", i);
                break;
            }
            if next_token == self.eos_token_id
                || next_token == self.eos_im_end_id
                || mc.model.is_eog_token(next_token)
            {
                println!(" [EOS at token {}]", i);
                break;
            }
            if i % 10 == 0 {
                print!(".");
                std::io::stdout().flush().ok();
            }

            // Create batch with single token
            batch.clear();
            batch
                .add(next_token, n_cur, &[0], true)
                .map_err(|e| Error::msg(format!("Failed to add token to batch: {:?}", e)))?;

            // Decode
            mc.context
                .decode(&mut batch)
                .map_err(|e| Error::msg(format!("Failed to decode: {}", e)))?;

            // Sample next token
            next_token = sampler.sample(&mc.context, batch.n_tokens() - 1);
            sampler.accept(next_token);

            generated_tokens.push(next_token);
            n_cur += 1;
        }
        let gen_time = gen_start.elapsed();
        println!();

        // Decode tokens back to string using token_to_piece (non-deprecated API)
        let mut decoded = String::new();
        for &tok in &generated_tokens {
            match mc.model.token_to_piece(tok, &mut decoder, true, None) {
                Ok(piece) => decoded.push_str(&piece),
                Err(_) => {} // skip undecodable tokens
            }
        }

        let cleaned = decoded
            .replace("<|endoftext|>", "")
            .replace("<|im_end|>", "")
            .trim()
            .to_string();

        let gen_tokens = generated_tokens.len();
        let tokens_per_sec = if gen_time.as_secs_f64() > 0.0 {
            gen_tokens as f64 / gen_time.as_secs_f64()
        } else {
            0.0
        };
        println!(
            "[LLM] Done: {} tokens in {:.0}ms ({:.1} tok/s) | Total: {:.0}ms",
            gen_tokens,
            gen_time.as_millis(),
            tokens_per_sec,
            total_start.elapsed().as_millis()
        );

        Ok(cleaned)
    }

    /// Run with default 512 max tokens and 0.7 temperature (for general inference).
    pub fn run(&mut self, prompt: &str) -> Result<String> {
        self.run_with_options(prompt, 512, 0.7)
    }

    /// Clean up a transcript with the tagged prompt from `req`.
    pub fn clean_transcript(&mut self, text: &str, req: &FlowRequest) -> Result<String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(String::new());
        }
        let prompt = build_v3_prompt(text, req);
        // Output is close to the input's length (~4 chars per token); formatted
        // email adds a few line breaks.
        let max_tokens = text.len() / 3 + 48;
        let output = self.run_with_options(&prompt, max_tokens, 0.0)?;
        if output_looks_broken(&output, text) {
            // Never paste a runaway generation; the plain transcript is safer.
            println!("[LLM] v3 output rejected (loop or too long): {:?}; using the transcript as-is", output.chars().take(120).collect::<String>());
            return Ok(text.to_string());
        }
        if !numbers_supported(&output, text) {
            println!("[LLM] v3 output rejected (a number not in the transcript): {output:?}; using the transcript as-is");
            return Ok(text.to_string());
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rendered by Qwen3.5's own chat template (enable_thinking=False) from the
    /// training code's make_prompt; the app must send exactly this.
    #[test]
    fn v3_prompt_matches_training_template() {
        let req = FlowRequest {
            engine: "granite".into(),
            level: "formatted".into(),
            app: "email".into(),
            vocab: vec!["Tauri".into(), "Jane".into()],
            prev: Some("Hi team.".into()),
        };
        let expected = "<|im_start|>system\nYou are FlowScribe. Rewrite the dictation in <text> as the speaker meant it, following the tags. Output only the result.<|im_end|>\n<|im_start|>user\n<engine=granite> <level=formatted> <app=email> <vocab=Tauri; Jane>\n<prev>Hi team.</prev>\n<text>um lets ship the tory build friday</text><|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n";
        assert_eq!(build_v3_prompt("um lets ship the tory build friday", &req), expected);
    }

    #[test]
    fn broken_outputs_are_detected() {
        assert!(output_looks_broken("Um, I, I, I, I, I, I", "um i think"));
        assert!(output_looks_broken("the car is making a car is making a car is making a car is making a", "the car is making a noise"));
        assert!(!output_looks_broken("Send the invoice to finance.", "um send the the invoice to finance"));
        assert!(!output_looks_broken("I I think so.", "i i think so"));
        let t = |v: &[i32]| v.iter().map(|&x| LlamaToken::new(x)).collect::<Vec<_>>();
        assert!(is_looping(&t(&[5, 9, 9, 9, 9])));
        assert!(is_looping(&t(&[1, 2, 3, 1, 2, 3, 1, 2, 3, 1, 2, 3])));
        assert!(!is_looping(&t(&[1, 2, 3, 4, 5, 6])));
    }

    #[test]
    fn number_guard_matches_prototype() {
        let cases = [
            ("25% reduction", "twenty five percent reduction", true),
            ("20% reduction", "twenty five percent reduction", false),
            ("$2,345.60 by the 28th", "two thousand three hundred and forty five dollars and sixty cents by the twenty eighth", true),
            ("$2,300 by the 28th", "two thousand three hundred and forty five dollars and sixty cents by the twenty eighth", false),
            ("October 2025", "october twenty twenty five", true),
            ("October 2020", "october twenty twenty five", false),
            ("at 3:30", "at three thirty", true),
            ("api-7f92b", "api dash seven f nine two b", true),
            ("5-minute warm-up, 3 sets of 12", "five minute warm up three sets of twelve", true),
            ("1,500 dollars", "fifteen hundred dollars", true),
            ("a 100 people", "a hundred people", true),
            ("from $5,000 to $7,000", "from $5,000 to $7,000", true),
            ("Friday at 4:00", "friday at four", true),
            ("Send the invoice to finance.", "um send the invoice to finance", true),
        ];
        for (out, inp, want) in cases {
            assert_eq!(numbers_supported(out, inp), want, "{out} <- {inp}");
        }
    }

    #[test]
    fn legacy_styles_map_to_clean() {
        assert_eq!(level_for_style(Some("Verbatim")), "verbatim");
        assert_eq!(level_for_style(Some("Formatted")), "formatted");
        assert_eq!(level_for_style(Some("Casual")), "clean");
        assert_eq!(level_for_style(None), "clean");
    }
}
