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
    use accessibility_sys::{AXUIElementCopyAttributeValue, AXUIElementCreateSystemWide, kAXErrorSuccess};
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
