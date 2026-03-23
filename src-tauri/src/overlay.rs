/// Platform-aware overlay module.
///
/// macOS  → native NSPanel (objc2) with an egui::Context driving state logic.
///          The panel lives at kCGScreenSaverWindowLevel so it pierces full-screen
///          app Spaces. NSTextField renders the current phase text.
///
/// Win/Linux → Tauri WebView window "overlay" + emitted "overlay-state" events
///             (unchanged from the original implementation).

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayStatePayload {
    pub phase: String,
    pub text: Option<String>,
    pub ms: Option<u64>,
    pub engine: Option<String>,
}

// ── Public API (platform-dispatched) ─────────────────────────────────────────

/// Initialise the native overlay. Call once from `setup` on every platform.
/// No-op on Windows / Linux.
pub fn init(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    mac::init(app);
    #[cfg(not(target_os = "macos"))]
    let _ = app; // suppress unused-variable warning
}

/// Show the overlay and position it at the bottom-centre of the active screen.
pub fn show(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    mac::show(app);
    #[cfg(not(target_os = "macos"))]
    webview::show(app);
}

/// Hide the overlay.
pub fn hide(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    mac::hide(app);
    #[cfg(not(target_os = "macos"))]
    webview::hide(app);
}

/// Update the overlay state (phase + optional latency).
/// macOS   → updates NSTextField and egui context directly.
/// Win/Linux → emits "overlay-state" to the "overlay" WebView window.
pub fn set_state(app: &AppHandle, payload: OverlayStatePayload) {
    #[cfg(target_os = "macos")]
    mac::set_state(app, payload);
    #[cfg(not(target_os = "macos"))]
    webview::set_state(app, payload);
}

/// Restores focus to the app that was active when the overlay opened.
pub fn restore_focus(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app;
    #[cfg(not(target_os = "macos"))]
    webview::restore_focus(app);
}

// ── WebView implementation (Windows / Linux) ──────────────────────────────────

#[cfg(not(target_os = "macos"))]
mod webview {
    use super::OverlayStatePayload;
    use std::sync::{Mutex, OnceLock};
    use tauri::{AppHandle, Emitter, Manager};

    #[cfg(target_os = "windows")]
    static LAST_FOREGROUND_HWND: OnceLock<Mutex<usize>> = OnceLock::new();

