/// Platform-aware overlay module.
///
/// macOS  → native non-activating NSPanel (objc2): a frosted capsule with a live
///          waveform, timer and status glyphs. It floats at the status window
///          level so it shows over fullscreen app Spaces.
///
/// Windows → Tauri WebView window "overlay" + emitted "overlay-state" events.
/// Linux    → no overlay window.
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
    #[cfg(target_os = "windows")]
    webview::show(app);
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let _ = app;
}

/// Hide the overlay.
pub fn hide(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    mac::hide(app);
    #[cfg(target_os = "windows")]
    webview::hide(app);
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let _ = app;
}

/// Update the overlay state (phase + optional latency).
/// macOS   → updates NSTextField and egui context directly.
/// Windows → emits "overlay-state" to the "overlay" WebView window.
/// Linux   → no-op.
pub fn set_state(app: &AppHandle, payload: OverlayStatePayload) {
    #[cfg(target_os = "macos")]
    mac::set_state(app, payload);
    #[cfg(target_os = "windows")]
    webview::set_state(app, payload);
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let _ = (app, payload);
}

/// Restores focus to the app that was active when the overlay opened.
pub fn restore_focus(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app;
    #[cfg(target_os = "windows")]
    webview::restore_focus(app);
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let _ = app;
}

/// Feed a microphone level (0..1) to the overlay waveform. The Windows WebView
/// listens to the "audio-level" event itself, so this only drives macOS.
pub fn push_level(app: &AppHandle, level: f32) {
    #[cfg(target_os = "macos")]
    mac::push_level(app, level);
    #[cfg(not(target_os = "macos"))]
    let _ = (app, level);
}

