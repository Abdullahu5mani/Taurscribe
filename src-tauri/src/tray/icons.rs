use crate::types::AppState;
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

// ---------------------------------------------------------------------------
// Platform-aware tray-icon macros
// ---------------------------------------------------------------------------
// macOS menu-bar icons should be monochrome "template" images so the OS can
// automatically tint them for light / dark mode.  We use small 22×22 PNGs
// with black-on-transparent artwork.
//
// Windows / Linux tray icons can be full-colour.  We use 32×32 coloured
// circle PNGs (green = ready, red = recording, yellow = processing).
// ---------------------------------------------------------------------------

// ── Ready (green / hollow circle) ──────────────────────────────────────────
macro_rules! tray_icon_ready {
    () => {{
        #[cfg(target_os = "macos")]
        {
            tauri::include_image!("icons/tray-readyTemplate@2x.png")
        }
        #[cfg(not(target_os = "macos"))]
        {
            tauri::include_image!("icons/tray-green.png")
        }
    }};
}

// ── Recording (red / filled circle) ────────────────────────────────────────
macro_rules! tray_icon_recording {
    () => {{
        #[cfg(target_os = "macos")]
        {
            tauri::include_image!("icons/tray-recordingTemplate@2x.png")
        }
        #[cfg(not(target_os = "macos"))]
        {
            tauri::include_image!("icons/tray-red.png")
        }
    }};
}

// ── Processing (yellow / circle-with-dot) ──────────────────────────────────
macro_rules! tray_icon_processing {
    () => {{
        #[cfg(target_os = "macos")]
        {
            tauri::include_image!("icons/tray-processingTemplate@2x.png")
        }
        #[cfg(not(target_os = "macos"))]
        {
            tauri::include_image!("icons/tray-yellow.png")
        }
    }};
}

/// Helper function to physically change the tray icon
pub fn update_tray_icon(app: &AppHandle, state: AppState) -> Result<(), String> {
    // Pick the right image macro based on state
    let icon = match state {
        AppState::Ready => tray_icon_ready!(),
        AppState::Recording => tray_icon_recording!(),
        AppState::Processing => tray_icon_processing!(),
    };

    // Pick the right hover text
    let tooltip = match state {
        AppState::Ready => "Taurscribe - Ready",
        AppState::Recording => "Taurscribe - Recording...",
        AppState::Processing => "Taurscribe - Processing...",
    };

    // Find the tray item by ID and apply changes
    if let Some(tray) = app.tray_by_id("main-tray") {
        tray.set_icon(Some(icon))
            .map_err(|e| format!("Failed to set tray icon: {}", e))?;
        tray.set_tooltip(Some(tooltip))
            .map_err(|e| format!("Failed to set tooltip: {}", e))?;

        println!("[TRAY] State changed to: {:?}", state);
    }

    Ok(())
}

fn do_unload(app: &AppHandle) {
    use crate::state::AudioState;
    use crate::types::ASREngine;
    use tauri::Emitter;
    let state = app.state::<AudioState>();
    if let Ok(active) = state.active_engine.lock() {
        match *active {
            ASREngine::Whisper => { if let Ok(mut w) = state.whisper.lock() { w.unload(); } }
            ASREngine::Parakeet => { if let Ok(mut p) = state.parakeet.lock() { p.unload(); } }
            ASREngine::GraniteSpeech => { if let Ok(mut g) = state.granite_speech.lock() { g.unload(); } }
        }
    }
    let _ = app.emit("model-unloaded", ());
    let _ = crate::tray::update_tray_icon(app, AppState::Ready);
}

/// Setup the system tray icon and menu (called from `setup()` closure)
#[allow(dead_code)]
pub fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};

    let show_item = MenuItem::with_id(app, "show", "Show Taurscribe", true, None::<&str>)?;
    let unload_item = MenuItem::with_id(app, "unload", "Unload Model", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Exit", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show_item, &unload_item, &separator, &quit_item])?;

    let icon = tray_icon_ready!();

    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("Taurscribe - Ready")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "unload" => do_unload(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
            use tauri::Emitter;
            // Only open the window on left-click. Right-click is handled by
            // the context menu (.show_menu_on_left_click(false) already gates
            // menu display, but we must not intercept right-click here or
            // Windows never shows the menu).
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                if let Some(window) = tray.app_handle().get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                    let _ = window.emit("window-restored", ());
                }
            }
        })
        .build(app)?;

    println!("[INFO] System tray icon created");
    Ok(())
}

/// Same as `setup_tray` but accepts an `AppHandle` (used for deferred init
/// after the frontend signals it's ready, outside the `setup()` closure).
#[allow(dead_code)]
pub fn setup_tray_from_handle(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};

    let show_item = MenuItem::with_id(app, "show", "Show Taurscribe", true, None::<&str>)?;
    let unload_item = MenuItem::with_id(app, "unload", "Unload Model", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Exit", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show_item, &unload_item, &separator, &quit_item])?;

    let icon = tray_icon_ready!();

    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("Taurscribe - Ready")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "unload" => do_unload(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
            use tauri::Emitter;
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                if let Some(window) = tray.app_handle().get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                    let _ = window.emit("window-restored", ());
                }
            }
        })
        .build(app)?;

    println!("[INFO] System tray icon created (deferred)");
    Ok(())
}