    #[cfg(target_os = "windows")]
    fn last_foreground_hwnd() -> &'static Mutex<usize> {
        LAST_FOREGROUND_HWND.get_or_init(|| Mutex::new(0))
    }

    pub fn show(app: &AppHandle) {
        if let Some(overlay) = app.get_webview_window("overlay") {
            #[cfg(target_os = "windows")]
            remember_foreground_window();

            let monitor = active_monitor(app)
                .or_else(|| overlay.primary_monitor().ok().flatten());

            if let Some(m) = monitor {
                let msize = m.size();
                let mpos = m.position();
                let osize = overlay
                    .outer_size()
                    .unwrap_or(tauri::PhysicalSize::new(80, 80));
                let x = mpos.x + ((msize.width as i32 - osize.width as i32) / 2);
                let bottom_margin = (120.0 * m.scale_factor()) as i32;
                let y = mpos.y + msize.height as i32
                    - osize.height as i32
                    - bottom_margin;
                let _ = overlay.set_position(tauri::PhysicalPosition::new(x, y));
            }
            let _ = overlay.set_always_on_top(true);
            let _ = overlay.set_ignore_cursor_events(false);
            let _ = overlay.show();
        }
    }

    pub fn hide(app: &AppHandle) {
        if let Some(overlay) = app.get_webview_window("overlay") {
            let _ = overlay.hide();
        }
    }

    pub fn set_state(app: &AppHandle, payload: OverlayStatePayload) {
        if let Some(overlay) = app.get_webview_window("overlay") {
            let _ = overlay.emit("overlay-state", payload);
        }
    }

    pub fn restore_focus(_app: &AppHandle) {
        #[cfg(target_os = "windows")]
        restore_foreground_window();
    }

    /// Returns the monitor containing the foreground window (the app the user
    /// was typing in when they triggered the hotkey). Falls back to the cursor
    /// position, then to None.
    #[cfg(target_os = "windows")]
    pub fn active_monitor(app: &AppHandle) -> Option<tauri::Monitor> {
        foreground_monitor(app).or_else(|| cursor_monitor(app))
    }

    /// GetForegroundWindow → MonitorFromWindow → match against Tauri monitors.
    /// This is more accurate than cursor position: the cursor may be parked on
    /// a second screen while the user types on the primary.
    #[cfg(target_os = "windows")]
    fn foreground_monitor(app: &AppHandle) -> Option<tauri::Monitor> {
        use std::ffi::c_void;

        #[repr(C)]
        struct RECT { left: i32, top: i32, right: i32, bottom: i32 }

        #[repr(C)]
        struct MONITORINFO {
            cb_size: u32,
            rc_monitor: RECT,
            rc_work: RECT,
            dw_flags: u32,
        }

        extern "system" {
            fn GetForegroundWindow() -> *mut c_void;
            fn MonitorFromWindow(hwnd: *mut c_void, flags: u32) -> *mut c_void;
            fn GetMonitorInfoW(hmonitor: *mut c_void, lpmi: *mut MONITORINFO) -> i32;
        }

        const MONITOR_DEFAULTTONEAREST: u32 = 2;

        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() { return None; }

            let hmonitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            if hmonitor.is_null() { return None; }

            let mut info = MONITORINFO {
                cb_size: std::mem::size_of::<MONITORINFO>() as u32,
                rc_monitor: RECT { left: 0, top: 0, right: 0, bottom: 0 },
                rc_work: RECT { left: 0, top: 0, right: 0, bottom: 0 },
                dw_flags: 0,
            };
            if GetMonitorInfoW(hmonitor, &mut info) == 0 { return None; }

            // Match by top-left origin — Tauri uses the same coordinate space
            let ml = info.rc_monitor.left;
            let mt = info.rc_monitor.top;
            app.available_monitors().ok()?.into_iter().find(|m| {
                let pos = m.position();
                pos.x == ml && pos.y == mt
            })
        }
    }

    /// Cursor-position fallback (used when GetForegroundWindow returns null).
    #[cfg(target_os = "windows")]
    fn cursor_monitor(app: &AppHandle) -> Option<tauri::Monitor> {
        #[repr(C)]
        struct POINT { x: i32, y: i32 }
        extern "system" { fn GetCursorPos(lp: *mut POINT) -> i32; }
        let mut pt = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut pt) } == 0 { return None; }
        let (cx, cy) = (pt.x, pt.y);
        app.available_monitors().ok()?.into_iter().find(|m| {
            let pos = m.position();
            let size = m.size();
            cx >= pos.x && cx < pos.x + size.width as i32
                && cy >= pos.y && cy < pos.y + size.height as i32
        })
    }

    #[cfg(not(target_os = "windows"))]
    pub fn active_monitor(_app: &AppHandle) -> Option<tauri::Monitor> {
        None
    }

    #[cfg(target_os = "windows")]
    fn remember_foreground_window() {
        use std::ffi::c_void;

        extern "system" {
            fn GetForegroundWindow() -> *mut c_void;
        }

        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_null() {
            return;
        }

        if let Ok(mut slot) = last_foreground_hwnd().lock() {
            *slot = hwnd as usize;
        }
    }

    #[cfg(target_os = "windows")]
    fn restore_foreground_window() {
        use std::ffi::c_void;

        extern "system" {
            fn IsWindow(hwnd: *mut c_void) -> i32;
            fn SetForegroundWindow(hwnd: *mut c_void) -> i32;
        }

        let hwnd = match last_foreground_hwnd().lock() {
            Ok(slot) if *slot != 0 => *slot as *mut c_void,
            _ => return,
        };

        unsafe {
            if IsWindow(hwnd) != 0 {
                let _ = SetForegroundWindow(hwnd);
            }
        }
    }
}

