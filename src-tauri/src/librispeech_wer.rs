//! Text normalization and word-error rate for LibriSpeech offline eval.
//! Same normalization is applied to reference and hypothesis before token alignment.

/// Lowercase, map non-alphanumeric (except apostrophe) to spaces, collapse whitespace.
pub fn normalize_for_wer(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let cleaned: String = lower
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '\'' {
                c
            } else {
                ' '
            }
        })
        .collect();
    cleaned
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Word-level WER = edit distance(ref, hyp) / max(len(ref), 1).
pub fn word_error_rate(ref_tokens: &[String], hyp_tokens: &[String]) -> f64 {
    if ref_tokens.is_empty() {
        return if hyp_tokens.is_empty() { 0.0 } else { 1.0 };
    }
    let dist = levenshtein_tokens(ref_tokens, hyp_tokens);
    dist as f64 / ref_tokens.len() as f64
}

fn levenshtein_tokens(a: &[String], b: &[String]) -> usize {
    let n = a.len();
    let m = b.len();
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr = vec![0usize; m + 1];
    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (curr[j - 1] + 1)
                .min(prev[j] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wer_perfect() {
        let r = normalize_for_wer("Hello world");
        let h = normalize_for_wer("hello world");
        assert!((word_error_rate(&r, &h) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn wer_one_substitution() {
        let r = normalize_for_wer("a b c");
        let h = normalize_for_wer("a x c");
        assert!((word_error_rate(&r, &h) - 1.0 / 3.0).abs() < 1e-9);
    }
}
