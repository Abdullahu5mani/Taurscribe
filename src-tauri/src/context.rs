/// Active-application context detection.
///
/// Returns a short string describing the currently focused application/window
/// (e.g. "Visual Studio Code – main.rs") that is injected into Whisper's
/// `initial_prompt` to bias decoding toward domain-relevant vocabulary.
///
/// Platform coverage:
///   Windows → GetForegroundWindow + GetWindowTextW (Win32, zero deps)
///   macOS   → AXFocusedApplication + kAXTitleAttribute (Accessibility API)
///   Linux   → not implemented (returns None)

/// Return the title of the currently focused window, or `None` if it cannot
/// be determined (unsupported platform, permission denied, empty title).
pub fn get_active_context() -> Option<String> {
    #[cfg(target_os = "windows")]
    return windows_context();

    #[cfg(target_os = "macos")]
    return macos_context();

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    return None;
}

// ── Windows ──────────────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn windows_context() -> Option<String> {
    use std::ffi::{c_void, OsString};
    use std::os::windows::ffi::OsStringExt;

    extern "system" {
        fn GetForegroundWindow() -> *mut c_void;
        fn GetWindowTextW(hwnd: *mut c_void, lp_string: *mut u16, n_max_count: i32) -> i32;
    }

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }

        let mut buf = vec![0u16; 256];
        let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), 256);
        if len <= 0 {
            return None;
        }

        OsString::from_wide(&buf[..len as usize])
            .into_string()
            .ok()
            .filter(|s| !s.trim().is_empty())
    }
}

// ── macOS ─────────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn macos_context() -> Option<String> {
    use accessibility_sys::{
        kAXErrorSuccess, AXUIElementCopyAttributeValue, AXUIElementCreateSystemWide,
    };
    use core_foundation::{
        base::{CFRelease, CFTypeRef, TCFType},
        string::{CFString, CFStringRef},
    };

    unsafe {
        // Get the system-wide accessibility element
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            return None;
        }

        // Get the frontmost application element
        let cf_app_attr = CFString::new("AXFocusedApplication");
        let mut focused_app: CFTypeRef = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            system,
            cf_app_attr.as_CFTypeRef() as *const _,
            &mut focused_app,
        );
        CFRelease(system as CFTypeRef);

        if err != kAXErrorSuccess || focused_app.is_null() {
            return None;
        }

        // Read its kAXTitleAttribute (the app/window name)
        let cf_title_attr = CFString::new("AXTitle");
        let mut title_ref: CFTypeRef = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            focused_app as accessibility_sys::AXUIElementRef,
            cf_title_attr.as_CFTypeRef() as *const _,
            &mut title_ref,
        );
        CFRelease(focused_app);

        if err != kAXErrorSuccess || title_ref.is_null() {
            return None;
        }

        let cf_str = CFString::wrap_under_create_rule(title_ref as CFStringRef);
        let title = cf_str.to_string();
        if title.trim().is_empty() {
            None
        } else {
            Some(title)
        }
    }
}

// ── Linux ─────────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn linux_context() -> Option<String> {
    // Try xdotool if available (X11 / XWayland)
    if let Ok(output) = std::process::Command::new("xdotool")
        .args(["getactivewindow", "getwindowname"])
        .output()
    {
        if output.status.success() {
            let title = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !title.is_empty() {
                return Some(title);
            }
        }
    }
    None
}

// ── Domain & Vocabulary Prompt Assembler ──────────────────────────────────────

/// Extracts technical/domain keywords based on the focused application or window title.
pub fn infer_domain_keywords(window_title: &str) -> Vec<&'static str> {
    let lower = window_title.to_lowercase();

    // Code editors & IDEs
    if lower.contains("visual studio code")
        || lower.contains("code")
        || lower.contains("cursor")
        || lower.contains("sublime")
        || lower.contains("xcode")
        || lower.contains("intellij")
        || lower.contains("pycharm")
        || lower.contains("webstorm")
        || lower.contains("rustrover")
        || lower.contains("nvim")
        || lower.contains("terminal")
        || lower.contains("iterm")
    {
        return vec!["function", "async", "await", "commit", "merge", "repo", "branch", "debug"];
    }

    // Meeting & communication apps
    if lower.contains("slack")
        || lower.contains("discord")
        || lower.contains("teams")
        || lower.contains("zoom")
        || lower.contains("meet")
    {
        return vec!["agenda", "action item", "sync", "timeline", "standup"];
    }

    // Medical / Clinical software
    if lower.contains("epic")
        || lower.contains("cerner")
        || lower.contains("ehr")
        || lower.contains("clinical")
    {
        return vec!["patient", "dosage", "diagnosis", "symptoms", "prescription"];
    }

    // Legal / Contract tools
    if lower.contains("docusign")
        || lower.contains("contract")
        || lower.contains("clio")
        || lower.contains("legal")
    {
        return vec!["clause", "agreement", "party", "jurisdiction", "indemnity"];
    }

    Vec::new()
}

