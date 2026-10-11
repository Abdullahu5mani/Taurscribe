// Fixed-fixture accuracy oracle shared by live ASR and engine lifecycle checks.
pub const JFK_REFERENCE: &str = "And so my fellow Americans ask not what your country can do for you ask what you can do for your country";

pub fn word_error_rate(reference: &str, actual: &str) -> f64 {
    let words = |text: &str| {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .map(str::to_lowercase)
            .collect::<Vec<_>>()
    };
    let expected = words(reference);
    let actual = words(actual);
    let mut previous: Vec<usize> = (0..=actual.len()).collect();
    for (i, word) in expected.iter().enumerate() {
        let mut row = vec![i + 1; actual.len() + 1];
        for (j, found) in actual.iter().enumerate() {
            row[j + 1] = (previous[j] + usize::from(word != found))
                .min(previous[j + 1] + 1)
                .min(row[j] + 1);
        }
        previous = row;
    }
    previous[actual.len()] as f64 / expected.len().max(1) as f64
}

pub fn check_transcript(engine: &str, text: &str) -> Result<f64, String> {
    let wer = word_error_rate(JFK_REFERENCE, text);
    eprintln!("{engine}: WER={wer:.3}; transcript={text:?}");
    if wer > 0.25 {
        Err(format!(
            "{engine}: JFK word error rate {wer:.3} exceeds 0.25"
        ))
    } else {
        Ok(wer)
    }
}
