//! LLM engine for transcript grammar correction.
//! Loads from taurscribe-runtime/models/qwen_finetuned_gguf (GGUF Q4_K_M).
//! n_gpu_layers=0 forces CPU; change to -1 or layer count for GPU.

use anyhow::{Error, Result};
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;
use std::sync::{Arc, Mutex, OnceLock};

const GGUF_FILENAME: &str = "model_q4_k_m.gguf";

/// Hardcoded path for the GGUF grammar model.
const GRAMMAR_LLM_PATH: &str =
    r"C:\Users\abdul\OneDrive\Desktop\Taurscribe\taurscribe-runtime\models\qwen_finetuned_gguf";

/// Global backend instance (initialized once)
static BACKEND: OnceLock<Arc<LlamaBackend>> = OnceLock::new();

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
    /// Create LLM from taurscribe-runtime/models/qwen_finetuned_gguf (or AppData fallback).
    /// Uses CUDA when available (via llama-cpp-2 features) and use_gpu is true.
    pub fn new(use_gpu: bool) -> Result<Self> {
        let base_path = get_grammar_llm_dir().map_err(Error::msg)?;
        let model_path = base_path.join(GGUF_FILENAME);

        if !model_path.exists() {
            return Err(Error::msg(format!(
                "Grammar LLM model not found. Expected at: {:?}\nPlace {} in taurscribe-runtime/models/qwen_finetuned_gguf",
                model_path, GGUF_FILENAME
            )));
        }

        println!("[LLM] Loading grammar model from: {:?}", model_path);

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

        // Create context with default params
        let context_params = llama_cpp_2::context::params::LlamaContextParams::default();
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

        // Create sampler chain: temperature -> top_p -> greedy
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::temp(temperature as f32),
            LlamaSampler::top_p(0.95, 1),
            LlamaSampler::greedy(),
        ]);

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

    /// Format transcript for grammar correction. Uses ChatML-style prompt so the model
    /// acts only as a copy editor (no chat, no greeting, no continuation).
    /// Format transcript with a specific style.
    pub fn format_transcript(&mut self, text: &str, style: Option<&str>) -> Result<String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(String::new());
        }

        // Use selected style or default to 'Auto'
        let style_name = style.unwrap_or("Auto");

        // Qwen2.5 ChatML: Wispr Flow persona
        let prompt = format!(
            r#"<|im_start|>system
You are Wispr Flow, an AI that transcribes and polishes speech.
Instruction: Transcribe and format this with style: {}<|im_end|>
<|im_start|>user
{}<|im_end|>
<|im_start|>assistant
"#,
            style_name, text
        );
        // Correction output is usually close to input length, but we give it room to breathe.
        // We estimate token count as slightly more than chars/4 and add a safety buffer.
        // We do NOT cap this arbitrarily, as that would truncate long inputs.
        let max_tokens = (text.len() / 2) + 128;
        let temperature = 0.3; // more deterministic, model tends to EOS sooner
        self.run_with_options(&prompt, max_tokens, temperature)
    }
}
