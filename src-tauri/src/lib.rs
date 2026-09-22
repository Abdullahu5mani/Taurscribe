// Module declarations
mod audio;
pub mod audio_decode;
pub mod audio_dual_channel;
pub mod audio_preprocess;
pub mod commands;
pub mod context;
pub mod cpu_features;
mod denoise;
pub mod diarization;
pub mod neural_diarizer;
pub mod meeting_continuation;
pub mod mcp_server;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub mod nemotron_diar_mlx;
/// Granite Speech 5 and Qwen3-ASR (GGUF, transcribe.cpp).
pub mod gguf_asr;
mod hotkeys;
pub mod librispeech_wer;
pub mod llm;
pub mod memory;
pub mod meeting_audio;
pub mod meeting_detector;
pub mod meeting_summary;
pub mod control_server;
mod ort_session;
mod overlay;
pub mod platform_tuning;
mod state;
pub mod storage;
pub mod speaker_embedding;
mod system_audio;
pub mod text_injection;
mod tray;
pub mod types;
pub mod utils;
pub mod vad;
mod watcher;
pub mod whisper;

pub use commands::misc::sort_audio_devices_by_priority;

// Imports
use gguf_asr::GgufAsrManager;
use state::AudioState;
use tauri::Manager;
use vad::VADManager;
use whisper::WhisperManager;

/// A boolean from settings.json (written by the frontend's store), if set.
fn saved_setting(key: &str) -> Option<bool> {
    let text = std::fs::read_to_string(mcp_server::settings_path()?).ok()?;
    serde_json::from_str::<serde_json::Value>(&text).ok()?.get(key)?.as_bool()
}

