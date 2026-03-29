use regex::Regex;
use std::collections::HashSet;
use std::sync::OnceLock;

/// Post-process raw ASR output: fix punctuation artifacts and remove Whisper hallucinations.
pub fn clean_transcript(text: &str) -> String {
    let mut cleaned = text.trim().to_string();

    // Remove Whisper hallucination repetitions before anything else
    cleaned = remove_repetitions(&cleaned);

    // Strip known subtitle-style sound/caption tags ([music], (laughter), …) from Whisper / Granite
    cleaned = strip_whitelisted_sound_captions(&cleaned);

    // Fix floating punctuation
    cleaned = cleaned.replace(" ,", ",");
    cleaned = cleaned.replace(" .", ".");
    cleaned = cleaned.replace(" ?", "?");
    cleaned = cleaned.replace(" !", "!");

    // Fix percent signs
    cleaned = cleaned.replace(" %", "%");

    // Fix double spaces
    while cleaned.contains("  ") {
        cleaned = cleaned.replace("  ", " ");
    }

    // Capitalize first letter
    if let Some(first) = cleaned.chars().next() {
        if first.is_lowercase() {
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
/// Used for live streaming chunks so the UI matches `clean_transcript` output. Whisper / Granite only.
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
    let stripped = n
        .trim_end_matches(|c: char| matches!(c, '.' | ',' | '!' | '?' | ':' | ';' | '…'));
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
        // Runs of 3+ identical tokens -> keep 1; shorter runs kept as-is
        let keep = if run >= 3 { 1 } else { run };
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

    // Deduplicate consecutive identical sentences (case-insensitive)
    let mut deduped: Vec<String> = Vec::new();
    let mut prev_key = String::new();
    for sent in sentences {
        let key = sent.to_lowercase();
        if key != prev_key {
            deduped.push(sent);
            prev_key = key;
        }
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

/// Helper: Find or create the directory to save models
pub fn get_models_dir() -> Result<std::path::PathBuf, String> {
    // Get the standard AppData folder (C:\Users\Name\AppData\Local)
    let app_data = dirs::data_local_dir().ok_or("Could not find AppData directory")?;

    // Append our specific folder: ...\Taurscribe\models
    let models_dir = app_data.join("Taurscribe").join("models");

    // Create folder if it doesn't exist
    std::fs::create_dir_all(&models_dir)
        .map_err(|e| format!("Failed to create models directory: {}", e))?;

    Ok(models_dir)
}
