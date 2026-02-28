// Module declarations
mod audio;
mod commands;
mod hotkeys;
mod llm;
mod parakeet;
mod parakeet_loaders;
mod spellcheck;
mod state;
mod tray;
mod types;
mod utils;
mod vad;
mod watcher;
mod whisper;

// Imports
use parakeet::ParakeetManager;
use state::AudioState;
use tauri::Manager;
use vad::VADManager;
use whisper::WhisperManager;

/// MAIN ENTRY POINT
/// This is where the app starts!
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 1. Initialize Whisper AI
    println!("[INFO] Initializing Whisper transcription engine...");
    let whisper = WhisperManager::new();

    let (whisper, init_result) = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024) // 8 MiB Stack
        .spawn(move || {
            let mut whisper = whisper;
            let res = whisper.initialize(None);
            (whisper, res)
        })
        .expect("Failed to spawn whisper init thread")
        .join()
        .expect("Whisper init thread panicked");

    match init_result {
        Ok(backend_msg) => {
            println!("[SUCCESS] {}", backend_msg);
        }
        Err(e) => {
            eprintln!("[ERROR] Failed to initialize Whisper: {}", e);
            eprintln!("   ⚠️  No models found. Please download the Base model from Settings > Download Manager.");
            eprintln!("   Transcription will be disabled until a model is downloaded.");
        }
    }

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

    // 4. Build the Tauri App
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .manage(AudioState::new(whisper, parakeet, vad))
        .setup(|app| {
            // Setup System Tray
            tray::setup_tray(app)?;

            // Start Hotkey Listener in Background Thread
            // Clone the hotkey_config Arc so the listener reacts to config changes immediately.
            let hotkey_config = app.state::<AudioState>().hotkey_config.clone();
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                hotkeys::start_hotkey_listener(app_handle, hotkey_config);
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
                let _ = window.hide();
                api.prevent_close();
                println!("[INFO] Window minimized to tray");
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::greet,
            commands::get_system_info,
            commands::start_recording,
            commands::stop_recording,
            commands::get_backend_info,
            commands::list_models,
            commands::get_current_model,
            commands::switch_model,
            commands::list_parakeet_models,
            commands::init_parakeet,
            commands::get_parakeet_status,
            commands::set_active_engine,
            commands::get_active_engine,
            commands::set_tray_state,
            commands::init_llm,
            commands::unload_llm,
            commands::run_llm_inference,
            commands::check_llm_status,
            commands::correct_text,
            commands::type_text,
            commands::init_spellcheck,
            commands::unload_spellcheck,
            commands::check_spellcheck_status,
            commands::correct_spelling,
            commands::download_model,
            commands::get_download_status,
            commands::delete_model,
            commands::verify_model_hash,
            commands::get_platform,
            commands::get_hotkey,
            commands::set_hotkey,
            commands::list_input_devices,
            commands::get_input_device,
            commands::set_input_device
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
