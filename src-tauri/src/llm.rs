//! LLM engine for transcript grammar correction.
//! Loads from taurscribe-runtime/models/qwen_finetuned_gguf (GGUF Q4_K_M). Uses CUDA when available.

use anyhow::{Error, Result};
use candle_core::{DType, Device, IndexOp, Tensor};
use candle_core::quantized::gguf_file;
use candle_transformers::generation::LogitsProcessor;
use candle_transformers::models::quantized_qwen2::ModelWeights;
use tokenizers::Tokenizer;

const GGUF_FILENAME: &str = "model_q4_k_m.gguf";

/// Hardcoded path for the GGUF grammar model.
const GRAMMAR_LLM_PATH: &str = r"C:\Users\abdul\OneDrive\Desktop\Taurscribe\taurscribe-runtime\models\qwen_finetuned_gguf";

/// Grammar LLM model path: hardcoded path, or GRAMMAR_LLM_DIR env override, or AppData fallback.
pub fn get_grammar_llm_dir() -> Result<std::path::PathBuf, String> {
    // 0. Hardcoded path
    let hardcoded = std::path::PathBuf::from(GRAMMAR_LLM_PATH);
    if hardcoded.join(GGUF_FILENAME).exists() {
        return Ok(hardcoded);
    }
    // 1. Explicit path from env override
    if let Ok(dir) = std::env::var("GRAMMAR_LLM_DIR") {
        let path = std::path::PathBuf::from(&dir);
        if path.join(GGUF_FILENAME).exists() {
            return Ok(path);
        }
    }
    // 2. Fallback: AppData/Taurscribe/models/qwen_finetuned_gguf
    let models_dir = crate::utils::get_models_dir()?;
    Ok(models_dir.join("qwen_finetuned_gguf"))
}

/// Build a tokenizer from GGUF embedded metadata (tokenizer.ggml.tokens + tokenizer.ggml.merges).
/// Falls back to tokenizer.json in the same folder if metadata is absent.
fn build_tokenizer(
    base_path: &std::path::Path,
    content: &gguf_file::Content,
) -> Result<Tokenizer> {
    use gguf_file::Value;
    use tokenizers::models::bpe::BPE;

    // Try GGUF metadata: tokenizer.ggml.tokens + tokenizer.ggml.merges (self-contained GGUF)
    if let (Some(Value::Array(tokens_arr)), Some(Value::Array(merges_arr))) = (
        content.metadata.get("tokenizer.ggml.tokens"),
        content.metadata.get("tokenizer.ggml.merges"),
    ) {
        // Collect vocab: token string -> token id
        let vocab: tokenizers::models::bpe::Vocab = tokens_arr
            .iter()
            .enumerate()
            .filter_map(|(i, v)| match v {
                Value::String(s) => Some((s.clone(), i as u32)),
                _ => None,
            })
            .collect();
        // Collect merges: pairs of strings
        let merges: tokenizers::models::bpe::Merges = merges_arr
            .iter()
            .filter_map(|v| match v {
                Value::String(s) => {
                    let mut parts = s.splitn(2, ' ');
                    match (parts.next(), parts.next()) {
                        (Some(a), Some(b)) => Some((a.to_string(), b.to_string())),
                        _ => None,
                    }
                }
                _ => None,
            })
            .collect();
        if !vocab.is_empty() && !merges.is_empty() {
            println!(
                "[LLM] Building tokenizer from GGUF metadata ({} tokens, {} merges)",
                vocab.len(),
                merges.len()
            );
            let bpe = BPE::new(vocab, merges);
            let tokenizer = Tokenizer::new(bpe);
            return Ok(tokenizer);
        }
    }

    // Fallback: tokenizer.json in same folder as the GGUF
    let tokenizer_path = base_path.join("tokenizer.json");
    println!(
        "[LLM] No tokenizer in GGUF metadata, loading from {:?}",
        tokenizer_path
    );
    Tokenizer::from_file(&tokenizer_path).map_err(Error::msg)
}

pub struct LLMEngine {
    model: ModelWeights,
    tokenizer: Tokenizer,
    device: Device,
    eos_token_id: u32,
    eos_im_end_id: u32,
}

