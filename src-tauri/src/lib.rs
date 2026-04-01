// Module declarations
mod audio;
pub mod audio_decode;
pub mod audio_preprocess;
pub mod cohere;
mod cohere_features;
mod commands;
mod context;
mod denoise;
mod hotkeys;
pub mod librispeech_wer;
mod llm;
pub mod memory;
mod ort_session;
mod overlay;
pub mod parakeet;
pub mod parakeet_loaders;
mod parakeet_runtime;
mod state;
mod system_audio;
mod tray;
mod types;
pub mod utils;
pub mod vad;
mod watcher;
pub mod whisper;

// Imports
use cohere::CohereManager;
use parakeet::ParakeetManager;
use state::AudioState;
use tauri::Manager;
use vad::VADManager;
use whisper::WhisperManager;

fn focus_main_window(app_handle: &tauri::AppHandle) {
    let windows = app_handle.webview_windows();
    if let Some(window) = windows.values().next() {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn cleanup_before_exit(app_handle: &tauri::AppHandle) {
    // Explicitly drop ggml/Metal resources BEFORE exit() runs C++ static
    // destructors. Without this, ggml_metal_device's unique_ptr destructor
    // races with a background dispatch queue that may still be initializing
    // Metal resource sets, causing ggml_abort → SIGABRT on quit.
    println!("[EXIT] App exiting — cleaning up AI engine resources...");
    if let Some(state) = app_handle.try_state::<AudioState>() {
        if let Ok(mut whisper) = state.whisper.lock() {
            whisper.unload();
        }
        if let Ok(mut parakeet) = state.parakeet.lock() {
            parakeet.unload();
        }
        if let Ok(mut cohere) = state.cohere.lock() {
            cohere.unload();
        }
        if let Ok(mut llm) = state.llm.lock() {
            *llm = None;
        }
    }
    // Safety unmute in case the app exits mid-recording
    let _ = system_audio::force_unmute();
    println!("[EXIT] Cleanup complete");
}

/// MAIN ENTRY POINT
/// This is where the app starts!
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Err(e) = commands::perform_pending_factory_reset_on_startup() {
        eprintln!("[RESET] Failed to complete pending factory reset: {}", e);
    }

    match ort_session::initialize_low_ram_ort_environment() {
        Ok(true) => println!("[INFO] ONNX Runtime low-RAM environment configured"),
        Ok(false) => println!("[INFO] ONNX Runtime environment already configured"),
        Err(e) => eprintln!("[WARN] Failed to configure ONNX Runtime environment: {}", e),
    }

    // 1. Create Whisper manager only. The model itself loads lazily on first use.
    println!("[INFO] Initializing Whisper transcription engine manager...");
    let whisper = WhisperManager::new();
    let whisper_loaded_at_startup = false;
    println!("[INFO] Whisper startup load disabled; model will load on demand");

    // 2. Initialize VAD
    println!("[INFO] Initializing Voice Activity Detection...");
    let vad = VADManager::new().unwrap_or_else(|e| {
        eprintln!("[ERROR] Failed to initialize VAD: {}", e);
        panic!("VAD initialization failed");
    });
    println!("[SUCCESS] VAD initialized successfully");

    // 3. Initialize Parakeet & Load Model
    println!("[INFO] Initializing Parakeet ASR manager...");
    let parakeet = ParakeetManager::new();

    // NOTE: Parakeet is NOT lazy-loaded at startup anymore to save VRAM.
    // It will be loaded on demand when the user switches to it.

    // 3b. Initialize Cohere Transcribe (lazy-loaded on demand)
    println!("[INFO] Initializing Cohere Transcribe ASR manager...");
    let cohere = CohereManager::new();

    // 4. Build the Tauri App
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // This callback is called when a second instance tries to launch.
            // Instead of allowing it, we bring the existing window to the front.
            println!("[INFO] Second instance detected - focusing existing window");

            focus_main_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .manage(AudioState::new(whisper, parakeet, vad, cohere))
        .setup(move |app| {
            // Clean up any partial model files left over from a previous download
            // that was interrupted by a crash or force-quit.
            commands::downloader::scan_and_clean_stale_downloads();

            // Safety: if the app crashed mid-recording while system audio was
            // muted, restore it now so the user doesn't start with no sound.
            if let Err(e) = system_audio::force_unmute() {
                eprintln!("[WARN] Safety unmute on startup failed: {}", e);
            } else {
                println!("[INFO] Safety unmute on startup completed");
            }

            // Initialise the native overlay (macOS: creates NSPanel; others: no-op)
            overlay::init(app.handle());

            // Setup System Tray
            tray::setup_tray(app)?;

            // Sync initial model state with tray menu item.
            use std::sync::atomic::Ordering;
            if whisper_loaded_at_startup {
                app.state::<AudioState>()
                    .model_loaded
                    .store(true, Ordering::Relaxed);
                tray::update_tray_model_item(app.handle(), true);
            } else {
                app.state::<AudioState>()
                    .model_loaded
                    .store(false, Ordering::Relaxed);
                tray::update_tray_model_item(app.handle(), false);
            }

            // Start Hotkey Listener in Background Thread
            // Clone the hotkey_config Arc so the listener reacts to config changes immediately.
            let hotkey_config = app.state::<AudioState>().hotkey_config.clone();
            let hotkey_suppressed = app.state::<AudioState>().hotkey_suppressed.clone();
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                hotkeys::start_hotkey_listener(app_handle, hotkey_config, hotkey_suppressed);
            });

            println!("[INFO] Global hotkey listener started (configurable hotkey)");

            // Start File Watcher for Models Directory
            let watcher_handle = app.handle().clone();
            if let Err(e) = watcher::start_models_watcher(watcher_handle) {
                eprintln!("[WARN] Failed to start models watcher: {}", e);
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Check the user's preferred close behavior (persisted in settings.json
                // and applied to AudioState at startup via set_close_behavior command).
                // "tray" (default) → hide to system tray, keep process alive.
                // "quit"           → exit the process immediately.
                let behavior = {
                    let state = window.app_handle().state::<AudioState>();
                    // Explicitly bind the clone so the MutexGuard is dropped
                    // before the block closes (avoiding E0597 borrow error).
                    let b = state.close_behavior.lock().unwrap().clone();
                    b
                };
                if behavior == "quit" {
                    println!("[INFO] Window close → quit (close_behavior=quit)");
                    window.app_handle().exit(0);
                } else {
                    let _ = window.hide();
                    api.prevent_close();
                    // Notify the frontend so it can close the settings modal before the
                    // window is hidden (hotkey won't work while settings is open).
                    use tauri::Emitter;
                    let _ = window.emit("window-hidden", ());
                    println!("[INFO] Window close → hide to tray (close_behavior=tray)");
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::show_main_window,
            commands::get_system_info,
            commands::get_process_memory_stats,
            commands::start_recording,
            commands::stop_recording,
            commands::get_backend_info,
            commands::get_engine_selection_state,
            commands::list_models,
            commands::get_current_model,
            commands::switch_model,
            commands::list_parakeet_models,
            commands::init_parakeet,
            commands::get_parakeet_status,
            commands::set_active_engine,
            commands::get_active_engine,
            commands::set_tray_state,
            commands::check_grammar_llm_available,
            commands::init_llm,
            commands::unload_llm,
            commands::run_llm_inference,
            commands::check_llm_status,
            commands::correct_text,
            commands::type_text,
            commands::save_transcript_history,
            commands::list_transcript_history,
            commands::delete_transcript_history,
            commands::download_model,
            commands::cancel_download,
            commands::get_download_status,
            commands::delete_model,
            commands::get_platform,
            commands::is_apple_silicon,
            commands::get_hotkey,
            commands::set_hotkey,
            commands::set_hotkey_suppressed,
            commands::list_input_devices,
            commands::get_active_input_device,
            commands::set_input_device,
            commands::show_overlay,
            commands::hide_overlay,
            commands::set_overlay_state,
            commands::request_overlay_action,
            commands::mute_system_audio,
            commands::unmute_system_audio,
            commands::check_microphone_permission,
            commands::request_microphone_permission,
            commands::check_accessibility_permission,
            commands::request_accessibility_permission,
            commands::check_input_monitoring_permission,
            commands::request_input_monitoring_permission,
            commands::open_accessibility_settings,
            commands::open_input_monitoring_settings,
            commands::open_microphone_settings,
            commands::open_app_folder,
            commands::unload_current_model,
            commands::relaunch_app,
            commands::factory_reset_app_data,
            commands::get_close_behavior,
            commands::set_close_behavior,
            commands::init_cohere,
            commands::get_cohere_status,
            commands::list_cohere_models,
            commands::pause_recording,
            commands::resume_recording,
            commands::cancel_recording,
            commands::transcribe_file,
            commands::cancel_file_transcription
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            match event {
                #[cfg(target_os = "macos")]
                tauri::RunEvent::Reopen { .. } => {
                    // macOS: clicking the Dock icon when all windows are hidden should
                    // show the main window.
                    focus_main_window(app_handle);
                }
                tauri::RunEvent::Exit => cleanup_before_exit(app_handle),
                _ => {}
            }
        });
}