// ── macOS native implementation ───────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod mac {
    use super::OverlayStatePayload;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use tauri::AppHandle;

    // NSStatusWindowLevel (25) is high enough to float above normal windows.
    // Combined with FullScreenAuxiliary it also appears on fullscreen Spaces.
    const STATUS_WINDOW_LEVEL: i64 = 25;

    // NSWindowCollectionBehavior raw flag values
    const CAN_JOIN_ALL_SPACES: u64 = 1 << 0;   // NSWindowCollectionBehaviorCanJoinAllSpaces
    const STATIONARY: u64 = 1 << 4;             // NSWindowCollectionBehaviorStationary
    const IGNORES_CYCLE: u64 = 1 << 6;          // NSWindowCollectionBehaviorIgnoresCycle
    const FULL_SCREEN_AUXILIARY: u64 = 1 << 8;  // NSWindowCollectionBehaviorFullScreenAuxiliary

    // NSFontWeight: medium ≈ 0.23
    const FONT_WEIGHT_MEDIUM: f64 = 0.23;

    // Raw pointers to AppKit objects — ONLY dereferenced on the main thread.
    static PANEL_PTR: AtomicUsize = AtomicUsize::new(0);
    static TEXTFIELD_PTR: AtomicUsize = AtomicUsize::new(0);

    // Shared phase state (written from any thread, read on main thread for rendering).
    static PHASE_STATE: OnceLock<Arc<Mutex<PhaseState>>> = OnceLock::new();

    #[derive(Clone, Default)]
    struct PhaseState {
        phase: String,
        done_ms: Option<u64>,
        engine: Option<String>,
    }

    fn phase_state() -> Arc<Mutex<PhaseState>> {
        PHASE_STATE
            .get_or_init(|| Arc::new(Mutex::new(PhaseState::default())))
            .clone()
    }

    // ── Public entry points ───────────────────────────────────────────────────

    pub fn init(app: &AppHandle) {
        let _ = app.run_on_main_thread(create_panel);
    }

    pub fn show(app: &AppHandle) {
        let _ = app.run_on_main_thread(show_panel);
    }

    pub fn hide(app: &AppHandle) {
        let _ = app.run_on_main_thread(hide_panel);
    }

    pub fn set_state(app: &AppHandle, payload: OverlayStatePayload) {
        {
            let arc = phase_state();
            let mut st = arc.lock().unwrap();
            st.phase = payload.phase.clone();
            st.done_ms = payload.ms;
            st.engine = payload.engine.clone();
        }
        let _ = app.run_on_main_thread(refresh_text);
    }

    // ── Main-thread functions ─────────────────────────────────────────────────

    fn create_panel() {
        use objc2::rc::Retained;
        use objc2::runtime::AnyObject;
        use objc2::{msg_send, ClassType};
        use objc2_app_kit::*;
        use objc2_foundation::*;

        // Safety: called only via run_on_main_thread
        let mtm = unsafe { MainThreadMarker::new_unchecked() };

        unsafe {
            // Already created
            if PANEL_PTR.load(Ordering::Relaxed) != 0 {
                return;
            }

            // 1. Borderless, non-activating NSPanel — won't steal focus from
            //    whatever fullscreen app the user is in.
            let panel: Retained<NSPanel> = NSPanel::initWithContentRect_styleMask_backing_defer(
                mtm.alloc(),
                NSRect::new(
                    NSPoint::new(-2000.0, -2000.0), // off-screen until show()
                    NSSize::new(340.0, 64.0),
                ),
                NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
                NSBackingStoreType::Buffered,
                false,
            );

            // 2. NSPanel-specific: float above all windows, never become key,
            //    and critically — do NOT hide when the app deactivates (which
            //    happens whenever a fullscreen app is in the foreground).
            panel.setFloatingPanel(true);
            panel.setBecomesKeyOnlyIfNeeded(true);
            panel.setHidesOnDeactivate(false);

            // 3. Window level + collection behavior.
            //    FullScreenAuxiliary is the flag that lets the panel appear on
            //    fullscreen Spaces (native macOS fullscreen, not exclusive-mode games).
            let level = STATUS_WINDOW_LEVEL;
            let _: () = msg_send![&panel, setLevel: level];
            let behavior: u64 = CAN_JOIN_ALL_SPACES | STATIONARY | IGNORES_CYCLE | FULL_SCREEN_AUXILIARY;
            let _: () = msg_send![&panel, setCollectionBehavior: behavior];

            // 4. Transparent, click-through, no shadow
            panel.setOpaque(false);
            panel.setBackgroundColor(Some(&NSColor::clearColor()));
            panel.setIgnoresMouseEvents(true);
            panel.setHasShadow(false);

            // 5. Rounded pill background on the content view's backing layer
            if let Some(view) = panel.contentView() {
                view.setWantsLayer(true);
                if let Some(layer) = view.layer() {
                    let radius = 14.0_f64;
                    let _: () = msg_send![&layer, setCornerRadius: radius];
                    // rgba(10, 10, 18, 0.90) — dark navy, matches app palette
                    let bg = NSColor::colorWithRed_green_blue_alpha(
                        0.039, 0.039, 0.071, 0.90_f64,
                    );
                    let cg_color: *mut AnyObject = msg_send![&bg, CGColor];
                    let _: () = msg_send![&layer, setBackgroundColor: cg_color];
                }
            }

            // 6. NSTextField for the phase label
            let tf: Retained<NSTextField> = NSTextField::initWithFrame(
                mtm.alloc(),
                NSRect::new(NSPoint::new(0.0, 18.0), NSSize::new(340.0, 28.0)),
            );
            tf.setEditable(false);
            tf.setSelectable(false);
            tf.setBordered(false);
            tf.setDrawsBackground(false);
            tf.setAlignment(NSTextAlignment::Center);

            // Monospaced system font, medium weight, 14pt
            let font: *mut AnyObject = msg_send![
                NSFont::class(),
                monospacedSystemFontOfSize: 14.0_f64,
                weight: FONT_WEIGHT_MEDIUM
            ];
            let _: () = msg_send![&tf, setFont: font];
            tf.setTextColor(Some(&NSColor::whiteColor()));
            tf.setStringValue(&NSString::from_str("● Recording"));

            if let Some(view) = panel.contentView() {
                view.addSubview(&tf);
            }

            // 7. Leak both into ObjC ownership (they live for the process lifetime)
            let panel_ptr = Retained::as_ptr(&panel) as usize;
            let tf_ptr = Retained::as_ptr(&tf) as usize;
            std::mem::forget(panel);
            std::mem::forget(tf);

            PANEL_PTR.store(panel_ptr, Ordering::Relaxed);
            TEXTFIELD_PTR.store(tf_ptr, Ordering::Relaxed);
        }
    }

    fn show_panel() {
        use objc2::runtime::AnyObject;
        use objc2::msg_send;
        use objc2_app_kit::NSScreen;
        use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};

        let ptr = PANEL_PTR.load(Ordering::Relaxed);
        if ptr == 0 {
            return;
        }

        unsafe {
            let panel = &*(ptr as *const AnyObject);
            let mtm = MainThreadMarker::new_unchecked();

            // Bottom-centre of the main screen (Dock-coordinates, flipped)
            let screen_frame: NSRect = NSScreen::mainScreen(mtm)
                .map(|s| s.frame())
                .unwrap_or(NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    NSSize::new(1440.0, 900.0),
                ));

            let w = 340.0_f64;
            let h = 64.0_f64;
            let x = screen_frame.origin.x + (screen_frame.size.width - w) / 2.0;
            let y = screen_frame.origin.y + 80.0; // 80pt above the Dock area

            let new_frame = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
            let _: () = msg_send![panel, setFrame: new_frame, display: false];
            let _: () = msg_send![panel, orderFrontRegardless];

            refresh_text();
        }
    }

    fn hide_panel() {
        use objc2::runtime::AnyObject;
        use objc2::msg_send;

        let ptr = PANEL_PTR.load(Ordering::Relaxed);
        if ptr == 0 {
            return;
        }
        unsafe {
            let panel = &*(ptr as *const AnyObject);
            let nil: *const AnyObject = std::ptr::null();
            let _: () = msg_send![panel, orderOut: nil];
        }
    }

    /// Translate the current PhaseState into a display string and push it to
    /// the NSTextField. Must be called on the main thread.
    fn refresh_text() {
        use objc2::runtime::AnyObject;
        use objc2::msg_send;
        use objc2_foundation::NSString;

        let tf_ptr = TEXTFIELD_PTR.load(Ordering::Relaxed);
        if tf_ptr == 0 {
            return;
        }

        let st = phase_state().lock().unwrap().clone();
        let text = phase_to_label(&st.phase, st.done_ms, st.engine.as_deref());

        // Empty string → nothing to show, leave the previous label visible
        // until hide() is called.
        if text.is_empty() {
            return;
        }

        unsafe {
            let tf = &*(tf_ptr as *const AnyObject);
            let ns_text = NSString::from_str(&text);
            let _: () = msg_send![tf, setStringValue: &*ns_text];
        }
    }

    /// Convert a phase name + optional latency into the label shown in the pill.
    fn phase_to_label(phase: &str, done_ms: Option<u64>, engine: Option<&str>) -> String {
        let engine_label = match engine.unwrap_or_default() {
            "whisper" => "Whisper",
            "parakeet" => "Parakeet",
            "granite_speech" => "Granite",
            _ => "Taurscribe",
        };
        match phase {
            "recording" => format!("●  {} recording", engine_label),
            "paused" => format!("⏸  {} paused", engine_label),
            "transcribing" => format!("·  ·  ·   {} transcribing", engine_label),
            "correcting" => format!("·  ·  ·   {} correcting", engine_label),
            "done" => match done_ms {
                Some(ms) if ms >= 1000 => format!("✓  {} done  ({:.1}s)", engine_label, ms as f64 / 1000.0),
                Some(ms) => format!("✓  {} done  ({}ms)", engine_label, ms),
                None => format!("✓  {} done", engine_label),
            },
            "cancelled" => "✕  Recording discarded".to_string(),
            "too_short" => "⚠  Too short".to_string(),
            "paste_failed" => "⚠  Couldn't paste".to_string(),
            "no_model" => "⊗  No model loaded".to_string(),
            "model_loading" => "⟳  Model loading...".to_string(),
            _ => String::new(),
        }
    }
}
