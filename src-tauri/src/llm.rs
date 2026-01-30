use anyhow::{anyhow, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::generation::LogitsProcessor;
use candle_transformers::models::llama::{Cache, Config, Llama, LlamaEosToks};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokenizers::Tokenizer;

#[derive(Debug, Clone, Serialize)]
pub enum GpuBackend {
    Cuda,
    Cpu,
}

/// SmolLM2 config from Hugging Face (maps to Llama config)
#[derive(Debug, Clone, Deserialize)]
struct SmolLM2Config {
    hidden_size: usize,
    intermediate_size: usize,
    vocab_size: usize,
    num_hidden_layers: usize,
    num_attention_heads: usize,
    num_key_value_heads: Option<usize>,
    rms_norm_eps: f64,
    #[serde(default = "default_rope_theta")]
    rope_theta: f32,
    #[serde(default)]
    use_flash_attn: bool,
    #[serde(default = "default_bos")]
    bos_token_id: u32,
    #[serde(default = "default_eos")]
    eos_token_id: u32,
    #[serde(default = "default_max_pos")]
    max_position_embeddings: usize,
    #[serde(default = "default_tie_word")]
    tie_word_embeddings: bool,
}

fn default_rope_theta() -> f32 {
    100000.0
}

fn default_bos() -> u32 {
    0
}

fn default_eos() -> u32 {
    0
}

fn default_max_pos() -> usize {
    8192
}

fn default_tie_word() -> bool {
    true
}

impl From<SmolLM2Config> for Config {
    fn from(cfg: SmolLM2Config) -> Self {
        Config {
            hidden_size: cfg.hidden_size,
            intermediate_size: cfg.intermediate_size,
            vocab_size: cfg.vocab_size,
            num_hidden_layers: cfg.num_hidden_layers,
            num_attention_heads: cfg.num_attention_heads,
            num_key_value_heads: cfg.num_key_value_heads.unwrap_or(cfg.num_attention_heads),
            rms_norm_eps: cfg.rms_norm_eps,
            rope_theta: cfg.rope_theta,
            use_flash_attn: cfg.use_flash_attn,
            bos_token_id: Some(cfg.bos_token_id),
            eos_token_id: Some(LlamaEosToks::Single(cfg.eos_token_id)),
            max_position_embeddings: cfg.max_position_embeddings,
            rope_scaling: None,
            tie_word_embeddings: cfg.tie_word_embeddings,
        }
    }
}

pub struct LlmManager {
    model: Option<Llama>,
    tokenizer: Option<Tokenizer>,
    cache: Option<Cache>,
    device: Device,
    backend: GpuBackend,
    model_id: Option<String>,
    config: Option<Config>,
}

impl LlmManager {
    pub fn new() -> Self {
        Self {
            model: None,
            tokenizer: None,
            cache: None,
            device: Device::Cpu,
            backend: GpuBackend::Cpu,
            model_id: None,
            config: None,
        }
    }

    fn get_models_dir() -> Result<PathBuf> {
        let possible_paths = [
            "taurscribe-runtime/models/llm",
            "../taurscribe-runtime/models/llm",
            "../../taurscribe-runtime/models/llm",
        ];

        for path in possible_paths {
            if let Ok(canonical) = std::fs::canonicalize(path) {
                if canonical.is_dir() {
                    return Ok(canonical);
                }
            }
        }

        Err(anyhow!("Could not find LLM models directory"))
    }

    pub fn initialize(&mut self) -> Result<String> {
        let models_dir = Self::get_models_dir()?;

        // Platform-specific CUDA detection
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        {
            if let Ok(device) = Device::new_cuda(0) {
                println!("[LLM] CUDA device available");
                self.device = device;
                self.backend = GpuBackend::Cuda;
            } else {
                println!("[LLM] CUDA not available, using CPU");
                self.device = Device::Cpu;
                self.backend = GpuBackend::Cpu;
            }
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            if let Ok(device) = Device::new_cuda(0) {
                println!("[LLM] CUDA device available");
                self.device = device;
                self.backend = GpuBackend::Cuda;
            } else {
                println!("[LLM] CUDA not available, using CPU");
                self.device = Device::Cpu;
                self.backend = GpuBackend::Cpu;
            }
        }
        #[cfg(not(any(
            all(target_os = "windows", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "x86_64")
        )))]
        {
            self.device = Device::Cpu;
            self.backend = GpuBackend::Cpu;
        }

        let config_path = models_dir.join("config.json");
        let tokenizer_path = models_dir.join("tokenizer.json");
        let weights_path = models_dir.join("model.safetensors");

        if !config_path.exists() || !tokenizer_path.exists() || !weights_path.exists() {
            return Err(anyhow!("Missing model files in {:?}", models_dir));
        }

        // Load and convert config
        let config_str = std::fs::read_to_string(&config_path)?;
        let smol_config: SmolLM2Config = serde_json::from_str(&config_str)?;
        let config: Config = smol_config.into();

        // Load tokenizer
        let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|e| anyhow!("{}", e))?;

        // Load weights
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path], DType::F32, &self.device)?
        };

        let model = Llama::load(vb, &config)?;
        let cache = Cache::new(true, DType::F32, &config, &self.device)?;

        self.model = Some(model);
        self.tokenizer = Some(tokenizer);
        self.cache = Some(cache);
        self.config = Some(config);
        self.model_id = Some("smollm2-135m".to_string());

        Ok(format!("SmolLM2 loaded on {:?}", self.backend))
    }

    pub fn get_backend_info(&self) -> String {
        format!("{:?}", self.backend)
    }

    pub fn generate_correction(&mut self, text: &str) -> Result<String> {
        let model = self.model.as_ref().ok_or_else(|| anyhow!("Model not initialized"))?;
        let tokenizer = self.tokenizer.as_ref().ok_or_else(|| anyhow!("Tokenizer not initialized"))?;
        let cache = self.cache.as_mut().ok_or_else(|| anyhow!("Cache not initialized"))?;

        // Simple prompt for grammar correction
        let prompt = format!(
            "<|im_start|>system\nFix grammar errors. Output only the corrected text.<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
            text
        );

        let encoding = tokenizer.encode(prompt, true).map_err(|e| anyhow!("{}", e))?;
        let mut tokens: Vec<u32> = encoding.get_ids().to_vec();

        let mut logits_processor = LogitsProcessor::new(42, Some(0.3), None);
        let mut result_tokens: Vec<u32> = Vec::new();

        let eos_id = tokenizer.token_to_id("<|im_end|>").unwrap_or(2);

        // Generate up to 100 tokens
        for i in 0..100 {
            let context_size = if i > 0 { 1 } else { tokens.len() };
            let start_pos = tokens.len().saturating_sub(context_size);
            let input = Tensor::new(&tokens[start_pos..], &self.device)?.unsqueeze(0)?;

            let logits = model.forward(&input, start_pos, cache)?;
            let logits = logits.squeeze(0)?;
            let logits = logits.get(logits.dim(0)? - 1)?;

            let next_token = logits_processor.sample(&logits)?;
            tokens.push(next_token);
            result_tokens.push(next_token);

            // Check for end token
            if next_token == eos_id {
                break;
            }
        }

        // Decode result tokens
        let decoded = tokenizer.decode(&result_tokens, true).map_err(|e| anyhow!("{}", e))?;

        Ok(decoded.trim().to_string())
    }
}
