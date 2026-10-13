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

/// CTC Viterbi forced alignment. `log_probs` is `frames x vocab`, row-major.
/// Returns, for each label, the first and last frame it occupies.
pub fn ctc_force_align(log_probs: &[f32], frames: usize, vocab: usize, labels: &[usize]) -> Option<Vec<(usize, usize)>> {
    let l = labels.len();
    if l == 0 || frames == 0 || log_probs.len() != frames * vocab {
        return None;
    }
    let s = 2 * l + 1;
    // The path needs a frame per label plus a blank between equal neighbours.
    let repeats = labels.windows(2).filter(|w| w[0] == w[1]).count();
    if frames < l + repeats {
        return None;
    }
    let state_label = |i: usize| if i % 2 == 0 { BLANK } else { labels[i / 2] };
    let lp = |t: usize, i: usize| log_probs[t * vocab + state_label(i)];
    const NEG: f32 = f32::NEG_INFINITY;
    let mut dp = vec![NEG; frames * s];
    // Where each state's best score came from: 0 stay, 1 one state back, 2 two back.
    let mut from = vec![0u8; frames * s];
    dp[0] = lp(0, 0);
    if s > 1 {
        dp[1] = lp(0, 1);
    }
    for t in 1..frames {
        for i in 0..s {
            let mut best = dp[(t - 1) * s + i];
            let mut how = 0u8;
            if i >= 1 && dp[(t - 1) * s + i - 1] > best {
                best = dp[(t - 1) * s + i - 1];
                how = 1;
            }
            // Skip over a blank, unless the labels either side are the same.
            if i >= 2 && i % 2 == 1 && state_label(i) != state_label(i - 2) && dp[(t - 1) * s + i - 2] > best {
                best = dp[(t - 1) * s + i - 2];
                how = 2;
            }
            if best > NEG {
                dp[t * s + i] = best + lp(t, i);
                from[t * s + i] = how;
            }
        }
    }
    let last = (frames - 1) * s;
    let mut state = if s > 1 && dp[last + s - 2] >= dp[last + s - 1] { s - 2 } else { s - 1 };
    if dp[last + state] == NEG {
        return None;
    }
    // Backtrack, recording each label's frame span.
    let mut spans: Vec<(usize, usize)> = vec![(usize::MAX, 0); l];
    for t in (0..frames).rev() {
        if state % 2 == 1 {
            let k = state / 2;
            spans[k].0 = spans[k].0.min(t);
            spans[k].1 = spans[k].1.max(t);
        }
        if t == 0 {
            break;
        }
        state -= from[t * s + state] as usize;
    }
    if !spans.iter().all(|&(a, b)| a != usize::MAX && b >= a) {
        return None;
    }
    // A label "occupies" frames where the path merely waited for the next
    // one (the model is not peaky enough to leave those as blank). Time each
    // label by the frames where it is actually likely, so a word does not
    // swallow the silence after it.
    const CONFIDENT: f32 = -2.3; // p > 0.1
    for (k, span) in spans.iter_mut().enumerate() {
        let label = labels[k];
        let sure: Vec<usize> = (span.0..=span.1).filter(|&t| log_probs[t * vocab + label] > CONFIDENT).collect();
        *span = match (sure.first(), sure.last()) {
            (Some(&a), Some(&b)) => (a, b),
            _ => {
                let peak = (span.0..=span.1)
                    .max_by(|&x, &y| log_probs[x * vocab + label].total_cmp(&log_probs[y * vocab + label]))
                    .unwrap_or(span.0);
                (peak, peak)
            }
        };
    }
    Some(spans)
}

