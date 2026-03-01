use crate::types::AppState;
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

// Macros to load icon images into the executable at compile time.
// This is faster and safer than loading from disk at runtime.
macro_rules! tray_icon_green {
    () => {
        tauri::include_image!("icons/emoji-green_circle.ico")
    };
}
macro_rules! tray_icon_red {
    () => {
        tauri::include_image!("icons/emoji-red_circle.ico")
    };
}
macro_rules! tray_icon_yellow {
    () => {
        tauri::include_image!("icons/emoji-yellow_circle.ico")
    };
}

/// Helper function to physically change the tray icon
pub fn update_tray_icon(app: &AppHandle, state: AppState) -> Result<(), String> {
    // Pick the right image macro based on state
    let icon = match state {
        AppState::Ready => tray_icon_green!(),
        AppState::Recording => tray_icon_red!(),
        AppState::Processing => tray_icon_yellow!(),
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

/// Setup the system tray icon and menu (called from `setup()` closure)
#[allow(dead_code)]
pub fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};

    let show_item = MenuItem::with_id(app, "show", "Show Taurscribe", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Exit", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show_item, &separator, &quit_item])?;

    let icon = tray_icon_green!();

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
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            use tauri::tray::TrayIconEvent;
            if let TrayIconEvent::Click { .. } = event {
                if let Some(window) = tray.app_handle().get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    println!("[INFO] System tray icon created");
    Ok(())
}

/// Same as `setup_tray` but accepts an `AppHandle` (used for deferred init
/// after the frontend signals it's ready, outside the `setup()` closure).
pub fn setup_tray_from_handle(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};

    let show_item = MenuItem::with_id(app, "show", "Show Taurscribe", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Exit", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show_item, &separator, &quit_item])?;

    let icon = tray_icon_green!();

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
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            use tauri::tray::TrayIconEvent;
            if let TrayIconEvent::Click { .. } = event {
                if let Some(window) = tray.app_handle().get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    println!("[INFO] System tray icon created (deferred)");
    Ok(())
}
