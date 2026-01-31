use anyhow::{Error, Result};
use candle_core::quantized::gguf_file;
use candle_core::{Device, Tensor};
use candle_transformers::generation::LogitsProcessor;
use candle_transformers::models::quantized_gemma3 as model; 
// NOTE: Using quantized_gemma2 as it is the stable architecture for Gemma models in Candle 0.9.2.
// If actual Gemma 3 architecture differs significantly, this might need adjustment, 
// but typically they share the same base or are backward compatible.
use std::path::PathBuf;
use tokenizers::Tokenizer;

pub struct LLMEngine {
    model: model::ModelWeights,
    tokenizer: Tokenizer,
    device: Device,
    logits_processor: LogitsProcessor,
}

impl LLMEngine {
    pub fn new() -> Result<Self> {
        // Hardcoded path to the specific model requested
        let base_path = PathBuf::from(r"c:\Users\abdul\OneDrive\Desktop\Taurscribe\taurscribe-runtime\models\GRMR-V3-G1B-GGUF");
        let model_path = base_path.join("GRMR-V3-G1B-Q2_K.gguf");
        // Note: Gemma models usually use a specific tokenizer, ensure tokenizer.json is correct for Gemma
        let tokenizer_path = base_path.join("tokenizer.json");

        if !model_path.exists() {
            return Err(Error::msg(format!(
                "Model file not found at: {:?}",
                model_path
            )));
        }

        println!("[LLM] Loading model from: {:?}", model_path);

        // 1. Select Device (Try CUDA, fallback to CPU)
        let device = Device::new_cuda(0).unwrap_or(Device::Cpu);
        println!("[LLM] Using device: {:?}", device);

        // 2. Load Tokenizer
        let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(Error::msg)?;

        // 3. Load Model (GGUF) - Use Gemma Architecture
        let mut file = std::fs::File::open(&model_path)?;
        let content = gguf_file::Content::read(&mut file)
            .map_err(|e| Error::msg(format!("GGUF Read Error: {}", e)))?;

        // Initialize Gemma weights
        let model = model::ModelWeights::from_gguf(content, &mut file, &device)?;

        // 4. Initialize LogitsProcessor (Sampler)
        // Recommended: Temperature = 0.7, Top-P = 0.95
        let logits_processor = LogitsProcessor::new(1337, Some(0.7), Some(0.95));

        Ok(Self {
            model,
            tokenizer,
            device,
            logits_processor,
        })
    }

    pub fn run(&mut self, prompt: &str) -> Result<String> {
        // Apply the model's specific grammar correction template
        // Adapting to user prior request: <bos>text\n{}\ncorrected\n
        let formatted_prompt = format!("<bos>text\n{}\ncorrected\n", prompt.trim());

        let tokens = self
            .tokenizer
            .encode(formatted_prompt, true)
            .map_err(Error::msg)?
            .get_ids()
            .to_vec();

        let input = Tensor::new(tokens.as_slice(), &self.device)?;

        // Gemma2 ModelWeights expects tokens and the starting position
        let logits = self.model.forward(&input, 0)?;

        // Logits returned are [seq_len, vocab_size]
        // We take the last token's logits to predict the next token
        let (_seq_len, _vocab_size) = logits.dims2()?;
        let last_logits = logits.get(_seq_len - 1)?;

        // Sample using the LogitsProcessor (Temperature + Top-P)
        let next_token = self.logits_processor.sample(&last_logits)?;

        let decoded = self.tokenizer.decode(&[next_token], true).map_err(Error::msg)?;

        Ok(decoded)
    }
}