/// Turns label spans into one `(start_ms, end_ms)` per word, interpolating any
/// word that has no alignable characters, and keeping the words in order.
fn word_times(words: &[Word<'_>], spans: &[(usize, usize)], total_ms: u64) -> Vec<(u64, u64)> {
    let mut cursor = 0usize;
    let mut raw: Vec<Option<(u64, u64)>> = Vec::with_capacity(words.len());
    for w in words {
        if w.tokens.is_empty() {
            raw.push(None);
            continue;
        }
        let first = spans[cursor].0;
        let last = spans[cursor + w.tokens.len() - 1].1;
        cursor += w.tokens.len();
        let (start, mut end) = (first as u64 * FRAME_MS, (last as u64 + 1) * FRAME_MS);
        // The decoder and the acoustic model sometimes disagree about a word;
        // the aligner then stretches it over its neighbours' silence. A word
        // lasts about a quarter second plus ~90 ms a letter.
        let longest = 250 + 90 * w.tokens.len() as u64;
        if end - start > longest {
            end = start + longest;
        }
        raw.push(Some((start, end)));
    }
    let mut out: Vec<(u64, u64)> = Vec::with_capacity(words.len());
    for i in 0..raw.len() {
        let t = match raw[i] {
            Some(t) => t,
            None => {
                let before = out.last().map(|p| p.1).unwrap_or(0);
                let after = raw[i + 1..].iter().flatten().next().map(|t| t.0).unwrap_or(total_ms);
                (before, after.max(before + FRAME_MS))
            }
        };
        let start = t.0.max(out.last().map(|p| p.1).unwrap_or(0));
        out.push((start, t.1.max(start + FRAME_MS)));
    }
    out
}

/// Aligns `text` to 16 kHz mono `pcm`; one unit per whitespace-separated word.
pub fn align_words_with<F>(pcm: &[f32], text: &str, mut logits: F) -> Option<Vec<TimedText>>
where
    F: FnMut(&[f32]) -> Option<(Vec<f32>, usize, usize)>,
{
    let total_ms = pcm.len() as u64 / 16;
    if total_ms == 0 || total_ms > MAX_AUDIO_MS {
        return None;
    }
    let words = tokenize(text);
    let labels: Vec<usize> = words.iter().flat_map(|w| w.tokens.iter().copied()).collect();
    if labels.is_empty() {
        return None;
    }
    let (log_probs, frames, vocab) = logits(pcm)?;
    let spans = ctc_force_align(&log_probs, frames, vocab, &labels)?;
    let stretched = {
        let mut cursor = 0usize;
        let mut n = 0usize;
        for w in &words {
            if w.tokens.is_empty() {
                continue;
            }
            let (first, last) = (spans[cursor].0 as u64, spans[cursor + w.tokens.len() - 1].1 as u64);
            if (last + 1 - first) * FRAME_MS > 250 + 90 * w.tokens.len() as u64 {
                n += 1;
            }
            cursor += w.tokens.len();
        }
        n
    };
    // When the audio does not match the text, many words stretch: do not trust it.
    if stretched * 5 > words.iter().filter(|w| !w.tokens.is_empty()).count() {
        return None;
    }
    let times = word_times(&words, &spans, total_ms);
    // Plausibility: the speech should not be squeezed into a sliver of the clip.
    let covered = times.last().map(|t| t.1).unwrap_or(0).saturating_sub(times.first().map(|t| t.0).unwrap_or(0));
    if covered * 5 < total_ms.min(labels.len() as u64 * 60) {
        return None;
    }
    Some(
        words
            .iter()
            .zip(times)
            .map(|(w, (s, e))| TimedText { start_ms: s, end_ms: e.min(total_ms.max(s + FRAME_MS)), text: w.original.to_string() })
            .collect(),
    )
}

// ── ONNX model ───────────────────────────────────────────────────────────────

pub struct WordAligner {
    session: ort::session::Session,
}

impl WordAligner {
    pub fn load() -> Result<Self, String> {
        let path = model_path().ok_or("word alignment model not installed")?;
        let session = ort::session::Session::builder()
            .map_err(|e| format!("ort builder: {e}"))?
            .commit_from_file(&path)
            .map_err(|e| format!("load {}: {e}", path.display()))?;
        Ok(Self { session })
    }

    /// `frames x vocab` log-probabilities for 16 kHz mono audio.
    fn log_probs(&mut self, pcm: &[f32]) -> Option<(Vec<f32>, usize, usize)> {
        let n = pcm.len() as f32;
        let mean = pcm.iter().sum::<f32>() / n;
        let var = pcm.iter().map(|x| (x - mean) * (x - mean)).sum::<f32>() / n;
        let inv = 1.0 / (var + 1e-7).sqrt();
        let input: Vec<f32> = pcm.iter().map(|x| (x - mean) * inv).collect();
        let tensor = ort::value::Value::from_array((vec![1usize, input.len()], input)).ok()?;
        let outputs = self.session.run(ort::inputs![tensor]).ok()?;
        let (_, value) = outputs.iter().next()?;
        let (shape, data) = value.try_extract_tensor::<f32>().ok()?;
        let dims: Vec<usize> = shape.iter().map(|&d| d as usize).collect();
        let (frames, vocab) = match dims.as_slice() {
            [1, f, v] => (*f, *v),
            [f, v] => (*f, *v),
            _ => return None,
        };
        // log-softmax per frame
        let mut out = Vec::with_capacity(data.len());
        for row in data.chunks(vocab) {
            let max = row.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            let sum: f32 = row.iter().map(|x| (x - max).exp()).sum();
            let log_sum = sum.ln() + max;
            out.extend(row.iter().map(|x| x - log_sum));
        }
        Some((out, frames, vocab))
    }

    pub fn align(&mut self, pcm: &[f32], text: &str) -> Option<Vec<TimedText>> {
        align_words_with(pcm, text, |audio| self.log_probs(audio))
    }
}

/// Aligns using a lazily loaded process-wide model; `None` when unavailable.
pub fn align_transcript(pcm: &[f32], text: &str) -> Option<Vec<TimedText>> {
    use std::sync::{Mutex, OnceLock};
    static ALIGNER: OnceLock<Mutex<Option<WordAligner>>> = OnceLock::new();
    let cell = ALIGNER.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock().ok()?;
    if guard.is_none() {
        *guard = WordAligner::load().ok();
    }
    guard.as_mut()?.align(pcm, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_spelled_for_alignment() {
        assert_eq!(spell_number(0), "zero");
        assert_eq!(spell_number(14), "fourteen");
        assert_eq!(spell_number(42), "forty two");
        assert_eq!(spell_number(105), "one hundred five");
        assert_eq!(spell_number(2026), "two thousand twenty six");
    }

    #[test]
    fn words_keep_their_original_text_and_drop_punctuation_only_tokens() {
        let w = tokenize("Hello, world — it's 3 PM.");
        assert_eq!(w.iter().map(|x| x.original).collect::<Vec<_>>(), ["Hello,", "world", "—", "it's", "3", "PM."]);
        assert!(w[2].tokens.is_empty());
        assert_eq!(w[0].tokens.len(), 5);
        assert_eq!(w[3].tokens.len(), 4); // I T ' S
        assert_eq!(w[4].tokens.len(), "three".len());
    }

    /// Log-probs where label `k` of `labels` is the confident class over its frames.
    fn synthetic(labels: &[usize], frames_per: usize, gap: usize, vocab: usize) -> (Vec<f32>, usize) {
        let mut rows: Vec<usize> = Vec::new();
        for (i, &l) in labels.iter().enumerate() {
            rows.extend(std::iter::repeat(l).take(frames_per));
            if i + 1 < labels.len() {
                rows.extend(std::iter::repeat(BLANK).take(gap));
            }
        }
        let mut lp = vec![-8.0f32; rows.len() * vocab];
        for (t, &c) in rows.iter().enumerate() {
            lp[t * vocab + c] = -0.01;
        }
        (lp, rows.len())
    }

    #[test]
    fn viterbi_recovers_label_spans() {
        let labels = [5, 6, 6, 9]; // note the repeat: needs a blank between
        let (lp, frames) = synthetic(&labels, 3, 2, 32);
        let spans = ctc_force_align(&lp, frames, 32, &labels).unwrap();
        // label k starts at k * (3 + 2)
        for (k, (a, b)) in spans.iter().enumerate() {
            assert_eq!(*a, k * 5, "{spans:?}");
            assert_eq!(*b, k * 5 + 2, "{spans:?}");
        }
    }

    #[test]
    fn too_few_frames_cannot_hold_the_text() {
        let lp = vec![-1.0f32; 3 * 32];
        assert!(ctc_force_align(&lp, 3, 32, &[5, 6, 7, 8]).is_none());
    }

    #[test]
    fn words_are_timed_in_order_and_punctuation_is_interpolated() {
        // "ab — cd": a,b then an unalignable dash then c,d; 4 frames each letter.
        let ids = vocab_ids();
        let (a, b, c, d) = (ids[&'A'], ids[&'B'], ids[&'C'], ids[&'D']);
        let labels = [a, b, c, d];
        let (lp, frames) = synthetic(&labels, 4, 1, 32);
        let pcm = vec![0.1f32; frames * 320];
        let out = align_words_with(&pcm, "ab — cd", |_| Some((lp.clone(), frames, 32))).unwrap();
        assert_eq!(out.iter().map(|u| u.text.as_str()).collect::<Vec<_>>(), ["ab", "—", "cd"]);
        for w in out.windows(2) {
            assert!(w[0].end_ms <= w[1].start_ms, "{out:?}");
        }
        assert!(out.iter().all(|u| u.end_ms > u.start_ms));
        assert!(out[0].start_ms < out[2].start_ms);
    }

    #[test]
    fn over_long_audio_is_not_attempted() {
        let pcm = vec![0.0f32; 16 * 46_000];
        assert!(align_words_with(&pcm, "hello", |_| unreachable!()).is_none());
    }
}
