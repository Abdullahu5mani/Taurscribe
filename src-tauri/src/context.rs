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
                let short_ctx = if clean_ctx.len() > 60 {
                    let mut s = clean_ctx[..57].to_string();
                    s.push_str("...");
                    s
                } else {
                    clean_ctx.to_string()
                };
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
    let prompt = if joined.len() > 250 {
        let mut truncated = joined[..247].to_string();
        truncated.push_str("...");
        truncated
    } else {
        joined
    };

    Some(prompt)
}

/// Reads `custom_vocabulary` (array of strings) and `context_bias_enabled` (bool)
/// from the application's persisted `settings.json`.
pub fn load_custom_vocabulary_from_settings() -> (Vec<String>, bool) {
    let Ok(models_dir) = crate::utils::get_models_dir() else {
        return (Vec::new(), true);
    };
    let settings_path = models_dir
        .parent()
        .unwrap_or(&models_dir)
        .join("settings.json");

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
