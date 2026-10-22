use regex::Regex;
use std::collections::HashSet;
use std::sync::OnceLock;

/// Truncate to a byte budget without splitting a UTF-8 code point.
pub fn truncate_utf8_with_ellipsis(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let suffix = if max_bytes >= 3 { "..." } else { "" };
    let mut end = max_bytes.saturating_sub(suffix.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &text[..end], suffix)
}

#[cfg(test)]
mod utf8_truncation_tests {

    use super::truncate_utf8_with_ellipsis;

    #[test]
    fn truncates_on_character_boundaries() {
        assert_eq!(truncate_utf8_with_ellipsis("é🙂abcd", 8), "é...");
        assert_eq!(truncate_utf8_with_ellipsis("é🙂abcd", 9), "é🙂...");
        assert_eq!(truncate_utf8_with_ellipsis("é🙂abcd", 10), "é🙂abcd");
    }

    proptest::proptest! {
        #[test]
        fn truncated_text_stays_valid_utf8_and_within_the_budget(
            text in "\\PC{0,80}",
            max in 0usize..64,
        ) {
            let out = truncate_utf8_with_ellipsis(&text, max);
            assert!(std::str::from_utf8(out.as_bytes()).is_ok());
            if text.len() <= max {
                assert_eq!(out, text);
            } else {
                assert!(out.len() <= max, "len {} > max {max} for {out:?}", out.len());
            }
        }
    }
}

