use anyhow::{Error, Result};
use candle_core::quantized::gguf_file;
use candle_core::{Device, Tensor};
use candle_transformers::generation::LogitsProcessor;
use candle_transformers::models::quantized_gemma3 as model;
use std::path::PathBuf;
use tokenizers::Tokenizer;

pub struct LLMEngine {
    model: model::ModelWeights,
    tokenizer: Tokenizer,
    device: Device,
    eos_token_id: u32,
    newline_token_id: u32,
}

impl LLMEngine {
    pub fn new() -> Result<Self> {
        // Hardcoded path to the specific model requested
        let base_path = PathBuf::from(
            r"c:\Users\abdul\OneDrive\Desktop\Taurscribe\taurscribe-runtime\models\GRMR-V3-G1B-GGUF",
        );
        let model_path = base_path.join("GRMR-V3-G1B-Q4_K_M.gguf");
        let tokenizer_path = base_path.join("tokenizer.json");

        if !model_path.exists() {
            return Err(Error::msg(format!(
                "Model file not found at: {:?}",
                model_path
            )));
        }

        println!("[LLM] Loading model from: {:?}", model_path);

        // Force CPU for now (user requested CPU-only)
        let device = Device::Cpu;
        println!("[LLM] Using device: {:?}", device);

        // Load Tokenizer
        let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(Error::msg)?;

        // Get special token IDs
        let vocab = tokenizer.get_vocab(true);
        
        // For Gemma 3, EOS is typically token ID 1 or <end_of_turn> (107)
        let eos_token_id = vocab.get("<eos>").copied()
            .or_else(|| vocab.get("<end_of_turn>").copied())
            .unwrap_or(1);
        
        // Get newline token for stopping (grammar output should be single line)
        let newline_token_id = vocab.get("\n").copied().unwrap_or(108);
        
        println!("[LLM] EOS token ID: {}, Newline token ID: {}", eos_token_id, newline_token_id);

        // Load Model (GGUF)
        let mut file = std::fs::File::open(&model_path)?;
        let content = gguf_file::Content::read(&mut file)
            .map_err(|e| Error::msg(format!("GGUF Read Error: {}", e)))?;

        let model = model::ModelWeights::from_gguf(content, &mut file, &device)?;

        Ok(Self {
            model,
            tokenizer,
            device,
            eos_token_id,
            newline_token_id,
        })
    }

    pub fn run(&mut self, prompt: &str) -> Result<String> {
        use std::time::Instant;
        let total_start = Instant::now();

        // GRMR-V3-G1B chat template format:
        // <bos>text\n{content}\ncorrected\n
        // The tokenizer will add BOS when we encode with add_special_tokens=true
        // So we just need: "text\n{content}\ncorrected\n"
        let formatted_prompt = format!("text\n{}\ncorrected\n", prompt.trim());

        // Encode with special tokens (adds BOS automatically)
        let tokens = self
            .tokenizer
            .encode(formatted_prompt, true)
            .map_err(Error::msg)?
            .get_ids()
            .to_vec();
        
        let prompt_tokens = tokens.len();

        let mut generated_tokens = Vec::new();

        // Create a fresh LogitsProcessor for each run (recommended settings from model card)
        // temperature=0.7, top_p=0.95
        let mut logits_processor = LogitsProcessor::new(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            Some(0.7),
            Some(0.95),
        );

        // Initial prompt processing (prefill)
        let prefill_start = Instant::now();
        let input = Tensor::new(tokens.as_slice(), &self.device)?.unsqueeze(0)?;
        let logits = self.model.forward(&input, 0)?;
        let logits = logits.squeeze(0)?;

        let last_logits = if logits.rank() == 1 {
            logits
        } else {
            let (seq_len, _) = logits.dims2()?;
            logits.get(seq_len - 1)?
        };

        let mut next_token = logits_processor.sample(&last_logits)?;
        generated_tokens.push(next_token);
        let prefill_time = prefill_start.elapsed();

        // Generation loop - grammar correction should be short
        let max_gen_tokens = 512; // Allow enough for longer texts
        let mut pos = tokens.len();

        println!("[LLM] Prefill: {} tokens in {:?}", prompt_tokens, prefill_time);
        print!("[LLM] Generating: ");
        use std::io::Write;
        std::io::stdout().flush().ok();
        
        let gen_start = Instant::now();

        for i in 0..max_gen_tokens {
            // Stop conditions: EOS token or newline (output should be single line)
            if next_token == self.eos_token_id {
                println!(" [EOS at token {}]", i);
                break;
            }
            if next_token == self.newline_token_id {
                println!(" [NEWLINE at token {}]", i);
                break;
            }

            // Progress indicator (every 10 tokens)
            if i % 10 == 0 {
                print!(".");
                std::io::stdout().flush().ok();
            }

            let input = Tensor::new(&[next_token], &self.device)?.unsqueeze(0)?;
            let logits = self.model.forward(&input, pos)?;
            let logits = logits.squeeze(0)?;

            let last_logits = if logits.rank() == 1 {
                logits
            } else {
                logits.get(0)?
            };

            next_token = logits_processor.sample(&last_logits)?;
            generated_tokens.push(next_token);
            pos += 1;
        }
        let gen_time = gen_start.elapsed();
        println!();

        // Decode generated tokens (skip special tokens)
        let decoded = self
            .tokenizer
            .decode(&generated_tokens, true)
            .map_err(Error::msg)?;

        // Clean up the output
        let cleaned = decoded
            .replace("<eos>", "")
            .replace("<end_of_turn>", "")
            .trim()
            .to_string();

        // Performance summary
        let total_time = total_start.elapsed();
        let gen_tokens = generated_tokens.len();
        let tokens_per_sec = if gen_time.as_secs_f64() > 0.0 {
            gen_tokens as f64 / gen_time.as_secs_f64()
        } else {
            0.0
        };
        
        println!(
            "[LLM] Done: {} tokens generated in {:.0}ms ({:.1} tok/s) | Total: {:.0}ms",
            gen_tokens,
            gen_time.as_millis(),
            tokens_per_sec,
            total_time.as_millis()
        );

        Ok(cleaned)
    }
}
