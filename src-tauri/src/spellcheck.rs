use anyhow::{Error, Result};
use std::path::PathBuf;
use std::time::Instant;
use symspell::{SymSpell, Verbosity, UnicodeStringStrategy};

pub struct SpellChecker {
    symspell: SymSpell<UnicodeStringStrategy>,
}

impl SpellChecker {
    pub fn new() -> Result<Self> {
        let start = Instant::now();
        println!("[SPELL] Initializing SymSpell spell checker...");

        // Look for dictionary in runtime models folder
        let dict_path = PathBuf::from(
            r"c:\Users\abdul\OneDrive\Desktop\Taurscribe\taurscribe-runtime\models\frequency_dictionary_en_82_765.txt"
        );

        let mut symspell: SymSpell<UnicodeStringStrategy> = SymSpell::default();

        if dict_path.exists() {
            println!("[SPELL] Loading dictionary from: {:?}", dict_path);
            symspell.load_dictionary(
                dict_path.to_str().unwrap(),
                0,  // term_index
                1,  // count_index
                " " // separator
            );
            println!("[SPELL] Dictionary loaded in {:?}", start.elapsed());
        } else {
            println!("[SPELL] Warning: Dictionary not found at {:?}", dict_path);
            println!("[SPELL] Download from: https://github.com/wolfgarbe/SymSpell/blob/master/SymSpell/frequency_dictionary_en_82_765.txt");
            return Err(Error::msg(format!(
                "Dictionary not found. Please download frequency_dictionary_en_82_765.txt to {:?}",
                dict_path
            )));
        }

        Ok(Self { symspell })
    }

    /// Correct spelling in text (word by word)
    pub fn correct(&self, text: &str) -> String {
        let start = Instant::now();
        let words: Vec<&str> = text.split_whitespace().collect();
        let mut corrected_words = Vec::with_capacity(words.len());
        let mut corrections_made = 0;

        for word in &words {
            // Skip short words, numbers, and punctuation-only
            if word.len() <= 1 || word.chars().all(|c| c.is_numeric() || c.is_ascii_punctuation()) {
                corrected_words.push(word.to_string());
                continue;
            }

            // Strip punctuation for lookup
            let (prefix, clean_word, suffix) = strip_punctuation(word);
            
            if clean_word.is_empty() {
                corrected_words.push(word.to_string());
                continue;
            }

            // Look up the word
            let suggestions = self.symspell.lookup(
                &clean_word.to_lowercase(),
                Verbosity::Closest,
                2 // max edit distance
            );

            if let Some(suggestion) = suggestions.first() {
                // Only correct if it's actually different
                if suggestion.term.to_lowercase() != clean_word.to_lowercase() {
                    // Preserve original capitalization pattern
                    let corrected = match_case(&suggestion.term, &clean_word);
                    corrected_words.push(format!("{}{}{}", prefix, corrected, suffix));
                    corrections_made += 1;
                } else {
                    corrected_words.push(word.to_string());
                }
            } else {
                // No suggestion found, keep original
                corrected_words.push(word.to_string());
            }
        }

        let result = corrected_words.join(" ");
        println!(
            "[SPELL] Corrected {} words in {:?} ({} corrections)",
            words.len(),
            start.elapsed(),
            corrections_made
        );

        result
    }
}

/// Strip leading/trailing punctuation from a word
fn strip_punctuation(word: &str) -> (String, String, String) {
    let chars: Vec<char> = word.chars().collect();
    let mut start = 0;
    let mut end = chars.len();

    // Find start of actual word
    while start < end && chars[start].is_ascii_punctuation() {
        start += 1;
    }

    // Find end of actual word
    while end > start && chars[end - 1].is_ascii_punctuation() {
        end -= 1;
    }

    let prefix: String = chars[..start].iter().collect();
    let core: String = chars[start..end].iter().collect();
    let suffix: String = chars[end..].iter().collect();

    (prefix, core, suffix)
}

/// Match the capitalization pattern of the original word
fn match_case(suggestion: &str, original: &str) -> String {
    if original.chars().all(|c| c.is_uppercase()) {
        // ALL CAPS
        suggestion.to_uppercase()
    } else if original.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
        // Title Case
        let mut chars: Vec<char> = suggestion.chars().collect();
        if let Some(first) = chars.first_mut() {
            *first = first.to_uppercase().next().unwrap_or(*first);
        }
        chars.into_iter().collect()
    } else {
        // lowercase
        suggestion.to_lowercase()
    }
}
