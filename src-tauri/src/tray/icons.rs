use crate::state::AudioState;
use crate::types::AppState;
use std::sync::atomic::Ordering;
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

    // Pick the right hover text.
    // When ready but no model is loaded, surface that in the tooltip so the
    // user can tell at a glance without opening the app.
    let tooltip = match state {
        AppState::Ready => app
            .try_state::<AudioState>()
            .map(|s| {
                let loaded = s.model_loaded.load(Ordering::Relaxed);
                if loaded {
                    "Taurscribe - Ready"
                } else if s.active_engine_has_downloaded_model() {
                    "Taurscribe — No model loaded"
                } else {
                    "Taurscribe — No model found"
                }
            })
            .unwrap_or("Taurscribe - Ready"),
        AppState::Recording => "Taurscribe - Recording...",
        AppState::Processing => "Taurscribe - Processing...",
    };

    // Find the tray item by ID and apply changes
    if let Some(tray) = app.tray_by_id("main-tray") {
        tray.set_icon(Some(icon))
            .map_err(|e| format!("Failed to set tray icon: {}", e))?;
        #[cfg(target_os = "macos")]
        tray.set_icon_as_template(true)
            .map_err(|e| format!("Failed to set icon as template: {}", e))?;
        tray.set_tooltip(Some(tooltip))
            .map_err(|e| format!("Failed to set tooltip: {}", e))?;

        println!("[TRAY] State changed to: {:?}", state);
    }

    Ok(())
}

/// Replaces the tray context menu: "Unload Model" when loaded, "Load Model" when a model
/// exists on disk for the active engine but is not loaded, or a disabled "No model found".
pub fn update_tray_model_item(app: &AppHandle, loaded: bool) {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    let Some(tray) = app.tray_by_id("main-tray") else { return };
    let state = app.state::<AudioState>();
    let has_downloaded = state.active_engine_has_downloaded_model();
    let (model_action_label, model_action_enabled) = if loaded {
        ("Unload Model", true)
    } else if has_downloaded {
        ("Load Model", true)
    } else {
        ("No model found", false)
    };
    let Ok(show_item) = MenuItem::with_id(app, "show", "Show Taurscribe", true, None::<&str>) else { return };
    let Ok(unload_item) =
        MenuItem::with_id(app, "unload", model_action_label, model_action_enabled, None::<&str>)
    else {
        return;
    };
    let Ok(quit_item) = MenuItem::with_id(app, "quit", "Exit", true, None::<&str>) else { return };
    let Ok(separator) = PredefinedMenuItem::separator(app) else { return };
    if let Ok(menu) = Menu::with_items(app, &[&show_item, &unload_item, &separator, &quit_item]) {
        let _ = tray.set_menu(Some(menu));
    }
    let tooltip = if loaded {
        "Taurscribe - Ready"
    } else if has_downloaded {
        "Taurscribe — No model loaded"
    } else {
        "Taurscribe — No model found"
    };
    let _ = tray.set_tooltip(Some(tooltip));
}

/// After a failed load or switch, align `model_loaded` and tray with whichever engine
/// actually holds a model (possibly none). Avoids a stuck "loaded" UI when unload
/// succeeded but the new init failed.
pub fn reconcile_model_loaded_tray(app: &AppHandle, state: &AudioState) {
    let loaded = {
        let w_ok = state
            .whisper
            .lock()
            .map(|g| g.get_current_model().is_some())
            .unwrap_or(false);
        let p_ok = state
            .parakeet
            .lock()
            .map(|g| g.get_status().loaded)
            .unwrap_or(false);
        let c_ok = state
            .cohere
            .lock()
            .map(|g| g.get_status().loaded)
            .unwrap_or(false);
        w_ok || p_ok || c_ok
    };
    state.model_loaded.store(loaded, Ordering::Relaxed);
    update_tray_model_item(app, loaded);
}

fn do_unload(app: &AppHandle) {
    use tauri::Emitter;
    let state = app.state::<AudioState>();

    // Guard: refuse to unload while loading is in progress
    if state.engine_loading.load(Ordering::Relaxed) {
        eprintln!("[TRAY] Unload requested while engine is loading — ignoring");
        return;
    }

    if let Err(e) = state.unload_all_loaded_asr() {
        eprintln!("[TRAY] Unload failed: {e}");
        return;
    }
    reconcile_model_loaded_tray(app, &state);
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

    let builder = TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("Taurscribe - Ready")
        .menu(&menu)
        .show_menu_on_left_click(false);

    #[cfg(target_os = "macos")]
    let builder = builder.icon_as_template(true);

    let _tray = builder
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "unload" => {
                use crate::state::AudioState;
                // If model is loaded → unload it; otherwise open the window so
                // the user can click the Load Model button in the UI.
                let loaded = app.state::<AudioState>().model_loaded.load(Ordering::Relaxed);
                if loaded {
                    do_unload(app);
                } else if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
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

