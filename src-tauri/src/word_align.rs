//! Word timestamps for decoders that do not return them (Granite Speech 5,
//! Qwen3-ASR), by forced alignment against a small CTC acoustic model.
//!
//! The transcribe.cpp runtime gives these two models only a single span for a
//! whole clip, so imported meetings could not assign words to speakers and
//! re-decoded each speaker's audio instead. Here the transcript the decoder
//! already produced is aligned to the audio with wav2vec2-base-960h (Apache-2.0,
//! int8 ONNX, 95 MB): the model's per-frame character probabilities are aligned
//! to the known text with a CTC Viterbi pass, giving each word a start and end
//! at 20 ms resolution. The text is never changed, only timed.
//!
//! Alignment is best effort. When the model is missing, the text cannot be laid
//! onto the audio (for example because the decoder dropped words), or the
//! result is implausible, `None` is returned and the caller keeps its existing
//! per-speaker decoding path.

use crate::meeting_transcript::TimedText;
use std::path::PathBuf;

pub const MODEL_ID: &str = "word-aligner";
pub const MODEL_DIR: &str = "word-aligner";
pub const MODEL_FILE: &str = "model_quantized.onnx";
pub const VOCAB_FILE: &str = "vocab.json";

/// wav2vec2 emits one frame per 320 samples at 16 kHz.
const FRAME_MS: u64 = 20;
const BLANK: usize = 0; // <pad>
/// Longest clip aligned in one pass (the model's attention grows with length).
const MAX_AUDIO_MS: u64 = 45_000;

pub fn model_path() -> Option<PathBuf> {
    let p = crate::utils::get_models_dir().ok()?.join(MODEL_DIR).join(MODEL_FILE);
    p.exists().then_some(p)
}

/// One word of the transcript, as the characters the acoustic model knows.
struct Word<'a> {
    original: &'a str,
    /// Uppercase A-Z and apostrophe, as vocabulary ids; empty when the word has
    /// nothing the model can align (punctuation only).
    tokens: Vec<usize>,
}

/// English words for a whole number, so "42" aligns as "forty two".
fn spell_number(mut n: u64) -> String {
    const ONES: [&str; 20] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven",
        "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen",
    ];
    const TENS: [&str; 10] = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
    if n < 20 {
        return ONES[n as usize].to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    for (unit, name) in [(1_000_000_000u64, "billion"), (1_000_000, "million"), (1_000, "thousand")] {
        if n >= unit {
            parts.push(format!("{} {name}", spell_number(n / unit)));
            n %= unit;
        }
    }
    if n >= 100 {
        parts.push(format!("{} hundred", ONES[(n / 100) as usize]));
        n %= 100;
    }
    if n >= 20 {
        let t = TENS[(n / 10) as usize];
        parts.push(if n % 10 > 0 { format!("{t} {}", ONES[(n % 10) as usize]) } else { t.to_string() });
    } else if n > 0 {
        parts.push(ONES[n as usize].to_string());
    }
    parts.join(" ")
}

/// The alignable letters of one transcript word.
fn spoken_form(word: &str) -> String {
    let digits: String = word.chars().filter(|c| c.is_ascii_digit()).collect();
    let letters: String = word.chars().filter(|c| c.is_alphabetic() || *c == '\'' || *c == '’').collect();
    if letters.is_empty() && !digits.is_empty() {
        if let Ok(n) = digits.parse::<u64>() {
            return spell_number(n);
        }
    }
    letters
}

fn vocab_ids() -> std::collections::HashMap<char, usize> {
    // wav2vec2-base-960h: <pad>=0 <s>=1 </s>=2 <unk>=3 |=4, then letters by
    // frequency. Read from vocab.json when present, else the published table.
    let table: &[(char, usize)] = &[
        ('\'', 27), ('A', 7), ('B', 24), ('C', 19), ('D', 14), ('E', 5), ('F', 20), ('G', 21), ('H', 11),
        ('I', 10), ('J', 29), ('K', 26), ('L', 15), ('M', 17), ('N', 9), ('O', 8), ('P', 23), ('Q', 30),
        ('R', 13), ('S', 12), ('T', 6), ('U', 16), ('V', 25), ('W', 18), ('X', 28), ('Y', 22), ('Z', 31),
    ];
    table.iter().copied().collect()
}

fn tokenize<'a>(text: &'a str) -> Vec<Word<'a>> {
    let ids = vocab_ids();
    text.split_whitespace()
        .map(|original| {
            let spoken = spoken_form(original);
            let mut tokens = Vec::new();
            for c in spoken.chars() {
                let c = if c == '’' { '\'' } else { c };
                if c == ' ' {
                    continue;
                }
                for up in c.to_uppercase() {
                    if let Some(&id) = ids.get(&up) {
                        tokens.push(id);
                    }
                }
            }
            Word { original, tokens }
        })
        .collect()
}

