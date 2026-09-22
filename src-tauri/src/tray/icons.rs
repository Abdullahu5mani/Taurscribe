use crate::state::AudioState;
use crate::types::AppState;
use std::sync::atomic::Ordering;
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

/// Sets the tray to `state`, keeping the current call (if any).
pub fn update_tray_icon(app: &AppHandle, state: AppState) -> Result<(), String> {
    let meeting = super::status::current_meeting_full();
    super::status::set(
        app,
        state,
        meeting.as_ref().map(|m| m.0.as_str()),
        meeting.as_ref().and_then(|m| m.1.as_deref()),
        meeting.as_ref().and_then(|m| m.2),
        None,
    )
}

/// Sets the tray to `state` for the given call (None = no call).
pub fn update_tray_icon_with_meeting(
    app: &AppHandle,
    state: AppState,
    meeting_platform: Option<&str>,
    meeting_process: Option<&str>,
    meeting_pid: Option<u32>,
) -> Result<(), String> {
    super::status::set(app, state, meeting_platform, meeting_process, meeting_pid, None)
}

/// Replaces the tray context menu: "Unload Model" when loaded, "Load Model" when a model
/// exists on disk for the active engine but is not loaded, or a disabled "No model found".
/// Keeps any detected call and recording state in the menu, and refreshes the idle
/// tooltip (set at startup, before a model has loaded).
pub fn update_tray_model_item(app: &AppHandle, loaded: bool) {
    let Some(state) = app.try_state::<AudioState>() else {
        return update_tray_menu(app, loaded, None, false);
    };
    let is_recording = state.recording_handle.lock().map(|h| h.is_some()).unwrap_or(false);
    let meeting = state.meeting_detector.get_status().active_meetings.into_iter().next();
    update_tray_menu(app, loaded, meeting.as_ref().map(|m| (m.platform.as_str(), m.pid)), is_recording);
    if !is_recording && meeting.is_none() {
        let _ = super::status::render(app);
    }
}

/// Replaces the tray context menu with meeting and recording metadata.
pub fn update_tray_menu(
    app: &AppHandle,
    loaded: bool,
    meeting_info: Option<(&str, u32)>,
    is_recording: bool,
) {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    let Some(tray) = app.tray_by_id("main-tray") else {
        return;
    };
    let state = app.state::<AudioState>();
    let has_downloaded = state.active_engine_has_downloaded_model();
    let (model_action_label, model_action_enabled) = if loaded {
        ("Unload Model", true)
    } else if has_downloaded {
        ("Load Model", true)
    } else {
        ("No model found", false)
    };

    let Ok(menu) = Menu::new(app) else {
        return;
    };

    // What went wrong, while an error-like state is showing.
    if let Some(line) = super::status::menu_status_line() {
        if let Ok(item) = MenuItem::with_id(app, "status_line", line, false, None::<&str>) {
            let _ = menu.append(&item);
        }
        if let Ok(sep) = PredefinedMenuItem::separator(app) {
            let _ = menu.append(&sep);
        }
    }

    if let Some((plat, pid)) = meeting_info {
        let plat = super::status::platform_display_name(plat);
        let status_label = if is_recording {
            format!("🔴 Recording: {} (PID {})", plat, pid)
        } else {
            format!("🟢 Active Call: {} (PID {})", plat, pid)
        };
        if let Ok(item) = MenuItem::with_id(app, "meeting_status", status_label, false, None::<&str>) {
            let _ = menu.append(&item);
        }
        if !is_recording {
            if let Ok(item) = MenuItem::with_id(app, "meeting_record", format!("Record {} Call", plat), true, None::<&str>) {
                let _ = menu.append(&item);
            }
        }
        if let Ok(sep) = PredefinedMenuItem::separator(app) {
            let _ = menu.append(&sep);
        }
    }

    if let Ok(show_item) = MenuItem::with_id(app, "show", "Show Taurscribe", true, None::<&str>) {
        let _ = menu.append(&show_item);
    }
    if let Ok(unload_item) = MenuItem::with_id(
        app,
        "unload",
        model_action_label,
        model_action_enabled,
        None::<&str>,
    ) {
        let _ = menu.append(&unload_item);
    }
    // Separator above Exit, as in the startup menu (it used to trail after Exit).
    if let Ok(separator) = PredefinedMenuItem::separator(app) {
        let _ = menu.append(&separator);
    }
    if let Ok(quit_item) = MenuItem::with_id(app, "quit", "Exit", true, None::<&str>) {
        let _ = menu.append(&quit_item);
    }

    let _ = tray.set_menu(Some(menu));
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
        let g_ok = state
            .granite
            .lock()
            .map(|g| g.get_status().loaded)
            .unwrap_or(false);
        let q_ok = state
            .qwen3
            .lock()
            .map(|g| g.get_status().loaded)
            .unwrap_or(false);
        w_ok || g_ok || q_ok
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

    let builder = TrayIconBuilder::with_id("main-tray")
        .icon(tauri::include_image!("icons/tray/mac-ready.png"))
        .tooltip("Taurscribe")
        .menu(&menu)
        .show_menu_on_left_click(false);


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
                let loaded = app
                    .state::<AudioState>()
                    .model_loaded
                    .load(Ordering::Relaxed);
                if loaded {
                    do_unload(app);
                } else if let Some(window) = app.get_webview_window("main") {
                    // "Load Model" loads (the window runs the same load as its
                    // Load button); it used to only open the window.
                    use tauri::Emitter;
                    let _ = window.show();
                    let _ = window.set_focus();
                    let _ = window.emit("tray-load-model", ());
                }
            }
            "meeting_record" => {
                use tauri::Emitter;
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                    let _ = window.emit("start-meeting-recording", ());
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
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
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