fn focus_main_window(app_handle: &tauri::AppHandle) {
    let windows = app_handle.webview_windows();
    // Prefer "main": on Windows/Linux the overlay is a webview window too.
    if let Some(window) = windows.get("main").or_else(|| windows.values().next()) {
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
        if let Ok(mut granite) = state.granite.lock() {
            granite.unload();
        }
        if let Ok(mut qwen3) = state.qwen3.lock() {
            qwen3.unload();
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
    println!("[INFO] Whisper startup load disabled; model will load on demand");

    // 2. Initialize VAD
    println!("[INFO] Initializing Voice Activity Detection...");
    let vad = VADManager::new().unwrap_or_else(|e| {
        eprintln!("[ERROR] Failed to initialize VAD: {}", e);
        panic!("VAD initialization failed");
    });
    println!("[SUCCESS] VAD initialized successfully");

    // 3. Granite Speech 5 and Qwen3-ASR managers (models load on demand).
    let granite = GgufAsrManager::granite();
    let qwen3 = GgufAsrManager::qwen3();

    // 4. Build the Tauri App
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // This callback is called when a second instance tries to launch.
            // Instead of allowing it, we bring the existing window to the front.
            println!("[INFO] Second instance detected - focusing existing window");

            focus_main_window(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .manage(AudioState::new(whisper, granite, vad, qwen3))
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

            // Older meetings kept their raw WAV; convert them to small playback
            // copies in the background (can take a while for long recordings).
            std::thread::spawn(|| {
                if let Ok(conn) = commands::meetings::open_connection() {
                    let n = commands::meetings::compress_legacy_meeting_audio(&conn);
                    if n > 0 {
                        println!("[INFO] Compressed {} older meeting recording(s) for playback", n);
                    }
                }
            });

            // Log CPU SIMD features for quantized inference dispatch
            cpu_features::log_simd_capabilities();

            // Initialise the native overlay (macOS: creates NSPanel; others: no-op)
            overlay::init(app.handle());
            #[cfg(debug_assertions)]
            overlay::debug_demo(app.handle());

            // Setup System Tray
            // The window starts invisible; the frontend shows it once loaded unless
            // "start hidden" is on. If the frontend never gets that far (a crash
            // or script error), show it anyway so the app is never stuck unseen.
            if saved_setting("start_hidden") != Some(true) {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(15));
                    if !commands::misc::MAIN_WINDOW_REQUESTED.load(std::sync::atomic::Ordering::Relaxed) {
                        eprintln!("[WARN] Frontend never showed the window; showing it now");
                        focus_main_window(&handle);
                    }
                });
            }

            tray::setup_tray(app)?;
            if saved_setting("show_tray_icon") == Some(false) {
                if let Some(tray) = app.tray_by_id("main-tray") {
                    let _ = tray.set_visible(false);
                }
            }

            // Sync initial model state with tray menu item (no model loaded at startup).
            use std::sync::atomic::Ordering;
            app.state::<AudioState>()
                .model_loaded
                .store(false, Ordering::Relaxed);
            tray::update_tray_model_item(app.handle(), false);

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

            storage::allow_recordings_in_asset_scope(app.handle());

            // A kept recording for continuing a meeting outlives its window if the
            // app quit meanwhile; drop it now.
            meeting_continuation::cleanup_expired();

            // Start Automated Meeting Detection Watcher
            let meeting_handle = app.handle().clone();
            let state = app.state::<AudioState>();
            if let Err(e) = state.meeting_detector.start_watching(meeting_handle) {
                eprintln!("[WARN] Failed to start meeting detector: {}", e);
            }

            // Start In-Process Test Control Server (Localhost simulation & test harness)
            control_server::spawn_control_server(app.handle().clone());

            // Start Inactivity Auto-Unload Watchdog Background Thread
            let auto_unload_handle = app.handle().clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    let state = auto_unload_handle.state::<AudioState>();
                    let timeout = state.auto_unload_seconds.load(std::sync::atomic::Ordering::Relaxed);
                    // 0 = never, 1 = immediate (handled synchronously on transcription finish)
                    if timeout <= 1 {
                        continue;
                    }

                    // Only check if an ASR model is loaded
                    if !state.model_loaded.load(std::sync::atomic::Ordering::Relaxed) {
                        continue;
                    }

                    // Guard: Do not unload if recording or engine is actively loading
                    if state.engine_loading.load(std::sync::atomic::Ordering::Relaxed)
                        || state.recording_handle.lock().unwrap().is_some()
                    {
                        continue;
                    }

                    let last = state.last_activity_timestamp.load(std::sync::atomic::Ordering::Relaxed);
                    if last == 0 {
                        continue;
                    }

                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();

                    if now.saturating_sub(last) >= timeout {
                        println!(
                            "[AUTO-UNLOAD] Inactivity timeout reached ({}s). Unloading models to free VRAM...",
                            timeout
                        );
                        if let Ok(unloaded) = state.unload_all_loaded_asr() {
                            if !unloaded.is_empty() {
                                state.last_activity_timestamp.store(0, std::sync::atomic::Ordering::Relaxed);
                                crate::memory::trim_process_memory();
                                crate::tray::reconcile_model_loaded_tray(&auto_unload_handle, &state);
                                use tauri::Emitter;
                                let _ = auto_unload_handle.emit("model-unloaded", ());
                                let _ = auto_unload_handle.emit(
                                    "model-auto-unloaded",
                                    serde_json::json!({
                                        "timeout_seconds": timeout,
                                        "unloaded_engines": unloaded,
                                    }),
                                );
                                let _ = crate::tray::update_tray_icon(&auto_unload_handle, crate::types::AppState::Ready);
                                println!("[AUTO-UNLOAD] Successfully freed VRAM for: {:?}", unloaded);
                            }
                        }
                    }
                }
            });

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
            commands::set_tray_icon_visible,
            commands::get_system_info,
            commands::get_hardware_diagnostics,
            commands::get_process_memory_stats,
            commands::start_recording,
            commands::stop_recording,
            commands::get_backend_info,
            commands::get_engine_selection_state,
            commands::list_models,
            commands::get_current_model,
            commands::switch_model,
            commands::list_granite_models,
            commands::init_granite,
            commands::get_granite_status,
            commands::list_qwen3_models,
            commands::init_qwen3,
            commands::get_qwen3_status,
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
            commands::list_audio_devices,
            commands::get_active_input_device,
            commands::set_input_device,
            commands::show_overlay,
            commands::hide_overlay,
            commands::set_overlay_state,
            storage::get_storage_locations,
            storage::set_storage_location,
            storage::measure_storage_speed,
            storage::open_storage_location,
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
            commands::get_auto_unload_timeout,
            commands::set_auto_unload_timeout,
            commands::get_auto_unload_status,
            commands::touch_activity,
            commands::pause_recording,
            commands::resume_recording,
            commands::cancel_recording,
            commands::transcribe_file,
            commands::finish_file_transcription_batch,
            commands::cancel_file_transcription,
            commands::scan_active_meetings,
            commands::get_meeting_detection_status,
            commands::start_meeting_detection,
            commands::stop_meeting_detection,
            commands::set_audio_source_mode,
            commands::get_audio_source_mode,
            commands::set_auto_record_meetings,
            commands::get_auto_record_meetings,
            commands::get_speaker_match_threshold,
            commands::set_speaker_match_threshold,
            commands::get_meeting_continue_minutes,
            commands::set_meeting_continue_minutes,
            commands::get_mcp_setup,
            commands::list_meetings,
            commands::get_meeting_platform_counts,
            commands::get_meeting_detail,
            commands::update_meeting,
            commands::save_meeting_review,
            commands::delete_meeting,
            commands::rename_speaker,
            commands::rename_vault_speaker,
            commands::list_speaker_vault,
            commands::delete_speaker_from_vault,
            commands::cycle_speaker_turn_snippet,
            commands::cycle_vault_speaker_snippet,
            commands::generate_meeting_summary,
            commands::export_meeting_notes,
            crate::context::get_active_context_preview
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
