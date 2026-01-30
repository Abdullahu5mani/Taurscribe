// Module declarations
mod audio;
mod commands;
mod hotkeys;
mod llm;
mod parakeet;
mod state;
mod tray;
mod types;
mod utils;
mod vad;
mod whisper;

// Imports
use parakeet::ParakeetManager;
use state::AudioState;
use vad::VADManager;
use whisper::WhisperManager;
use llm::LlmManager;

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
            eprintln!("   Transcription will be disabled.");
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
    let mut parakeet = ParakeetManager::new();

    println!("[INFO] Attempting to auto-load Parakeet model...");
    match parakeet.initialize(Some("nemotron:nemotron")) {
        Ok(msg) => println!("[SUCCESS] {}", msg),
        Err(_) => {
            match parakeet.initialize(None) {
                Ok(msg) => println!("[SUCCESS] Fallback load: {}", msg),
                Err(e) => eprintln!("[WARN] No Parakeet models loaded: {}", e),
            }
        }
    }

    // 4. Initialize LLM (SmolLM2)
    println!("[INFO] Initializing LLM for grammar correction...");
    let mut llm = LlmManager::new();
    match llm.initialize() {
        Ok(msg) => println!("[SUCCESS] {}", msg),
        Err(e) => eprintln!("[WARN] LLM not available: {}", e),
    }

    // 5. Build the Tauri App
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .manage(AudioState::new(whisper, parakeet, vad, llm))
        .setup(|app| {
            // Setup System Tray
            tray::setup_tray(app)?;

            // Start Hotkey Listener in Background Thread
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                hotkeys::start_hotkey_listener(app_handle);
            });

            println!("[INFO] Global hotkey listener started (Ctrl+Win to record)");
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
            commands::start_recording,
            commands::stop_recording,
            commands::get_backend_info,
            commands::benchmark_test,
            commands::list_sample_files,
            commands::list_models,
            commands::get_current_model,
            commands::switch_model,
            commands::list_parakeet_models,
            commands::init_parakeet,
            commands::get_parakeet_status,
            commands::set_active_engine,
            commands::get_active_engine,
            commands::set_tray_state,
            commands::correct_text
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