/// Post-process raw ASR output: fix punctuation artifacts and remove Whisper hallucinations.
pub fn clean_transcript(text: &str) -> String {
    let mut cleaned = text.trim().to_string();

    // Remove Whisper hallucination repetitions before anything else
    cleaned = remove_repetitions(&cleaned);

    // Strip known subtitle-style sound/caption tags ([music], (laughter), …) from Whisper / Qwen3
    cleaned = strip_whitelisted_sound_captions(&cleaned);

    // Fix floating punctuation ("hello , world" / "50 %"): only a mark that
    // stands on its own (then a space, the end, or a closing quote/bracket).
    // One that starts a word (".NET", ".gitignore") keeps
    // its space; a plain " ." replace turned "use .NET" into "use.NET".
    static FLOATING: OnceLock<Regex> = OnceLock::new();
    let floating = FLOATING.get_or_init(|| Regex::new(r#" +([,.?!%]+)(\s|$|["')\]\u{201D}\u{2019}])"#).unwrap());
    cleaned = floating.replace_all(&cleaned, "${1}${2}").into_owned();

    // Fix double spaces
    while cleaned.contains("  ") {
        cleaned = cleaned.replace("  ", " ");
    }

    // Capitalize first letter, unless the first word has its own casing ("iPhone", "macOS")
    let first_word = cleaned.split_whitespace().next().unwrap_or("");
    let own_casing = first_word.chars().skip(1).any(char::is_uppercase);
    if let Some(first) = cleaned.chars().next() {
        if first.is_lowercase() && !own_casing {
            let mut c = cleaned.chars();
            cleaned = match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            };
        }
    }

    cleaned
}

/// Remove `[…]` / `(…)` segments only when the inner text matches a known ASR sound/caption label.
/// Used for live streaming chunks so the UI matches `clean_transcript` output. Whisper / Qwen3 only.
pub(crate) fn strip_whitelisted_sound_captions(text: &str) -> String {
    static RE_BRACKETS: OnceLock<Regex> = OnceLock::new();
    static RE_PARENS: OnceLock<Regex> = OnceLock::new();
    let re_b = RE_BRACKETS.get_or_init(|| Regex::new(r"\[[^\]\n\r]*\]").unwrap());
    let re_p = RE_PARENS.get_or_init(|| Regex::new(r"\([^)\n\r]{1,200}\)").unwrap());

    let mut s = strip_labeled_regions(text, re_b, '[', ']', is_whitelisted_sound_caption_inner);
    s = strip_labeled_regions(&s, re_p, '(', ')', is_whitelisted_sound_caption_inner);
    collapse_spaces_trim(&s)
}

fn strip_labeled_regions(
    text: &str,
    re: &Regex,
    open: char,
    close: char,
    is_tag: fn(&str) -> bool,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    for m in re.find_iter(text) {
        out.push_str(&text[last..m.start()]);
        let frag = m.as_str();
        if frag.len() >= 2
            && frag.starts_with(open)
            && frag.ends_with(close)
            && is_tag(&frag[1..frag.len() - 1])
        {
            // drop
        } else {
            out.push_str(frag);
        }
        last = m.end();
    }
    out.push_str(&text[last..]);
    out
}

fn is_whitelisted_sound_caption_inner(inner: &str) -> bool {
    let n = normalize_sound_tag_inner(inner);
    if n.is_empty() {
        return false;
    }
    let set = sound_caption_whitelist();
    if set.contains(&n) {
        return true;
    }
    // "[(laughter)]" → inner "(laughter)"
    if n.len() >= 2 && n.starts_with('(') && n.ends_with(')') {
        let inner2 = normalize_sound_tag_inner(&n[1..n.len() - 1]);
        if !inner2.is_empty() && set.contains(&inner2) {
            return true;
        }
    }
    let stripped =
        n.trim_end_matches(|c: char| matches!(c, '.' | ',' | '!' | '?' | ':' | ';' | '…'));
    stripped != n.as_str() && set.contains(stripped)
}

fn normalize_sound_tag_inner(s: &str) -> String {
    s.trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn sound_caption_whitelist() -> &'static HashSet<String> {
    static SET: OnceLock<HashSet<String>> = OnceLock::new();
    SET.get_or_init(|| {
        const RAW: &[&str] = &[
            "applause",
            "applause and laughter",
            "audience",
            "audience applauding",
            "audience laughing",
            "audience laughter",
            "audio",
            "awkward silence",
            "background music",
            "background noise",
            "beat",
            "beep",
            "beeping",
            "birds chirping",
            "blank",
            "blank audio",
            "blank_audio",
            "cheering",
            "cheers",
            "clapping",
            "click",
            "clicking",
            "cough",
            "coughing",
            "crowd chattering",
            "crowd cheering",
            "crowd laughing",
            "crowd noise",
            "crosstalk",
            "distorted speech",
            "dog barking",
            "door closes",
            "door opening",
            "doorbell",
            "dramatic music",
            "explosion",
            "film music",
            "footsteps",
            "foreign language",
            "foreign speech",
            "gasps",
            "giggling",
            "gunshot",
            "horn honking",
            "hum",
            "humming",
            "inaudible",
            "indistinct",
            "instrumental",
            "intro music",
            "laughing",
            "laughter",
            "laughs",
            "light applause",
            "long silence",
            "loud music",
            "music",
            "music fades",
            "music fades out",
            "music playing",
            "muffled",
            "mumbled",
            "noise",
            "no audio",
            "no sound",
            "outro music",
            "overlapping dialogue",
            "overlapping speech",
            "overlapping voices",
            "phone ringing",
            "radio",
            "rain",
            "sfx",
            "sigh",
            "sighs",
            "silence",
            "singing",
            "sneezing",
            "soft music",
            "sound",
            "sound effect",
            "sound effects",
            "speaking another language",
            "speaking foreign language",
            "speaking in foreign language",
            "speech",
            "static",
            "sustained applause",
            "television",
            "theme music",
            "thunder",
            "thunderous applause",
            "tick",
            "ticking",
            "tv music",
            "tv playing",
            "typing",
            "unintelligible",
            "upbeat music",
            "vocals",
            "vocalizing",
            "water running",
            "white noise",
            "wind",
            "music stops",
            "music starts",
            "music swells",
            "car engine",
            "ambient noise",
            "audience cheers",
            "crowd applause",
            "laughter and applause",
            "music continues",
            "no speech",
            "silence.",
            "applause.",
            "laughter.",
            "footsteps.",
        ];
        RAW.iter().map(|s| (*s).to_string()).collect()
    })
}

fn collapse_spaces_trim(s: &str) -> String {
    let mut t = s.trim().to_string();
    while t.contains("  ") {
        t = t.replace("  ", " ");
    }
    t
}

/// Collapse consecutive repeated tokens and sentences — Whisper's hallucination signature
/// when it encounters silence, footsteps, static, or other ambient noise.
///
/// Two passes:
///   Pass 1 — token-level: "[ [ [ [" -> "["  |  "(footsteps) (footsteps) ..." -> "(footsteps)"
///   Pass 2 — sentence-level: "Okay. Okay. Okay." -> "Okay."
///
/// A run of 3+ identical tokens is collapsed to 1. Consecutive duplicate sentences
/// (any length) are collapsed to 1.
/// Whisper can loop on one word or sentence ("you you you you you …"). People
/// repeat themselves too ("No. No.", "very very very"), so only runs longer
/// than anyone says on purpose are collapsed, to a single copy.
const MAX_REPEATED_WORDS: usize = 4;
const MAX_REPEATED_SENTENCES: usize = 3;

fn remove_repetitions(text: &str) -> String {
    // ── Pass 1: token-level ──────────────────────────────────────────────────
    let parts: Vec<&str> = text.split(' ').collect();
    let mut t_out: Vec<&str> = Vec::with_capacity(parts.len());
    let mut i = 0;
    while i < parts.len() {
        let tok = parts[i];
        let mut j = i + 1;
        while j < parts.len() && parts[j].eq_ignore_ascii_case(tok) {
            j += 1;
        }
        let run = j - i;
        let keep = if run > MAX_REPEATED_WORDS { 1 } else { run };
        t_out.extend_from_slice(&parts[i..i + keep]);
        i = j;
    }
    let phase1 = t_out.join(" ");

    // ── Pass 2: sentence-level ───────────────────────────────────────────────
    // Split on ". " / "? " / "! " boundaries (sentence-ending punct followed by space).
    // Each sentence string retains its trailing punctuation.
    let mut sentences: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut chars = phase1.chars().peekable();
    while let Some(c) = chars.next() {
        buf.push(c);
        if matches!(c, '.' | '?' | '!') && chars.peek() == Some(&' ') {
            chars.next(); // consume the space
            sentences.push(buf.trim().to_string());
            buf.clear();
        }
    }
    if !buf.trim().is_empty() {
        sentences.push(buf.trim().to_string());
    }

    // Collapse long runs of the same sentence (case-insensitive)
    let mut deduped: Vec<String> = Vec::new();
    let mut i = 0;
    while i < sentences.len() {
        let key = sentences[i].to_lowercase();
        let run = sentences[i..].iter().take_while(|s| s.to_lowercase() == key).count();
        let keep = if run > MAX_REPEATED_SENTENCES { 1 } else { run };
        deduped.extend(sentences[i..i + keep].iter().cloned());
        i += run;
    }

    deduped.join(" ")
}

/// Helper: Find or create the directory to save recordings
pub fn get_recordings_dir() -> Result<std::path::PathBuf, String> {
    // Get the standard AppData folder (C:\Users\Name\AppData\Local)
    let app_data = dirs::data_local_dir().ok_or("Could not find AppData directory")?;

    // Append our specific folder: ...\Taurscribe\temp
    let recordings_dir = app_data.join("Taurscribe").join("temp");

    // Create folder if it doesn't exist
    std::fs::create_dir_all(&recordings_dir)
        .map_err(|e| format!("Failed to create recordings directory: {}", e))?;

    Ok(recordings_dir)
}

/// Helper: Find or create the directory to save models (default
/// AppData/Local/Taurscribe/models, or the folder chosen in Settings → Storage).
pub fn get_models_dir() -> Result<std::path::PathBuf, String> {
    crate::storage::resolve_dir(crate::storage::Area::Models)
}

#[cfg(test)]
mod repetition_tests {
    use super::remove_repetitions;

    #[test]
    fn spoken_repetition_is_kept() {
        for text in ["one one one", "No. No.", "No. No. No.", "That is very very very important.", "no no no no"] {
            assert_eq!(remove_repetitions(text), text);
        }
    }

    #[test]
    fn hallucination_loops_are_collapsed() {
        assert_eq!(remove_repetitions("you you you you you you you you"), "you");
        assert_eq!(remove_repetitions("I said thank thank thank thank thank you"), "I said thank you");
        assert_eq!(
            remove_repetitions("Thanks for watching. Thanks for watching. Thanks for watching. Thanks for watching."),
            "Thanks for watching."
        );
        assert_eq!(remove_repetitions("Hello. Bye. Bye. Bye. Bye. Bye. See you."), "Hello. Bye. See you.");
    }
}

#[cfg(test)]
mod clean_transcript_tests {
    #[test]
    fn cleanup_keeps_the_space_before_a_word_that_starts_with_punctuation() {
        assert_eq!(super::clean_transcript("I use .NET daily"), "I use .NET daily");
        assert_eq!(super::clean_transcript("rename it to .gitignore please"), "Rename it to .gitignore please");
        assert_eq!(super::clean_transcript("about ,5 or ?x"), "About ,5 or ?x");
        // Floating punctuation is still attached to the word before it.
        assert_eq!(super::clean_transcript("hello , world . done ! 50 %"), "Hello, world. done! 50%");
        assert_eq!(super::clean_transcript("is it ?"), "Is it?");
        assert_eq!(super::clean_transcript("she said \"stop !\" (then left .)"), "She said \"stop!\" (then left.)");
    }

    use super::clean_transcript;

    #[test]
    fn first_letter_is_capitalised_unless_the_word_has_its_own_casing() {
        assert_eq!(clean_transcript("hello there ."), "Hello there.");
        assert_eq!(clean_transcript("élan , mostly"), "Élan, mostly");
        for text in ["iPhone sales are up.", "macOS needs an update", "eBay sent the refund", "mRNA vaccines work"] {
            assert_eq!(clean_transcript(text), text);
        }
    }

    proptest::proptest! {
        #[test]
        fn cleaning_any_utf8_does_not_panic(text in "\\PC{0,200}") {
            let out = clean_transcript(&text);
            assert!(std::str::from_utf8(out.as_bytes()).is_ok());
        }
    }
}