impl LLMEngine {
    /// Create LLM from taurscribe-runtime/models/qwen_finetuned_gguf (or AppData fallback).
    /// Uses CPU (quantized GGUF tensors are CPU-only in candle today).
    pub fn new() -> Result<Self> {
        let base_path = get_grammar_llm_dir().map_err(Error::msg)?;
        let model_path = base_path.join(GGUF_FILENAME);

        if !model_path.exists() {
            return Err(Error::msg(format!(
                "Grammar LLM model not found. Expected at: {:?}\nPlace {} in taurscribe-runtime/models/qwen_finetuned_gguf",
                model_path, GGUF_FILENAME
            )));
        }

        println!("[LLM] Loading grammar model from: {:?}", model_path);

        // Quantized GGUF tensors are CPU-only in candle
        let device = Device::Cpu;
        println!("[LLM] Using device: {:?}", device);

        let mut file = std::fs::File::open(&model_path)?;
        let content = gguf_file::Content::read(&mut file)
            .map_err(|e| Error::msg(format!("Failed to read GGUF: {}", e)))?;

        let tokenizer = build_tokenizer(&base_path, &content)?;
        let vocab = tokenizer.get_vocab(true);
        let eos_token_id = vocab.get("<|endoftext|>").copied().unwrap_or(151643);
        let eos_im_end_id = vocab.get("<|im_end|>").copied().unwrap_or(151645);

        let model = ModelWeights::from_gguf(content, &mut file, &device)
            .map_err(|e| Error::msg(format!("Failed to load quantized Qwen2: {}", e)))?;

        println!(
            "[LLM] Model loaded. EOS tokens: <|endoftext|>={}, <|im_end|>={}",
            eos_token_id, eos_im_end_id
        );

        Ok(Self {
            model,
            tokenizer,
            device,
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

        let tokens = self
            .tokenizer
            .encode(prompt, true)
            .map_err(Error::msg)?
            .get_ids()
            .to_vec();
        let prompt_tokens = tokens.len();

        let mut logits_processor = LogitsProcessor::new(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            Some(temperature),
            Some(0.95),
        );

        // Prefill: process all prompt tokens at once
        let prefill_start = std::time::Instant::now();
        let input = Tensor::new(tokens.as_slice(), &self.device)?.unsqueeze(0)?;
        let logits = self.model.forward(&input, 0)?;
        let logits = logits.squeeze(0)?.to_dtype(DType::F32)?;
        // quantized_qwen2::forward returns shape [vocab] for single-token or [seq, vocab]
        let last_logits = if logits.dims().len() == 1 {
            logits
        } else {
            logits.i(logits.dim(0)? - 1)?
        };
        let mut next_token = logits_processor.sample(&last_logits)?;
        let mut generated_tokens = vec![next_token];
        let prefill_time = prefill_start.elapsed();
        let mut pos = tokens.len();

        println!(
            "[LLM] Prefill: {} tokens in {:?}",
            prompt_tokens, prefill_time
        );
        print!("[LLM] Generating: ");
        std::io::stdout().flush().ok();

        // Decode loop: generate one token at a time
        let gen_start = std::time::Instant::now();
        for i in 0..max_gen_tokens {
            if next_token == self.eos_token_id || next_token == self.eos_im_end_id {
                println!(" [EOS at token {}]", i);
                break;
            }
            if i % 10 == 0 {
                print!(".");
                std::io::stdout().flush().ok();
            }

            let input = Tensor::new(&[next_token], &self.device)?.unsqueeze(0)?;
            let logits = self.model.forward(&input, pos)?;
            let logits = logits.squeeze(0)?.to_dtype(DType::F32)?;
            let last_logits = if logits.dims().len() == 1 {
                logits
            } else {
                logits.i(logits.dim(0)? - 1)?
            };
            next_token = logits_processor.sample(&last_logits)?;
            generated_tokens.push(next_token);
            pos += 1;
        }
        let gen_time = gen_start.elapsed();
        println!();

        let decoded = self
            .tokenizer
            .decode(&generated_tokens, true)
            .map_err(Error::msg)?;
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

    /// Format transcript for grammar correction. Uses ChatML-style prompt so the model
    /// acts only as a copy editor (no chat, no greeting, no continuation).
    pub fn format_transcript(&mut self, text: &str) -> Result<String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(String::new());
        }
        // Qwen2.5 ChatML: strict system role so output is only the corrected text
        let prompt = format!(
            r#"<|im_start|>system
You are a copy editor. Your only task is to output the corrected text. Do not greet, explain, ask questions, or add anything. Output exactly one thing: the input text with grammar and punctuation fixed. No other words.<|im_end|>
<|im_start|>user
Correct and format this transcript. Output only the corrected text, nothing else:

{}<|im_end|>
<|im_start|>assistant
"#,
            text
        );
        // Correction output is similar length to input; cap tokens so we don't waste time.
        // ~4 chars per token, add headroom. Cap at 150 for speed.
        let max_tokens = (text.len() / 4).saturating_add(48).min(150);
        let temperature = 0.3; // more deterministic, model tends to EOS sooner
        self.run_with_options(&prompt, max_tokens, temperature)
    }
}