/// Builds an optimized `initial_prompt` string for Whisper's autoregressive decoder.
///
/// Combines user-specified custom vocabulary terms with active window context and
/// domain keywords while staying within Whisper's prompt window limit (<250 characters).
pub fn build_dynamic_prompt(custom_vocab: &[String], include_active_window: bool) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    // 1. User custom vocabulary has the highest priority
    let valid_vocab: Vec<&str> = custom_vocab
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    if !valid_vocab.is_empty() {
        parts.push(valid_vocab.join(", "));
    }

    // 2. Active window context & domain keywords (if enabled)
    if include_active_window {
        if let Some(ctx) = get_active_context() {
            let clean_ctx = ctx.trim();
            if !clean_ctx.is_empty() {
                // Shorten window title if too long
                let short_ctx = crate::utils::truncate_utf8_with_ellipsis(clean_ctx, 60);
                parts.push(format!("Active: {}", short_ctx));

                let domain_kw = infer_domain_keywords(clean_ctx);
                if !domain_kw.is_empty() {
                    parts.push(domain_kw.join(", "));
                }
            }
        }
    }

    if parts.is_empty() {
        return None;
    }

    let joined = parts.join(". ");
    // Cap at 250 characters so prompt tokens leave maximum space for Whisper audio transcript
    let prompt = crate::utils::truncate_utf8_with_ellipsis(&joined, 250);

    Some(prompt)
}

/// Reads `custom_vocabulary` (array of strings) and `context_bias_enabled` (bool)
/// from the application's persisted `settings.json`.
pub fn load_custom_vocabulary_from_settings() -> (Vec<String>, bool) {
    // The store's settings.json (not next to the models folder, which can be
    // moved to another drive in Settings → Storage).
    let Some(settings_path) = crate::mcp_server::settings_path() else {
        return (Vec::new(), true);
    };

    if !settings_path.is_file() {
        return (Vec::new(), true);
    }

    let Ok(content) = std::fs::read_to_string(&settings_path) else {
        return (Vec::new(), true);
    };

    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return (Vec::new(), true);
    };

    let vocab: Vec<String> = json
        .get("custom_vocabulary")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let enabled = json
        .get("context_bias_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    (vocab, enabled)
}

// ── App category (FlowScribe v3 `<app=...>` tag) ─────────────────────────────

/// Maps an app name / window title to the category FlowScribe v3 was trained
/// on. Checked in order, so specific apps win over generic words.
pub fn app_category_for(title: &str) -> &'static str {
    let t = title.to_lowercase();
    let tokens: std::collections::HashSet<&str> =
        t.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    // Single words match whole words ("zed" not in "authorized"); phrases and
    // file extensions match as substrings.
    let has = |words: &[&str]| {
        words.iter().any(|w| if w.contains(' ') || w.starts_with('.') { t.contains(w) } else { tokens.contains(w) })
    };
    if has(&["terminal", "iterm", "iterm2", "warp", "ghostty", "powershell", "command prompt", "cmd.exe", "alacritty", "kitty", "wezterm", "konsole", "bash", "zsh"]) {
        "terminal"
    } else if has(&["visual studio", "vs code", "vscode", "xcode", "cursor", "zed", "intellij", "pycharm", "webstorm", "android studio", "sublime", "neovim", "vim", "github", "gitlab", ".rs", ".ts", ".py", ".js", ".go"]) {
        "code_editor"
    } else if has(&["chatgpt", "claude", "gemini", "perplexity", "copilot", "grok", "deepseek"]) {
        "ai_prompt"
    } else if has(&["mail", "outlook", "gmail", "thunderbird", "spark", "superhuman", "proton"]) {
        "email"
    } else if has(&["slack", "teams", "discord", "messages", "whatsapp", "telegram", "signal", "messenger", "imessage", "google chat", "wechat"]) {
        "chat"
    } else if has(&["calendar", "reminders", "todoist", "things", "ticktick", "asana", "trello", "linear", "jira"]) {
        "calendar_task"
    } else if has(&["notes", "obsidian", "notion", "bear", "evernote", "onenote", "logseq", "craft"]) {
        "notes"
    } else if has(&["word", "pages", "google docs", "docs.google", "libreoffice", "writer", "scrivener", "overleaf"]) {
        "document"
    } else if has(&["google search", "bing", "duckduckgo", "new tab", "search"]) {
        "search"
    } else {
        "generic"
    }
}

/// Category of the app the user is dictating into right now.
pub fn active_app_category() -> &'static str {
    let mut title = get_active_context().unwrap_or_default();
    #[cfg(target_os = "macos")]
    if let Some(window) = macos_focused_window_title() {
        // The app name alone can't tell Gmail from Slack in a browser.
        title = format!("{title} {window}");
    }
    app_category_for(&title)
}