/// Dev builds only: when the file `$TMPDIR/taurscribe-overlay-demo` exists at
/// launch, cycle the overlay through every phase once (for design checks).
#[cfg(debug_assertions)]
pub fn debug_demo(app: &AppHandle) {
    let flag = std::env::temp_dir().join("taurscribe-overlay-demo");
    if !flag.exists() {
        return;
    }
    let _ = std::fs::remove_file(&flag);
    let app = app.clone();
    std::thread::spawn(move || {
        let step = |phase: &str, ms: Option<u64>, secs: u64| {
            set_state(&app, OverlayStatePayload { phase: phase.into(), text: None, ms, engine: None });
            show(&app);
            if phase == "recording" {
                for i in 0..secs * 20 {
                    let t = i as f32 / 20.0;
                    let level = ((t * 3.1).sin().abs() * 0.7 + (t * 7.3).sin().abs() * 0.3).min(1.0);
                    push_level(&app, level);
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            } else {
                std::thread::sleep(std::time::Duration::from_secs(secs));
            }
        };
        std::thread::sleep(std::time::Duration::from_secs(4));
        step("recording", None, 6);
        step("paused", None, 3);
        step("transcribing", None, 3);
        step("done", Some(820), 3);
        step("nothing_heard", None, 3);
        step("paste_failed", None, 3);
        hide(&app);
    });
}

// ── WebView implementation (Windows) ───────────────────────────────────────────

#[cfg(target_os = "windows")]
mod webview {
    use super::OverlayStatePayload;
    use tauri::{AppHandle, Emitter, Manager};

    use std::sync::{Mutex, OnceLock};

    static LAST_FOREGROUND_HWND: OnceLock<Mutex<usize>> = OnceLock::new();

    fn last_foreground_hwnd() -> &'static Mutex<usize> {
        LAST_FOREGROUND_HWND.get_or_init(|| Mutex::new(0))
    }

    pub fn show(app: &AppHandle) {
        if let Some(overlay) = app.get_webview_window("overlay") {
            remember_foreground_window();

            let monitor = active_monitor(app).or_else(|| overlay.primary_monitor().ok().flatten());

            if let Some(m) = monitor {
                let msize = m.size();
                let mpos = m.position();
                let osize = overlay
                    .outer_size()
                    .unwrap_or(tauri::PhysicalSize::new(80, 80));
                let x = mpos.x + ((msize.width as i32 - osize.width as i32) / 2);
                let bottom_margin = (120.0 * m.scale_factor()) as i32;
                let y = mpos.y + msize.height as i32 - osize.height as i32 - bottom_margin;
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
        restore_foreground_window();
    }

    /// Returns the monitor containing the foreground window (the app the user
    /// was typing in when they triggered the hotkey). Falls back to the cursor
    /// position, then to None.
    pub fn active_monitor(app: &AppHandle) -> Option<tauri::Monitor> {
        foreground_monitor(app).or_else(|| cursor_monitor(app))
    }

    /// GetForegroundWindow → MonitorFromWindow → match against Tauri monitors.
    /// This is more accurate than cursor position: the cursor may be parked on
    /// a second screen while the user types on the primary.
    fn foreground_monitor(app: &AppHandle) -> Option<tauri::Monitor> {
        use std::ffi::c_void;

        #[repr(C)]
        struct RECT {
            left: i32,
            top: i32,
            right: i32,
            bottom: i32,
        }

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
            if hwnd.is_null() {
                return None;
            }

            let hmonitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            if hmonitor.is_null() {
                return None;
            }

            let mut info = MONITORINFO {
                cb_size: std::mem::size_of::<MONITORINFO>() as u32,
                rc_monitor: RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
                rc_work: RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
                dw_flags: 0,
            };
            if GetMonitorInfoW(hmonitor, &mut info) == 0 {
                return None;
            }

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
    fn cursor_monitor(app: &AppHandle) -> Option<tauri::Monitor> {
        #[repr(C)]
        struct POINT {
            x: i32,
            y: i32,
        }
        extern "system" {
            fn GetCursorPos(lp: *mut POINT) -> i32;
        }
        let mut pt = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut pt) } == 0 {
            return None;
        }
        let (cx, cy) = (pt.x, pt.y);
        app.available_monitors().ok()?.into_iter().find(|m| {
            let pos = m.position();
            let size = m.size();
            cx >= pos.x
                && cx < pos.x + size.width as i32
                && cy >= pos.y
                && cy < pos.y + size.height as i32
        })
    }

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
    //! A small frosted capsule at the bottom of the screen: a leading status
    //! glyph (pulsing dot, spinner or SF Symbol), a live waveform or a status
    //! label, and a trailing timer. Built from AppKit views and Core Animation
    //! layers inside a non-activating NSPanel, so it floats over fullscreen apps
    //! without taking focus.
    use super::OverlayStatePayload;
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    use tauri::AppHandle;

    const W: f64 = 212.0;
    const H: f64 = 40.0;
    const BAR_COUNT: usize = 15;
    const BAR_W: f64 = 3.0;
    const BAR_GAP: f64 = 3.0;
    const BAR_MIN: f64 = 3.0;
    const BAR_MAX: f64 = 22.0;
    const LEAD_X: f64 = 14.0;
    const BODY_X: f64 = 40.0;
    const TIME_W: f64 = 52.0;
    const BOTTOM_OFFSET: f64 = 88.0;
    const ATTACK: f32 = 0.45;
    const DECAY: f32 = 0.14;

    // NSStatusWindowLevel floats above normal windows; with FullScreenAuxiliary
    // the panel also shows on fullscreen Spaces.
    const STATUS_WINDOW_LEVEL: i64 = 25;
    const CAN_JOIN_ALL_SPACES: u64 = 1 << 0;
    const STATIONARY: u64 = 1 << 4;
    const IGNORES_CYCLE: u64 = 1 << 6;
    const FULL_SCREEN_AUXILIARY: u64 = 1 << 8;

    /// AppKit objects, stored as raw pointers and only touched on the main thread.
    struct Ui {
        panel: usize,
        dot: usize,
        spinner: usize,
        icon: usize,
        label: usize,
        time: usize,
        bar_box: usize,
        bars: [usize; BAR_COUNT],
    }

    static UI: OnceLock<Ui> = OnceLock::new();
    /// Bumped on every show/hide so a delayed orderOut can't hide a newer session.
    static SHOW_GEN: AtomicU64 = AtomicU64::new(0);
    static LAST_TIME_SECS: AtomicUsize = AtomicUsize::new(usize::MAX);

    struct State {
        phase: String,
        done_ms: Option<u64>,
        started: Instant,
        paused_total: Duration,
        pause_started: Option<Instant>,
        levels: [f32; BAR_COUNT],
    }

    fn state() -> &'static Mutex<State> {
        static STATE: OnceLock<Mutex<State>> = OnceLock::new();
        STATE.get_or_init(|| {
            Mutex::new(State {
                phase: String::new(),
                done_ms: None,
                started: Instant::now(),
                paused_total: Duration::ZERO,
                pause_started: None,
                levels: [0.0; BAR_COUNT],
            })
        })
    }

    // ── Public entry points ───────────────────────────────────────────────────

    pub fn init(app: &AppHandle) {
        let _ = app.run_on_main_thread(create_panel);
    }

    pub fn show(app: &AppHandle) {
        let _ = app.run_on_main_thread(show_panel);
    }

    pub fn hide(app: &AppHandle) {
        let gen = SHOW_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = app.run_on_main_thread(fade_out);
        // Order the panel out once the fade has finished, unless it was shown again.
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(220));
            if SHOW_GEN.load(Ordering::SeqCst) == gen {
                let _ = app.run_on_main_thread(order_out);
            }
        });
    }

    pub fn set_state(app: &AppHandle, payload: OverlayStatePayload) {
        {
            let mut st = state().lock().unwrap();
            let prev = std::mem::take(&mut st.phase);
            let now = Instant::now();
            match payload.phase.as_str() {
                "recording" if prev == "paused" => {
                    if let Some(p) = st.pause_started.take() {
                        st.paused_total += now - p;
                    }
                }
                "recording" if prev != "recording" => {
                    st.started = now;
                    st.paused_total = Duration::ZERO;
                    st.pause_started = None;
                    st.levels = [0.0; BAR_COUNT];
                    LAST_TIME_SECS.store(usize::MAX, Ordering::Relaxed);
                }
                "paused" if prev != "paused" => st.pause_started = Some(now),
                _ => {}
            }
            st.phase = payload.phase.clone();
            st.done_ms = payload.ms;
        }
        let _ = app.run_on_main_thread(apply_phase);
    }

    /// Feed one microphone level (0..1, about 20 per second) into the waveform.
    pub fn push_level(app: &AppHandle, level: f32) {
        {
            let mut st = state().lock().unwrap();
            if st.phase != "recording" {
                return;
            }
            // New samples enter at the centre and travel outwards.
            let prev = st.levels;
            let mid = BAR_COUNT / 2;
            let mut next = prev;
            for i in 0..BAR_COUNT {
                let (target, old) = if i == mid {
                    (level.clamp(0.0, 1.0), prev[mid])
                } else if i < mid {
                    (prev[i + 1], prev[i])
                } else {
                    (prev[i - 1], prev[i])
                };
                let alpha = if target > old { ATTACK } else { DECAY };
                next[i] = old + alpha * (target - old);
            }
            st.levels = next;
        }
        let _ = app.run_on_main_thread(refresh_live);
    }

    // ── Helpers (main thread only) ────────────────────────────────────────────

    unsafe fn class(name: &str) -> &'static AnyClass {
        let c = std::ffi::CString::new(name).unwrap();
        AnyClass::get(&c).unwrap_or_else(|| panic!("missing class {name}"))
    }

    unsafe fn obj(ptr: usize) -> &'static AnyObject {
        &*(ptr as *const AnyObject)
    }

    unsafe fn rgba(r: f64, g: f64, b: f64, a: f64) -> *mut AnyObject {
        msg_send![class("NSColor"), colorWithSRGBRed: r, green: g, blue: b, alpha: a]
    }

    unsafe fn cg(color: *mut AnyObject) -> *mut AnyObject {
        msg_send![color, CGColor]
    }

    unsafe fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
    }

    unsafe fn new_view(cls: &str, frame: NSRect) -> *mut AnyObject {
        let v: *mut AnyObject = msg_send![class(cls), alloc];
        msg_send![v, initWithFrame: frame]
    }

    unsafe fn new_label(frame: NSRect, font: *mut AnyObject, color: *mut AnyObject, align: i64) -> *mut AnyObject {
        let tf = new_view("NSTextField", frame);
        let _: () = msg_send![tf, setEditable: false];
        let _: () = msg_send![tf, setSelectable: false];
        let _: () = msg_send![tf, setBordered: false];
        let _: () = msg_send![tf, setDrawsBackground: false];
        let _: () = msg_send![tf, setAlignment: align];
        let _: () = msg_send![tf, setFont: font];
        let _: () = msg_send![tf, setTextColor: color];
        let cell: *mut AnyObject = msg_send![tf, cell];
        let _: () = msg_send![cell, setLineBreakMode: 4_i64]; // truncate tail
        tf
    }

    unsafe fn set_hidden(ptr: usize, hidden: bool) {
        let _: () = msg_send![obj(ptr), setHidden: hidden];
    }

    unsafe fn set_text(ptr: usize, text: &str) {
        let s = NSString::from_str(text);
        let _: () = msg_send![obj(ptr), setStringValue: &*s];
    }

    unsafe fn set_symbol(ptr: usize, name: &str, color: *mut AnyObject) {
        let n = NSString::from_str(name);
        let img: *mut AnyObject = msg_send![
            class("NSImage"),
            imageWithSystemSymbolName: &*n,
            accessibilityDescription: std::ptr::null::<AnyObject>()
        ];
        if img.is_null() {
            return;
        }
        let cfg: *mut AnyObject = msg_send![
            class("NSImageSymbolConfiguration"),
            configurationWithPointSize: 14.0_f64,
            weight: 0.3_f64
        ];
        let img: *mut AnyObject = msg_send![img, imageWithSymbolConfiguration: cfg];
        let _: () = msg_send![obj(ptr), setImage: img];
        let _: () = msg_send![obj(ptr), setContentTintColor: color];
    }

    // ── Construction ──────────────────────────────────────────────────────────

    fn create_panel() {
        if UI.get().is_some() {
            return;
        }
        unsafe {
            // Borderless, non-activating panel: never takes focus from the app
            // being dictated into, and stays up when Taurscribe is inactive.
            let panel: *mut AnyObject = msg_send![class("NSPanel"), alloc];
            let style: u64 = (1 << 7) | 0; // NonactivatingPanel | Borderless
            let panel: *mut AnyObject = msg_send![
                panel,
                initWithContentRect: rect(-2000.0, -2000.0, W, H),
                styleMask: style,
                backing: 2_u64,
                defer: false
            ];
            let _: () = msg_send![panel, setFloatingPanel: true];
            let _: () = msg_send![panel, setBecomesKeyOnlyIfNeeded: true];
            let _: () = msg_send![panel, setHidesOnDeactivate: false];
            let _: () = msg_send![panel, setLevel: STATUS_WINDOW_LEVEL];
            let behavior = CAN_JOIN_ALL_SPACES | STATIONARY | IGNORES_CYCLE | FULL_SCREEN_AUXILIARY;
            let _: () = msg_send![panel, setCollectionBehavior: behavior];
            let _: () = msg_send![panel, setOpaque: false];
            let _: () = msg_send![panel, setBackgroundColor: rgba(0.0, 0.0, 0.0, 0.0)];
            let _: () = msg_send![panel, setIgnoresMouseEvents: true];
            let _: () = msg_send![panel, setHasShadow: false];
            let _: () = msg_send![panel, setAlphaValue: 0.0_f64];
            let dark = NSString::from_str("NSAppearanceNameDarkAqua");
            let appearance: *mut AnyObject = msg_send![class("NSAppearance"), appearanceNamed: &*dark];
            let _: () = msg_send![panel, setAppearance: appearance];

            // Frosted capsule (HUD material) with a dark tint and a hairline edge.
            let effect = new_view("NSVisualEffectView", rect(0.0, 0.0, W, H));
            let _: () = msg_send![effect, setMaterial: 13_i64]; // HUDWindow
            let _: () = msg_send![effect, setBlendingMode: 0_i64]; // behind window
            let _: () = msg_send![effect, setState: 1_i64]; // active
            let _: () = msg_send![effect, setWantsLayer: true];
            let layer: *mut AnyObject = msg_send![effect, layer];
            let _: () = msg_send![layer, setCornerRadius: H / 2.0];
            let _: () = msg_send![layer, setMasksToBounds: true];
            let _: () = msg_send![layer, setBorderWidth: 0.5_f64];
            let _: () = msg_send![layer, setBorderColor: cg(rgba(1.0, 1.0, 1.0, 0.14))];
            let _: () = msg_send![panel, setContentView: effect];

            let tint = new_view("NSView", rect(0.0, 0.0, W, H));
            let _: () = msg_send![tint, setWantsLayer: true];
            let tl: *mut AnyObject = msg_send![tint, layer];
            let _: () = msg_send![tl, setBackgroundColor: cg(rgba(0.04, 0.035, 0.03, 0.55))];
            let _: () = msg_send![effect, addSubview: tint];

            // Leading slot: recording dot (a layer), spinner, or SF Symbol.
            let lead = new_view("NSView", rect(LEAD_X, (H - 16.0) / 2.0, 16.0, 16.0));
            let _: () = msg_send![lead, setWantsLayer: true];
            let _: () = msg_send![effect, addSubview: lead];
            let lead_layer: *mut AnyObject = msg_send![lead, layer];
            let dot: *mut AnyObject = msg_send![class("CALayer"), layer];
            let _: *mut AnyObject = msg_send![dot, retain];
            let _: () = msg_send![dot, setFrame: rect(4.0, 4.0, 8.0, 8.0)];
            let _: () = msg_send![dot, setCornerRadius: 4.0_f64];
            let _: () = msg_send![dot, setBackgroundColor: cg(rgba(1.0, 0.27, 0.23, 1.0))];
            let _: () = msg_send![lead_layer, addSublayer: dot];

            let spinner = new_view("NSProgressIndicator", rect(LEAD_X, (H - 16.0) / 2.0, 16.0, 16.0));
            let _: () = msg_send![spinner, setStyle: 1_u64]; // spinning
            let _: () = msg_send![spinner, setControlSize: 1_u64]; // small
            let _: () = msg_send![spinner, setDisplayedWhenStopped: false];
            let _: () = msg_send![effect, addSubview: spinner];

            let icon = new_view("NSImageView", rect(LEAD_X - 1.0, (H - 18.0) / 2.0, 18.0, 18.0));
            let _: () = msg_send![icon, setImageScaling: 3_u64]; // proportionally up or down
            let _: () = msg_send![effect, addSubview: icon];

            // Body: the waveform while live, a label otherwise.
            let box_w = BAR_COUNT as f64 * BAR_W + (BAR_COUNT - 1) as f64 * BAR_GAP;
            let bar_box = new_view("NSView", rect(BODY_X, 0.0, box_w, H));
            let _: () = msg_send![bar_box, setWantsLayer: true];
            let _: () = msg_send![effect, addSubview: bar_box];
            let box_layer: *mut AnyObject = msg_send![bar_box, layer];
            let mut bars = [0usize; BAR_COUNT];
            for (i, slot) in bars.iter_mut().enumerate() {
                let bar: *mut AnyObject = msg_send![class("CALayer"), layer];
                let _: *mut AnyObject = msg_send![bar, retain];
                let x = i as f64 * (BAR_W + BAR_GAP);
                let _: () = msg_send![bar, setFrame: rect(x, (H - BAR_MIN) / 2.0, BAR_W, BAR_MIN)];
                let _: () = msg_send![bar, setCornerRadius: BAR_W / 2.0];
                let _: () = msg_send![bar, setBackgroundColor: cg(rgba(1.0, 1.0, 1.0, 0.9))];
                let _: () = msg_send![box_layer, addSublayer: bar];
                *slot = bar as usize;
            }

            let body_font: *mut AnyObject = msg_send![class("NSFont"), systemFontOfSize: 13.0_f64, weight: 0.23_f64];
            let label = new_label(
                rect(BODY_X - 2.0, (H - 18.0) / 2.0, W - BODY_X - 14.0, 18.0),
                body_font,
                rgba(1.0, 1.0, 1.0, 0.92),
                0, // left
            );
            let _: () = msg_send![effect, addSubview: label];

            let time_font: *mut AnyObject = msg_send![class("NSFont"), monospacedDigitSystemFontOfSize: 12.0_f64, weight: 0.23_f64];
            let time = new_label(
                rect(W - TIME_W - 14.0, (H - 17.0) / 2.0, TIME_W, 17.0),
                time_font,
                rgba(1.0, 1.0, 1.0, 0.62),
                1, // right
            );
            let _: () = msg_send![effect, addSubview: time];

            let _ = UI.set(Ui {
                panel: panel as usize,
                dot: dot as usize,
                spinner: spinner as usize,
                icon: icon as usize,
                label: label as usize,
                time: time as usize,
                bar_box: bar_box as usize,
                bars,
            });
        }
    }

    // ── Show / hide ───────────────────────────────────────────────────────────

    fn show_panel() {
        let Some(ui) = UI.get() else { return };
        SHOW_GEN.fetch_add(1, Ordering::SeqCst);
        unsafe {
            let panel = obj(ui.panel);
            let screen: *mut AnyObject = msg_send![class("NSScreen"), mainScreen];
            let frame: NSRect = if screen.is_null() {
                rect(0.0, 0.0, 1440.0, 900.0)
            } else {
                msg_send![screen, frame]
            };
            let x = frame.origin.x + (frame.size.width - W) / 2.0;
            let y = frame.origin.y + BOTTOM_OFFSET;
            let visible: bool = msg_send![panel, isVisible];
            let alpha: f64 = msg_send![panel, alphaValue];
            if !visible || alpha < 0.01 {
                // Rise in from slightly below while fading in.
                let _: () = msg_send![panel, setFrame: rect(x, y - 8.0, W, H), display: false];
                let _: () = msg_send![panel, setAlphaValue: 0.0_f64];
                let _: () = msg_send![panel, orderFrontRegardless];
                let _: () = msg_send![class("NSAnimationContext"), beginGrouping];
                let ctx: *mut AnyObject = msg_send![class("NSAnimationContext"), currentContext];
                let _: () = msg_send![ctx, setDuration: 0.22_f64];
                let animator: *mut AnyObject = msg_send![panel, animator];
                let _: () = msg_send![animator, setFrame: rect(x, y, W, H), display: true];
                let _: () = msg_send![animator, setAlphaValue: 1.0_f64];
                let _: () = msg_send![class("NSAnimationContext"), endGrouping];
            } else {
                let _: () = msg_send![panel, setAlphaValue: 1.0_f64];
                let _: () = msg_send![panel, orderFrontRegardless];
            }
        }
        apply_phase();
    }

    fn fade_out() {
        let Some(ui) = UI.get() else { return };
        unsafe {
            let _: () = msg_send![class("NSAnimationContext"), beginGrouping];
            let ctx: *mut AnyObject = msg_send![class("NSAnimationContext"), currentContext];
            let _: () = msg_send![ctx, setDuration: 0.18_f64];
            let animator: *mut AnyObject = msg_send![obj(ui.panel), animator];
            let _: () = msg_send![animator, setAlphaValue: 0.0_f64];
            let _: () = msg_send![class("NSAnimationContext"), endGrouping];
            let _: () = msg_send![obj(ui.spinner), stopAnimation: std::ptr::null::<AnyObject>()];
        }
    }

    fn order_out() {
        let Some(ui) = UI.get() else { return };
        unsafe {
            let _: () = msg_send![obj(ui.panel), orderOut: std::ptr::null::<AnyObject>()];
        }
    }

    // ── Rendering ─────────────────────────────────────────────────────────────

    fn elapsed(st: &State) -> Duration {
        let pause = st.pause_started.map(|p| p.elapsed()).unwrap_or_default();
        st.started.elapsed().saturating_sub(st.paused_total + pause)
    }

    fn format_elapsed(d: Duration) -> String {
        let s = d.as_secs();
        format!("{}:{:02}", s / 60, s % 60)
    }

    fn format_latency(ms: u64) -> String {
        if ms >= 1000 {
            format!("{:.1}s", ms as f64 / 1000.0)
        } else {
            format!("{ms}ms")
        }
    }

    /// Label, leading glyph (SF Symbol name and colour) for non-live phases.
    fn phase_look(phase: &str) -> Option<(&'static str, Option<(&'static str, (f64, f64, f64))>)> {
        const GREEN: (f64, f64, f64) = (0.25, 0.83, 0.55);
        const AMBER: (f64, f64, f64) = (1.0, 0.72, 0.3);
        const GREY: (f64, f64, f64) = (0.62, 0.62, 0.64);
        Some(match phase {
            "transcribing" => ("Transcribing…", None),
            "correcting" => ("Polishing…", None),
            "model_loading" => ("Loading model…", None),
            "done" => ("Pasted", Some(("checkmark.circle.fill", GREEN))),
            "cancelled" => ("Discarded", Some(("xmark.circle.fill", GREY))),
            "too_short" => ("Too short", Some(("exclamationmark.circle.fill", AMBER))),
            "nothing_heard" => ("Nothing heard", Some(("waveform.slash", AMBER))),
            "paste_failed" => ("Couldn't paste", Some(("exclamationmark.circle.fill", AMBER))),
            "no_model" => ("No model loaded", Some(("exclamationmark.circle.fill", AMBER))),
            _ => return None,
        })
    }

    fn apply_phase() {
        let Some(ui) = UI.get() else { return };
        let (phase, done_ms) = {
            let st = state().lock().unwrap();
            (st.phase.clone(), st.done_ms)
        };
        if phase.is_empty() || phase == "hidden" {
            return;
        }
        let live = phase == "recording" || phase == "paused";
        unsafe {
            let dot = obj(ui.dot);
            let _: () = msg_send![class("CATransaction"), begin];
            let _: () = msg_send![class("CATransaction"), setDisableActions: true];
            let _: () = msg_send![dot, setHidden: phase != "recording"];
            let _: () = msg_send![class("CATransaction"), commit];
            let key = NSString::from_str("pulse");
            if phase == "recording" {
                let path = NSString::from_str("opacity");
                let anim: *mut AnyObject = msg_send![class("CABasicAnimation"), animationWithKeyPath: &*path];
                let from: *mut AnyObject = msg_send![class("NSNumber"), numberWithDouble: 0.4_f64];
                let to: *mut AnyObject = msg_send![class("NSNumber"), numberWithDouble: 1.0_f64];
                let _: () = msg_send![anim, setFromValue: from];
                let _: () = msg_send![anim, setToValue: to];
                let _: () = msg_send![anim, setDuration: 0.85_f64];
                let _: () = msg_send![anim, setAutoreverses: true];
                let _: () = msg_send![anim, setRepeatCount: f32::INFINITY];
                let ease = NSString::from_str("easeInEaseOut");
                let tf: *mut AnyObject = msg_send![class("CAMediaTimingFunction"), functionWithName: &*ease];
                let _: () = msg_send![anim, setTimingFunction: tf];
                let _: () = msg_send![dot, addAnimation: anim, forKey: &*key];
            } else {
                let _: () = msg_send![dot, removeAnimationForKey: &*key];
            }

            set_hidden(ui.bar_box, !live);
            set_hidden(ui.time, !(live || phase == "done"));

            let processing = matches!(phase.as_str(), "transcribing" | "correcting" | "model_loading");
            let spinner = obj(ui.spinner);
            if processing {
                let _: () = msg_send![spinner, startAnimation: std::ptr::null::<AnyObject>()];
            } else {
                let _: () = msg_send![spinner, stopAnimation: std::ptr::null::<AnyObject>()];
            }

            if phase == "paused" {
                set_symbol(ui.icon, "pause.fill", rgba(1.0, 0.72, 0.3, 1.0));
                set_hidden(ui.icon, false);
                set_hidden(ui.label, true);
            } else if live {
                set_hidden(ui.icon, true);
                set_hidden(ui.label, true);
            } else if let Some((text, glyph)) = phase_look(&phase) {
                set_text(ui.label, text);
                set_hidden(ui.label, false);
                match glyph {
                    Some((name, (r, g, b))) => {
                        set_symbol(ui.icon, name, rgba(r, g, b, 1.0));
                        set_hidden(ui.icon, false);
                    }
                    None => set_hidden(ui.icon, true),
                }
                if phase == "done" {
                    set_text(ui.time, &done_ms.map(format_latency).unwrap_or_default());
                }
            }
        }
        if live {
            LAST_TIME_SECS.store(usize::MAX, Ordering::Relaxed);
            refresh_live();
        }
    }

    /// Redraw the bars and the timer. Bars ease between levels over one frame
    /// interval so the motion reads as continuous.
    fn refresh_live() {
        let Some(ui) = UI.get() else { return };
        let (levels, elapsed, phase) = {
            let st = state().lock().unwrap();
            (st.levels, elapsed(&st), st.phase.clone())
        };
        let paused = phase == "paused";
        unsafe {
            let _: () = msg_send![class("CATransaction"), begin];
            let _: () = msg_send![class("CATransaction"), setAnimationDuration: 0.09_f64];
            for (i, &bar) in ui.bars.iter().enumerate() {
                let level = if paused { 0.0 } else { levels[i] as f64 };
                let h = BAR_MIN + (BAR_MAX - BAR_MIN) * level;
                let x = i as f64 * (BAR_W + BAR_GAP);
                let layer = obj(bar);
                let _: () = msg_send![layer, setFrame: rect(x, (H - h) / 2.0, BAR_W, h)];
                let _: () = msg_send![layer, setOpacity: if paused { 0.3_f32 } else { 0.55 + 0.45 * level as f32 }];
            }
            let _: () = msg_send![class("CATransaction"), commit];

            let secs = elapsed.as_secs() as usize;
            if LAST_TIME_SECS.swap(secs, Ordering::Relaxed) != secs {
                set_text(ui.time, &format_elapsed(elapsed));
            }
        }
    }
}