#[cfg(target_os = "macos")]
fn macos_focused_window_title() -> Option<String> {
    use accessibility_sys::{kAXErrorSuccess, AXUIElementCopyAttributeValue, AXUIElementCreateSystemWide};
    use core_foundation::{
        base::{CFRelease, CFTypeRef, TCFType},
        string::{CFString, CFStringRef},
    };

    unsafe {
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            return None;
        }
        let copy = |el: accessibility_sys::AXUIElementRef, attr: &str| -> Option<CFTypeRef> {
            let key = CFString::new(attr);
            let mut out: CFTypeRef = std::ptr::null();
            let err = AXUIElementCopyAttributeValue(el, key.as_CFTypeRef() as *const _, &mut out);
            (err == kAXErrorSuccess && !out.is_null()).then_some(out)
        };
        let app = copy(system, "AXFocusedApplication");
        CFRelease(system as CFTypeRef);
        let app = app?;
        let window = copy(app as accessibility_sys::AXUIElementRef, "AXFocusedWindow");
        CFRelease(app);
        let window = window?;
        let title = copy(window as accessibility_sys::AXUIElementRef, "AXTitle");
        CFRelease(window);
        let title = CFString::wrap_under_create_rule(title? as CFStringRef).to_string();
        (!title.trim().is_empty()).then_some(title)
    }
}

/// Tauri command to inspect current active window and preview the assembled decoder prompt.
#[tauri::command]
pub fn get_active_context_preview(
    custom_vocab: Vec<String>,
    include_window: bool,
) -> Result<serde_json::Value, String> {
    let active_window = get_active_context();
    let prompt = build_dynamic_prompt(&custom_vocab, include_window);

    Ok(serde_json::json!({
        "active_window": active_window,
        "assembled_prompt": prompt,
        "custom_vocab_count": custom_vocab.len(),
    }))
}
/// Post-processes speech transcript to ensure exact casing and spelling for all configured
/// custom vocabulary words (e.g. "taurscribe" -> "Taurscribe", "usecallback" -> "useCallback").
pub fn apply_custom_vocabulary_casing(text: &str, custom_vocab: &[String]) -> String {
    if custom_vocab.is_empty() || text.is_empty() {
        return text.to_string();
    }

    let mut result = text.to_string();
    for term in custom_vocab {
        let clean_term = term.trim();
        if clean_term.is_empty() {
            continue;
        }
        let pattern = format!(r"(?i)\b{}\b", regex::escape(clean_term));
        if let Ok(re) = regex::Regex::new(&pattern) {
            result = re.replace_all(&result, clean_term).to_string();
        }
    }
    result
}

#[cfg(test)]
mod context_tests {
    use super::*;

    #[test]
    fn test_apply_custom_vocabulary_casing() {
        let vocab = vec!["Taurscribe".to_string(), "useCallback".to_string()];
        let raw = "Welcome to taurscribe, make sure you use USECALLBACK here.";
        let cleaned = apply_custom_vocabulary_casing(raw, &vocab);
        assert_eq!(cleaned, "Welcome to Taurscribe, make sure you use useCallback here.");
    }

    #[test]
    fn test_app_category_for() {
        assert_eq!(app_category_for("Slack"), "chat");
        assert_eq!(app_category_for("Google Chrome Inbox (3) - jane@acme.com - Gmail"), "email");
        assert_eq!(app_category_for("Code main.rs — taurscribe"), "code_editor");
        assert_eq!(app_category_for("iTerm2"), "terminal");
        assert_eq!(app_category_for("Safari ChatGPT"), "ai_prompt");
        assert_eq!(app_category_for("Finder"), "generic");
        assert_eq!(app_category_for("Passwords"), "generic");
        assert_eq!(app_category_for("Request authorized"), "generic");
    }

    #[test]
    fn test_domain_inference_code_editor() {
        let kw = infer_domain_keywords("Visual Studio Code - main.rs");
        assert!(kw.contains(&"async"));
        assert!(kw.contains(&"commit"));
    }

    #[test]
    fn test_domain_inference_meeting() {
        let kw = infer_domain_keywords("Slack | #general | Taurscribe");
        assert!(kw.contains(&"agenda"));
        assert!(kw.contains(&"sync"));
    }

    #[test]
    fn test_build_dynamic_prompt_with_vocab() {
        let vocab = vec!["Taurscribe".to_string(), "Montalais".to_string()];
        let prompt = build_dynamic_prompt(&vocab, false).unwrap();
        assert!(prompt.contains("Taurscribe"));
        assert!(prompt.contains("Montalais"));
    }

    #[test]
    fn test_build_dynamic_prompt_empty() {
        let vocab: Vec<String> = Vec::new();
        assert!(build_dynamic_prompt(&vocab, false).is_none());
    }

    #[test]
    fn test_prompt_length_capping() {
        let long_vocab: Vec<String> = (0..50).map(|i| format!("SuperLongTechnicalWord_{i}")).collect();
        let prompt = build_dynamic_prompt(&long_vocab, false).unwrap();
        assert!(prompt.len() <= 250);
        assert!(prompt.ends_with("..."));
    }
}
