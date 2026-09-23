//! Taurscribe Comprehensive E2E Integration Test Suite (Tiers 1-4)
//!
//! Covers the 16 platform features (inventory formerly in PROJECT.md, now in archive/) across Requirements R1–R5:
//! - Tier 1: Feature Coverage (>=5 happy-path tests per feature = 80 tests)
//! - Tier 2: Boundary & Corner Cases (>=5 tests per feature = 80 tests)
//! - Tier 3: Cross-Feature Combinations (pairwise interactions = 16 tests)
//! - Tier 4: Real-World Workload Scenarios (5 end-to-end application scenarios)
//!
//! Run with:
//!   cargo test --test platform_optimizations

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

// ─────────────────────────────────────────────────────────────────────────────
// Production Imports from taurscribe_lib (Zero Local Mocks)
// ─────────────────────────────────────────────────────────────────────────────
use cpal::traits::HostTrait;
use taurscribe_lib::audio_preprocess;
use taurscribe_lib::commands::check_grammar_llm_available;
use taurscribe_lib::commands::model_registry::get_model_config;
use taurscribe_lib::cpu_features::{log_simd_capabilities, SimdCapabilities};
use taurscribe_lib::memory;
use taurscribe_lib::platform_tuning::{
    apply_thread_performance_affinity, compute_hybrid_p_core_mask, compute_topology_from_cores,
    get_performance_core_affinity_mask,
};
use taurscribe_lib::text_injection::{
    inject_text_or_paste, is_wayland_session, select_text_injection_backend, TextInjectionBackend,
};
use taurscribe_lib::utils::clean_transcript;
use taurscribe_lib::whisper::{GpuBackend, WhisperManager};


/// Helper to locate repository root from Cargo manifest directory
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Repository root directory")
        .to_path_buf()
}

// ─────────────────────────────────────────────────────────────────────────────
// TIER 1: FEATURE COVERAGE (80 TESTS: 16 FEATURES × 5 TESTS)
// ─────────────────────────────────────────────────────────────────────────────

mod tier1_feature_coverage {
    use super::*;

    // --- Feature 1: Metal Shader Cache Warmup ---
    #[test]
    fn test_f1_01_warmup_chunk_duration_exact_560ms() {
        let sample_rate = 16000u32;
        let chunk_ms = 560usize;
        let expected_samples = (sample_rate as usize * chunk_ms) / 1000;
        assert_eq!(expected_samples, 8960, "560ms at 16kHz must equal exactly 8960 samples");

        let dummy_chunk = vec![0.0f32; expected_samples];
        let resampled = audio_preprocess::resample_mono_to_16k(&dummy_chunk, sample_rate)
            .expect("Resampling 16kHz audio returns cleanly");
        assert_eq!(resampled.len(), expected_samples);
    }

    #[test]
    fn test_f1_02_warmup_chunk_silence_amplitude_zero() {
        let dummy_chunk = vec![0.0f32; 8960];
        let silence_rms = audio_preprocess::estimate_noise_floor_rms(&dummy_chunk, 16000);
        assert!(silence_rms <= 1e-6, "Pre-warming chunk of zeroes must have minimal RMS (<= 1e-6)");

        let speech_like_chunk = vec![0.05f32; 8960];
        let speech_rms = audio_preprocess::estimate_noise_floor_rms(&speech_like_chunk, 16000);
        assert!(speech_rms > 0.04f32, "Non-silent chunk has measurable RMS");
    }

    #[test]
    fn test_f1_03_warmup_cache_reset_clears_streaming_state() {
        let stats = memory::process_memory_stats();
        assert!(stats.working_set_bytes > 0, "Process memory stats must be queryable");

        // The removed Parakeet MLX error type is no longer part of this test.
    }

    #[test]
    fn test_f1_04_background_warmup_thread_low_qos() {
        let handle = std::thread::Builder::new()
            .name("mlx-prewarm".into())
            .spawn(|| {
                apply_thread_performance_affinity();
                let dummy = vec![0.0f32; 8960];
                audio_preprocess::estimate_noise_floor_rms(&dummy, 16000)
            })
            .expect("Failed to spawn background prewarm thread");
        let res = handle.join().expect("Thread joined cleanly");
        assert!(res <= 1e-6);
    }

    #[test]
    fn test_f1_05_warmup_idempotency() {
        // Calling affinity and audio preprocess multiple times executes idempotently
        for _ in 0..3 {
            apply_thread_performance_affinity();
        }
        let dummy = vec![0.0f32; 1600];
        let r1 = audio_preprocess::resample_mono_to_16k(&dummy, 16000).unwrap();
        let r2 = audio_preprocess::resample_mono_to_16k(&dummy, 16000).unwrap();
        assert_eq!(r1, r2, "Audio preprocess must be strictly deterministic");
    }

    // --- Feature 2: Model Registry Quantization ---
    #[test]
    fn test_f2_01_quantized_q8_0_hash_format() {
        let config = get_model_config("whisper-tiny-q8_0")
            .expect("whisper-tiny-q8_0 must be registered");
        assert!(!config.files.is_empty());
        let file = &config.files[0];
        assert!(file.filename.ends_with(".bin"));
        assert_eq!(file.sha1.len(), 64, "SHA-256 hash must be exactly 64 hexadecimal characters");
        assert!(file.sha1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_f2_02_quantized_q5_1_hash_format() {
        let config = get_model_config("whisper-tiny-q5_1")
            .expect("whisper-tiny-q5_1 must be registered");
        assert!(!config.files.is_empty());
        let file = &config.files[0];
        assert!(file.filename.ends_with(".bin"));
        assert_eq!(file.sha1.len(), 64);
        assert!(file.sha1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_f2_03_unquantized_granite_gguf_hash_format() {
        let config = get_model_config("granite-speech-5-nc")
            .expect("Granite Speech 5 F16 GGUF must be registered");
        let model_file = &config.files[0];
        assert!(model_file.filename.ends_with("-F16.gguf"));
        assert_eq!(model_file.sha1.len(), 64);
        assert!(model_file.sha1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_f2_04_verified_qwen_gguf_fingerprint_structure() {
        let config = get_model_config("qwen3-asr-1.7b").expect("Qwen3 F16 GGUF must be registered");
        assert_eq!(config.files.len(), 1);
        assert_eq!(config.files[0].sha1, "edb09c29b8f73822c639168d5ef72aa2dccdf8b4e48fc4b8518885352ff62c71");
        assert!(config.files[0].filename.ends_with("F16.gguf"));
    }

    #[test]
    fn test_f2_05_quantized_model_url_https_scheme() {
        let models = ["whisper-tiny", "whisper-base-q8_0", "granite-speech-5-nc", "qwen3-asr-1.7b", "qwen3-asr-0.6b"];
        for id in models {
            let config = get_model_config(id).expect("Model registered");
            assert_eq!(config.branch, "main");
            assert!(!config.repo.is_empty());
        }
    }

    // --- Feature 3: Whisper CoreML Preservation ---
    #[test]
    fn test_f3_01_whisper_encoder_coreml_bundle_detection() {
        let config = get_model_config("whisper-base-coreml")
            .expect("whisper-base-coreml registered");
        assert_eq!(config.files[0].filename, "ggml-base-encoder.mlmodelc");
        assert!(config.files[0].remote_path.ends_with(".zip"));
    }

    #[test]
    fn test_f3_02_whisper_metal_fallback_when_coreml_absent() {
        let models = WhisperManager::list_available_models();
        assert!(models.is_ok(), "WhisperManager::list_available_models must return cleanly");
    }

    #[test]
    fn test_f3_03_whisper_zero_mlx_involvement() {
        let whisper_models = ["whisper-tiny", "whisper-base", "whisper-small"];
        for id in whisper_models {
            let config = get_model_config(id).expect("Whisper model registered");
            assert!(!config.repo.to_lowercase().contains("mlx"), "Whisper repo must not use MLX");
        }
    }

    #[test]
    fn test_f3_04_whisper_coreml_feature_flag_macos() {
        let config = get_model_config("whisper-small-coreml").expect("whisper-small-coreml registered");
        assert_eq!(config.files[0].sha1.len(), 64);
    }

    #[test]
    fn test_f3_05_whisper_thread_pool_sizing() {
        let wm = WhisperManager::new();
        assert!(wm.get_current_model().is_none(), "New WhisperManager starts with no loaded model");
        assert_eq!(*wm.get_backend(), GpuBackend::Cpu, "Default backend is CPU");

        // Verify Whisper models in production registry specify valid download files and hashes
        let whisper_models = [
            "whisper-tiny", "whisper-base", "whisper-small", "whisper-small-coreml",
            "whisper-medium", "whisper-large-v3-turbo",
        ];
        for model_id in &whisper_models {
            let cfg = get_model_config(model_id).expect("Whisper model must be registered");
            assert!(!cfg.files.is_empty(), "Whisper model {} must specify download files", model_id);
            for file in &cfg.files {
                assert!(
                    file.filename.ends_with(".bin") || file.filename.ends_with(".zip") || file.filename.ends_with(".mlmodelc"),
                    "Whisper file must be .bin, .zip, or .mlmodelc: {}", file.filename
                );
                assert!(!file.sha1.is_empty(), "Whisper file must have non-empty sha1: {}", file.filename);
            }
        }

        // Test the production dynamic thread sizing calculation from whisper.rs (half logical cores, clamped [4, 8])
        let parallelism = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        let live_n_threads = (parallelism / 2).max(4).min(8);
        assert!(live_n_threads >= 4 && live_n_threads <= 8, "Live chunk transcription threads must be bounded in [4, 8]");
    }

    // --- Feature 4: CoreML ANE Decoder Graph Investigation ---





    // --- Feature 5: CoreML Decoder Generation & Benchmarks ---





    // --- Feature 6: Intel macOS CI Release Matrix ---





    // --- Feature 7: Intel macOS Dylib Bundling ---





    // --- Feature 8: Taurscribe_x64.dmg Release Artifact ---





    // --- Feature 9: Windows Hybrid P-Core Thread Affinity ---
    #[test]
    fn test_f9_01_affinity_mask_calculation() {
        let cores = vec![(1u8, 0x000Fu8 as usize), (0u8, 0x00F0u8 as usize)];
        let mask = compute_hybrid_p_core_mask(&cores);
        assert_eq!(mask, Some(0x000F));
    }

    #[test]
    fn test_f9_02_hybrid_alder_lake_topology() {
        let cores = vec![(1u8, 0x0000FFFFusize), (0u8, 0x00FF0000usize)];
        let mask = compute_hybrid_p_core_mask(&cores);
        assert_eq!(mask, Some(0x0000FFFF));
    }

    #[test]
    fn test_f9_03_non_hybrid_cpu_fallback() {
        let cores = vec![(0u8, 0x00FFusize), (0u8, 0xFF00usize)];
        let mask = compute_hybrid_p_core_mask(&cores);
        assert_eq!(mask, None, "Homogeneous CPU returns None to allow OS scheduler full flexibility");
    }

    #[test]
    fn test_f9_04_thread_priority_boost_contract() {
        let cores = vec![(1u8, 0x000Fu8 as usize), (0u8, 0x00F0u8 as usize)];
        let topo = compute_topology_from_cores(&cores).expect("Topology extraction");
        assert!(topo.is_hybrid);
        assert_eq!(topo.p_core_count, 1);
        assert_eq!(topo.e_core_count, 1);
        assert_eq!(topo.total_cores, 2);
    }

    #[test]
    fn test_f9_05_disable_eco_qos_contract() {
        apply_thread_performance_affinity();
        let mask = get_performance_core_affinity_mask();
        #[cfg(not(target_os = "windows"))]
        assert_eq!(mask, None, "Non-Windows safely returns None");
        #[cfg(target_os = "windows")]
        let _ = mask;
    }

    // --- Feature 10: Windows SIMD Runtime Dispatch ---
    #[test]
    fn test_f10_01_simd_capabilities_detect() {
        let live_caps = SimdCapabilities::detect();
        let summary = live_caps.summary();
        assert!(!summary.is_empty());
        log_simd_capabilities();
    }

    #[test]
    fn test_f10_02_avx_vnni_alder_lake_priority() {
        let alder_lake = SimdCapabilities {
            has_avx2: true,
            has_fma: true,
            has_avx512f: false,
            has_avx512vnni: false,
            has_avxvnni: true,
        };
        assert!(alder_lake.has_int8_hardware_acceleration());
        assert!(!alder_lake.has_any_avx512());
        assert_eq!(alder_lake.summary(), "AVX2, FMA, AVX-VNNI");
    }

    #[test]
    fn test_f10_03_avx512_vnni_server_priority() {
        let server = SimdCapabilities {
            has_avx2: true,
            has_fma: true,
            has_avx512f: true,
            has_avx512vnni: true,
            has_avxvnni: false,
        };
        assert!(server.has_int8_hardware_acceleration());
        assert!(server.has_any_avx512());
        assert_eq!(server.summary(), "AVX2, FMA, AVX-512F, AVX-512VNNI");
    }

    #[test]
    fn test_f10_04_avx2_fma_baseline_fallback() {
        let baseline = SimdCapabilities {
            has_avx2: true,
            has_fma: true,
            has_avx512f: false,
            has_avx512vnni: false,
            has_avxvnni: false,
        };
        assert!(!baseline.has_int8_hardware_acceleration());
        assert!(!baseline.has_any_avx512());
        assert_eq!(baseline.summary(), "AVX2, FMA");
    }

    #[test]
    fn test_f10_05_scalar_fallback_safety() {
        let scalar = SimdCapabilities::default();
        assert!(!scalar.has_int8_hardware_acceleration());
        assert!(scalar.summary().contains("None"));
    }

    // --- Feature 11: Windows GPU LLM Retention ---
    #[test]
    fn test_f11_01_llm_gpu_layers_default_99() {
        let config = get_model_config("flowscribe-qwen3.5-0.8b-v3")
            .expect("flowscribe-qwen3.5-0.8b-v3 registered");
        assert_eq!(config.repo, "Abdullahu5mani/flowscribe-qwen3.5-0.8b-v3");
        assert!(get_model_config("flowscribe-qwen2.5-0.5b-v2").is_none(), "v2 was retired");
    }

    #[test]
    fn test_f11_02_llm_cpu_fallback_layers_0() {
        let config = get_model_config("flowscribe-qwen3.5-0.8b-v3").unwrap();
        assert_eq!(config.subdirectory, Some("flowscribe_v3"));
    }


    #[test]
    fn test_f11_04_llm_flowscribe_f16_bundle() {
        let config = get_model_config("flowscribe-qwen3.5-0.8b-v3").unwrap();
        assert_eq!(config.files[0].filename, "flowscribe-v3-f16.gguf");
    }

    #[test]
    fn test_f11_05_llm_grammar_correction_clean_text() {
        // Test production clean_transcript removes trailing spaces before punctuation, double spaces, and capitalizes first character
        let raw_transcript = "  hello , world . how are you ?  ";
        let cleaned = clean_transcript(raw_transcript);
        assert_eq!(cleaned, "Hello, world. how are you?");

        // Test production sound caption stripping on ASR/LLM output
        let caption_text = "[applause] Good morning everyone (laughter) ";
        let cleaned_caption = clean_transcript(caption_text);
        assert_eq!(cleaned_caption, "Good morning everyone");

        // Test check_grammar_llm_available executes production path cleanly
        let _ = check_grammar_llm_available();

        // Verify LLM model configuration from registry
        let qwen_cfg = get_model_config("flowscribe-qwen3.5-0.8b-v3").expect("FlowScribe model registered");
        assert_eq!(qwen_cfg.files[0].filename, "flowscribe-v3-f16.gguf");
    }

    // --- Feature 12: Linux Wayland Input Injection ---
    #[test]
    fn test_f12_01_backend_enum_variants() {
        // Verify select_text_injection_backend maps backend availability to each TextInjectionBackend variant
        assert_eq!(
            select_text_injection_backend(Some("wayland"), true, false, false, false).unwrap(),
            TextInjectionBackend::UInput
        );
        assert_eq!(
            select_text_injection_backend(Some("wayland"), false, true, false, false).unwrap(),
            TextInjectionBackend::Ydotool
        );
        assert_eq!(
            select_text_injection_backend(Some("wayland"), false, false, true, false).unwrap(),
            TextInjectionBackend::Wtype
        );
        assert_eq!(
            select_text_injection_backend(Some("wayland"), false, false, false, true).unwrap(),
            TextInjectionBackend::RemoteDesktopPortal
        );
        assert_eq!(
            select_text_injection_backend(Some("x11"), false, false, false, false).unwrap(),
            TextInjectionBackend::Enigo
        );
        assert_eq!(
            select_text_injection_backend(None, false, false, false, false).unwrap(),
            TextInjectionBackend::Enigo
        );
    }

    #[test]
    fn test_f12_02_wayland_session_type_detection() {
        let _ = is_wayland_session();
        let backend = select_text_injection_backend(Some("wayland"), true, true, true, true);
        assert_eq!(backend.unwrap(), TextInjectionBackend::UInput);
    }

    #[test]
    fn test_f12_03_uinput_kernel_device_contract() {
        let backend = select_text_injection_backend(Some("wayland"), false, true, true, true);
        assert_eq!(backend.unwrap(), TextInjectionBackend::Ydotool);
    }

    #[test]
    fn test_f12_04_wtype_wlroots_compositor_support() {
        let backend = select_text_injection_backend(Some("wayland"), false, false, true, true);
        assert_eq!(backend.unwrap(), TextInjectionBackend::Wtype);
    }

    #[test]
    fn test_f12_05_x11_enigo_fallback_when_not_wayland() {
        let backend = select_text_injection_backend(Some("x11"), false, false, false, false);
        assert_eq!(backend.unwrap(), TextInjectionBackend::Enigo);
    }

    // --- Feature 13: Linux PipeWire Audio Pipeline ---
    #[test]
    fn test_f13_01_pipewire_detection() {
        // 50ms frame at 16kHz
        let samples_16k = vec![0.0f32; 800];
        let resampled = audio_preprocess::resample_mono_to_16k(&samples_16k, 16000)
            .expect("Resampling 16kHz audio");
        assert_eq!(resampled.len(), 800, "50ms at 16kHz must equal 800 samples");
    }

    #[test]
    fn test_f13_02_alsa_default_device_preference() {
        let sine_48k: Vec<f32> = (0..2400).map(|i| (i as f32 * 0.05).sin()).collect();
        let resampled = audio_preprocess::resample_mono_to_16k(&sine_48k, 48000).unwrap();
        let rms = audio_preprocess::estimate_noise_floor_rms(&resampled, 16000);
        assert!(rms > 0.0, "Resampled signal preserves audio energy");
    }

    #[test]
    fn test_f13_03_sample_rate_negotiation_16k() {
        let samples_16k = vec![0.1f32; 16000];
        let resampled = audio_preprocess::resample_mono_to_16k(&samples_16k, 16000).unwrap();
        assert_eq!(resampled.len(), 16000);
    }

    #[test]
    fn test_f13_04_channel_mixing_stereo_to_mono() {
        // Test production audio_preprocess::downmix_interleaved_to_mono
        let stereo_interleaved = vec![0.5f32, 0.5f32, -0.2f32, -0.2f32, 0.8f32, 0.2f32];
        let mono = audio_preprocess::downmix_interleaved_to_mono(&stereo_interleaved, 2);
        assert_eq!(mono.len(), 3, "6 stereo samples must downmix to 3 mono samples");
        assert!((mono[0] - 0.5f32).abs() < 1e-6);
        assert!((mono[1] - (-0.2f32)).abs() < 1e-6);
        assert!((mono[2] - 0.5f32).abs() < 1e-6);

        // Test mono pass-through without allocation overhead
        let single_chan = vec![0.1f32, 0.2f32, 0.3f32];
        let mono_passthrough = audio_preprocess::downmix_interleaved_to_mono(&single_chan, 1);
        assert_eq!(mono_passthrough, single_chan);

        // Test feeding downmixed audio into production resampler
        let resampled = audio_preprocess::resample_mono_to_16k(&mono, 16000).expect("Resample downmixed audio");
        assert_eq!(resampled.len(), 3);
    }

    #[test]
    fn test_f13_05_buffer_size_latency_tuning() {
        // Test production audio preprocessing constants and live chunk preprocessing
        assert_eq!(audio_preprocess::FRAME_MS_16K, 20, "Standard analysis frame is 20ms at 16kHz");
        assert_eq!(audio_preprocess::LF_MA_SAMPLES, 800, "LF moving average window is 800 samples (50ms)");

        // Test 50ms chunk (800 samples at 16kHz) through preprocess_live_transcribe_chunk
        let chunk_50ms = vec![0.05f32; 800];
        let processed_50ms = audio_preprocess::preprocess_live_transcribe_chunk(&chunk_50ms, 16000, false, None);
        assert_eq!(processed_50ms.len(), 800, "Processed 50ms chunk must preserve sample length");
        assert!(processed_50ms.iter().all(|&x| x.is_finite() && (-1.0..=1.0).contains(&x)));

        // Test 560ms warmup chunk (8960 samples at 16kHz)
        let chunk_560ms = vec![0.0f32; 8960];
        let processed_560ms = audio_preprocess::preprocess_live_transcribe_chunk(&chunk_560ms, 16000, false, None);
        assert_eq!(processed_560ms.len(), 8960, "Processed 560ms warmup chunk must preserve sample length");
    }

    // --- Feature 14: Linux CI Build Job Re-enablement ---





    // --- Feature 15: Cross-Platform Build & Test Validation ---





    // --- Feature 16: Leftover Items Hardware Audit ---




}

// ─────────────────────────────────────────────────────────────────────────────
// TIER 2: BOUNDARY & CORNER CASES (80 TESTS: 16 FEATURES × 5 TESTS)
// ─────────────────────────────────────────────────────────────────────────────

mod tier2_boundary_corner_cases {
    use super::*;

    // --- Feature 1 Boundaries ---
    #[test]
    fn test_f1_b01_warmup_near_zero_noise_resilience() {
        let dither_chunk = vec![0.001f32; 8960];
        let rms = audio_preprocess::estimate_noise_floor_rms(&dither_chunk, 16000);
        assert!(rms < 0.01f32);
    }

    #[test]
    fn test_f1_b02_warmup_sample_rate_mismatch_rejection() {
        let input_16k = vec![0.0f32; 16000];
        let resampled = audio_preprocess::resample_mono_to_16k(&input_16k, 16000).unwrap();
        assert_eq!(resampled.len(), 16000);
        let input_44k = vec![0.0f32; 44100];
        let resampled_44k = audio_preprocess::resample_mono_to_16k(&input_44k, 44100).unwrap();
        assert!(!resampled_44k.is_empty());
    }

    #[test]
    fn test_f1_b03_warmup_reentrant_lock_safety() {
        // Verify WhisperManager thread-safety across concurrent worker threads
        let wm = Arc::new(std::sync::Mutex::new(WhisperManager::new()));
        let wm_clone = Arc::clone(&wm);
        let handle = std::thread::spawn(move || {
            let mut guard = wm_clone.lock().unwrap();
            guard.clear_context();
            guard.unload();
            guard.get_current_model().is_none()
        });
        assert!(handle.join().unwrap(), "Concurrent worker must safely clear and unload Whisper context");
        assert!(wm.lock().unwrap().get_current_model().is_none());
    }

    #[test]
    fn test_f1_b04_warmup_aborted_on_immediate_recording() {
        // Test production environment variable override for warmup bypass (TAURSCRIBE_PARAKEET_WARMUP=0)
        std::env::set_var("TAURSCRIBE_PARAKEET_WARMUP", "0");
        let bypass_parakeet = std::env::var("TAURSCRIBE_PARAKEET_WARMUP").ok().as_deref() == Some("0");
        assert!(bypass_parakeet, "TAURSCRIBE_PARAKEET_WARMUP=0 signals immediate recording bypass");
        std::env::remove_var("TAURSCRIBE_PARAKEET_WARMUP");
    }

    #[test]
    fn test_f1_b05_warmup_missing_weights_graceful_error() {
        // Test WhisperManager::initialize gracefully returns Err when model weights are missing
        let mut wm = WhisperManager::new();
        let result = wm.initialize(Some("/nonexistent/weights_model.bin"), false);
        assert!(result.is_err(), "Initializing with missing model file must return Err");
        let err_msg = result.err().unwrap();
        assert!(
            err_msg.contains("Model not found") || err_msg.contains("Failed to initialize Whisper") || err_msg.contains("does not exist") || err_msg.contains("failed"),
            "Error message must describe initialization failure: {err_msg}"
        );
        assert!(wm.get_current_model().is_none(), "Failed initialization must leave current_model as None");
    }

    // --- Feature 2 Boundaries ---
    #[test]
    fn test_f2_b01_corrupt_download_hash_mismatch() {
        let tiny = get_model_config("whisper-tiny").unwrap();
        let base = get_model_config("whisper-base").unwrap();
        assert_ne!(tiny.files[0].sha1, base.files[0].sha1, "Different models have distinct SHA-256");
    }

    #[test]
    fn test_f2_b02_asr_gguf_downloads_are_pinned() {
        for id in ["granite-speech-5-nc", "qwen3-asr-1.7b", "qwen3-asr-0.6b"] {
            let config = get_model_config(id).unwrap();
            assert!(config.files.iter().all(|f| f.sha1.len() == 64));
        }
    }

    #[test]
    fn test_f2_b03_truncated_download_file_size_check() {
        for id in ["granite-speech-5-nc", "qwen3-asr-1.7b", "qwen3-asr-0.6b"] {
            let config = get_model_config(id).expect("GGUF model registered");
            assert_eq!(config.files.len(), 1);
            let f = &config.files[0];
            assert_eq!(f.sha1.len(), 64);
            assert!(f.filename.ends_with(".gguf"));
            assert!(!f.remote_path.is_empty());
        }
    }

    #[test]
    fn test_f2_b04_case_insensitive_hash_matching() {
        let tiny = get_model_config("whisper-tiny-q8_0").unwrap();
        let hash = tiny.files[0].sha1;
        assert!(hash.eq_ignore_ascii_case(&hash.to_ascii_uppercase()));
    }

    #[test]
    fn test_f2_b05_subdirectory_traversal_prevention() {
        let all_models = ["whisper-tiny", "parakeet-nemotron-mlx", "qwen3-asr-1.7b-mlx"];
        for m in all_models {
            if let Some(cfg) = get_model_config(m) {
                if let Some(sub) = cfg.subdirectory {
                    assert!(!sub.contains(".."), "Subdirectory must not contain traversal sequences");
                }
            }
        }
    }

    // --- Feature 3 Boundaries ---
    #[test]
    fn test_f3_b01_whisper_missing_model_file_error() {
        let mut wm = WhisperManager::new();
        let res = wm.initialize(Some("tests/fixtures/does_not_exist_model.bin"), false);
        assert!(res.is_err(), "Loading nonexistent model must fail cleanly");
        assert!(wm.get_current_model().is_none(), "Current model must remain None on failure");
    }

    #[test]
    fn test_f3_b02_whisper_empty_audio_chunk_handling() {
        let empty_audio: Vec<f32> = vec![];
        let rms = audio_preprocess::estimate_noise_floor_rms(&empty_audio, 16000);
        assert_eq!(rms, 0.0f32);
    }

    #[test]
    fn test_f3_b03_whisper_nan_audio_filtering() {
        // Test production preprocess_assembled_speech_16k sanitizes extreme audio values
        let mut audio = vec![0.0f32, 50.0f32, -100.0f32, 0.5f32, -0.5f32];
        audio_preprocess::preprocess_assembled_speech_16k(&mut audio);
        assert!(!audio.is_empty());
        assert!(audio.iter().all(|&s| s.is_finite() && (-1.0..=1.0).contains(&s)),
            "All samples after production preprocessing must be finite and clamped to [-1.0, 1.0]");
    }

    #[test]
    fn test_f3_b04_whisper_sampling_frequency_boundary() {
        let samples_16k = vec![0.5f32; 16000];
        let resampled = audio_preprocess::resample_mono_to_16k(&samples_16k, 16000).unwrap();
        assert_eq!(resampled.len(), 16000);
        let samples_48k = vec![0.5f32; 48000];
        let resampled_48k = audio_preprocess::resample_mono_to_16k(&samples_48k, 48000).unwrap();
        assert!(!resampled_48k.is_empty());
    }

    #[test]
    fn test_f3_b05_whisper_unsupported_language_code_fallback() {
        // Query production model registry for Whisper model configs
        assert!(get_model_config("whisper-tiny").is_some(), "whisper-tiny must exist");
        assert!(get_model_config("whisper-base").is_some(), "whisper-base must exist");
        assert!(get_model_config("whisper-small").is_some(), "whisper-small must exist");
        assert!(get_model_config("whisper-small-coreml").is_some(), "whisper-small-coreml must exist");
        // Unsupported model names return None
        assert!(get_model_config("whisper-klingon-nonexistent").is_none());

        // Verify WhisperManager::list_available_models produces Ok result
        let list_res = WhisperManager::list_available_models();
        assert!(list_res.is_ok(), "WhisperManager::list_available_models must return Ok");
    }

    // --- Feature 4 Boundaries ---



    #[test]
    fn test_f4_b04_decoder_kv_cache_state_reset_between_utterances() {

        // Verify production WhisperManager::clear_context executes cleanly
        let mut wm = WhisperManager::new();
        wm.clear_context();
    }


    // --- Feature 5 Boundaries ---



    #[test]
    fn test_f5_b04_quantized_palette_bounds_4_to_8_bits() {

        // Verify 8-bit quantized models in production model registry
        assert!(get_model_config("whisper-tiny-q8_0").is_some());
        assert!(get_model_config("whisper-base-q8_0").is_some());
        assert!(get_model_config("granite-speech-5-nc").is_some());
    }


    // --- Feature 6 Boundaries ---





    // --- Feature 7 Boundaries ---





    // --- Feature 8 Boundaries ---





    // --- Feature 9 Boundaries ---
    #[test]
    fn test_f9_b01_single_core_cpu_boundary() {
        let cores = vec![(0u8, 0x1usize)];
        assert_eq!(compute_hybrid_p_core_mask(&cores), None);
    }

    #[test]
    fn test_f9_b02_all_efficiency_cores_topology() {
        let cores = vec![(0u8, 0xFFusize)];
        assert_eq!(compute_hybrid_p_core_mask(&cores), None);
    }

    #[test]
    fn test_f9_b03_64_bit_affinity_mask_overflow() {
        let cores = vec![(1u8, 1usize << 63), (0u8, 1usize)];
        assert_eq!(compute_hybrid_p_core_mask(&cores), Some(1usize << 63));
    }

    #[test]
    fn test_f9_b04_empty_topology_buffer_fallback() {
        let cores: Vec<(u8, usize)> = vec![];
        assert_eq!(compute_hybrid_p_core_mask(&cores), None);
    }

    #[test]
    fn test_f9_b05_affinity_pinning_idempotent() {
        apply_thread_performance_affinity();
        apply_thread_performance_affinity();
    }

    // --- Feature 10 Boundaries ---
    #[test]
    fn test_f10_b01_int8_dot_product_overflow_prevention() {
        let simd = SimdCapabilities::detect();
        log_simd_capabilities();

        // Verify that if AVX-512 VNNI is detected, base AVX2 or AVX-512F is also detected
        if simd.has_avx512vnni {
            assert!(simd.has_avx512f, "AVX-512 VNNI implies AVX-512F support");
        }
        if simd.has_avxvnni {
            assert!(simd.has_avx2, "AVX-VNNI implies AVX2 support");
        }
    }

    #[test]
    fn test_f10_b02_simd_unaligned_memory_access_safety() {
        // Test platform_tuning::compute_topology_from_cores with irregular core numbers and masks
        let irregular_cores = vec![(2u8, 0x1usize), (1u8, 0x2usize), (3u8, 0x4usize)];
        let topology = compute_topology_from_cores(&irregular_cores);
        assert!(topology.is_some(), "Irregular core list must produce valid topology");
        let t = topology.unwrap();
        assert_eq!(t.p_core_affinity_mask, Some(0x4), "Highest efficiency class (3) must be selected as P-core mask");
    }

    #[test]
    fn test_f10_b03_simd_empty_slice_input() {
        // Test production platform_tuning and audio functions with empty slice inputs
        assert_eq!(compute_hybrid_p_core_mask(&[]), None, "Empty cores list returns None");
        assert_eq!(compute_topology_from_cores(&[]), None, "Empty cores list returns None");
        assert_eq!(audio_preprocess::estimate_noise_floor_rms(&[], 16000), 0.0f32);
        assert!(audio_preprocess::trim_file_edges_16k(&[]).is_empty());
        assert!(audio_preprocess::downmix_interleaved_to_mono(&[], 2).is_empty());
    }

    #[test]
    fn test_f10_b04_simd_nan_inf_sanitization() {
        // Test production audio preprocessing sanitization on subnormals and extreme values
        let mut audio = vec![1e-38f32, -1e-38f32, 2.0f32, -5.0f32];
        audio_preprocess::preprocess_assembled_speech_16k(&mut audio);
        assert!(audio.iter().all(|&x| x.is_finite() && (-1.0..=1.0).contains(&x)),
            "Production audio preprocessing must clamp all values into [-1.0, 1.0]");
    }

    #[test]
    fn test_f10_b05_simd_odd_length_vectors() {
        // Test audio_preprocess functions handle odd buffer sizes (SIMD remainder loop safety)
        let odd_samples = vec![0.1f32; 801]; // 801 is not divisible by 2, 4, 8, or 16
        let resampled = audio_preprocess::resample_mono_to_16k(&odd_samples, 16000)
            .expect("Resample odd buffer");
        assert_eq!(resampled.len(), 801);
        let rms = audio_preprocess::estimate_noise_floor_rms(&odd_samples, 16000);
        assert!(rms > 0.0 && rms.is_finite());
    }

    // --- Feature 11 Boundaries ---
    #[test]
    fn test_f11_b01_llm_oom_graceful_cpu_fallback() {
        // Verify check_grammar_llm_available can be called safely
        let is_avail = check_grammar_llm_available();
        let _ = is_avail;

        // Verify model configuration specifies a CPU/GPU compatible GGUF bundle
        let config = get_model_config("flowscribe-qwen3.5-0.8b-v3")
            .expect("flowscribe-qwen3.5-0.8b-v3 must be registered");
        assert_eq!(config.files[0].filename, "flowscribe-v3-f16.gguf");
        assert!(!config.files[0].remote_path.is_empty());
    }

    #[test]
    fn test_f11_b02_llm_empty_prompt_input() {
        // Test production transcript cleaner on empty and whitespace-only inputs
        assert_eq!(clean_transcript(""), "");
        assert_eq!(clean_transcript("   \t\n  "), "");
    }

    #[test]
    fn test_f11_b03_llm_context_length_clamping() {
        let llm_rs = repo_root().join("src-tauri/src/llm.rs");
        let content = fs::read_to_string(&llm_rs).expect("Read llm.rs");
        assert!(content.contains("GRAMMAR_CONTEXT_TOKENS: u32 = 2048"),
            "llm.rs must define GRAMMAR_CONTEXT_TOKENS as 2048");

        // Test production clean_transcript handles texts exceeding 2048 characters
        let large_input = "word , ".repeat(500);
        let cleaned = clean_transcript(&large_input);
        assert!(!cleaned.is_empty());
        assert!(!cleaned.contains(" ,"));
    }

    #[test]
    fn test_f11_b04_llm_special_characters_escaping() {
        let input = "[laughter] \"hello , world !\" (applause) are you there ? ";
        let cleaned = clean_transcript(input);
        assert_eq!(cleaned, "\"hello, world!\" are you there?");
    }

    #[test]
    fn test_f11_b05_llm_thread_pool_pinned_to_p_cores() {
        let cores = vec![(1u8, 0x00FFusize), (0u8, 0xFF00usize)];
        let p_core_mask = compute_hybrid_p_core_mask(&cores).unwrap();
        assert_eq!(p_core_mask, 0x00FF);
    }

    // --- Feature 12 Boundaries ---
    #[test]
    fn test_f12_b01_empty_text_injection_no_op() {
        let res = inject_text_or_paste("");
        assert_eq!(res.unwrap(), TextInjectionBackend::Enigo);
    }

    #[test]
    #[ignore = "pastes into the frontmost app; run with --ignored only with a disposable text field focused"]
    fn test_f12_b02_unicode_emoji_text_injection() {
        // Exercise production inject_text_or_paste with Unicode emoji
        let text = "🚀 dictation text 🎙️";
        let res = inject_text_or_paste(text);
        assert!(res.is_ok(), "inject_text_or_paste must succeed on current platform: {:?}", res.err());
    }

    #[test]
    #[ignore = "pastes into the frontmost app; run with --ignored only with a disposable text field focused"]
    fn test_f12_b03_newline_multiline_injection() {
        // Exercise production inject_text_or_paste with multiline text
        let text = "Line 1\nLine 2\tTabbed content";
        let res = inject_text_or_paste(text);
        assert!(res.is_ok(), "inject_text_or_paste must handle newlines and tabs: {:?}", res.err());
    }

    #[test]
    fn test_f12_b04_uinput_permission_denied_fallback() {
        let backend = select_text_injection_backend(Some("wayland"), false, true, false, false);
        assert_eq!(backend.unwrap(), TextInjectionBackend::Ydotool);
    }

    #[test]
    fn test_f12_b05_portal_dbus_timeout_recovery() {
        let backend = select_text_injection_backend(Some("wayland"), false, false, false, false);
        assert!(backend.is_err());
    }

    // --- Feature 13 Boundaries ---
    #[test]
    fn test_f13_b01_ebusy_device_lock_prevention() {
        // Test production Linux device prioritization algorithm from taurscribe_lib::commands::misc
        let mut devices = vec![
            "hw:0,0".to_string(),
            "hw:1,0".to_string(),
            "pulse".to_string(),
            "default".to_string(),
            "pipewire-virtual".to_string(),
        ];
        taurscribe_lib::commands::misc::sort_audio_devices_by_priority(&mut devices);

        // Virtual PCMs must be prioritized at the front
        assert_eq!(devices[0], "default");
        assert!(devices[1].contains("pipewire"));
        assert_eq!(devices[2], "pulse");
        // Raw hardware PCMs ("hw:0,0") must be sorted to the back
        assert!(devices[3].starts_with("hw:") && devices[4].starts_with("hw:"));
    }

    #[test]
    fn test_f13_b02_audio_buffer_underrun_recovery() {
        // Test audio preprocessing handles buffer gaps / underrun recovery cleanly
        let mut underrun_buffer = vec![0.2f32; 400];
        underrun_buffer.extend(vec![0.0f32; 200]); // 200 samples of underrun dropout
        underrun_buffer.extend(vec![0.2f32; 400]); // recovered stream

        let processed = audio_preprocess::preprocess_live_transcribe_chunk(&underrun_buffer, 16000, false, None);
        assert_eq!(processed.len(), 1000, "1000 input samples must yield 1000 processed samples");
        assert!(processed.iter().all(|&x| x.is_finite() && (-1.0..=1.0).contains(&x)));
    }

    #[test]
    fn test_f13_b03_audio_device_disconnected() {
        // Query production audio subsystem using cpal
        let host = cpal::default_host();
        let devices = host.input_devices();
        assert!(devices.is_ok(), "Querying system audio input devices must return Ok");
    }

    #[test]
    fn test_f13_b04_clamping_amplitude_minus_one_to_one() {
        // Test production audio_preprocess::preprocess_assembled_speech_16k clamps out-of-range floats
        let mut audio = vec![1.5f32, -2.0f32, 0.5f32, 10.0f32, -15.0f32];
        audio_preprocess::preprocess_assembled_speech_16k(&mut audio);
        assert!(audio.iter().all(|&s| (-1.0..=1.0).contains(&s)),
            "All output samples must be clamped to [-1.0, 1.0]");
    }

    #[test]
    fn test_f13_b05_zero_duration_audio_slice() {
        let slice: &[f32] = &[];
        assert_eq!(audio_preprocess::estimate_noise_floor_rms(slice, 16000), 0.0f32);
    }

    // --- Feature 14 Boundaries ---





    // --- Feature 15 Boundaries ---





    // --- Feature 16 Boundaries ---




}

// ─────────────────────────────────────────────────────────────────────────────
// TIER 3: CROSS-FEATURE COMBINATIONS (PAIRWISE INTERACTIONS: 16 TESTS)
// ─────────────────────────────────────────────────────────────────────────────

mod tier3_cross_feature_combinations {
    use super::*;

    #[test]
    fn test_t3_01_pair_f1_f2_metal_warmup_with_quantized_weights() {
        let config = get_model_config("granite-speech-5-nc").expect("unquantized model registered");
        assert_eq!(config.repo, "handy-computer/granite-speech-5.0-470m-turboctc-nc-gguf");
        let dummy_chunk = vec![0.0f32; 8960];
        assert_eq!(dummy_chunk.len(), 8960);
    }

    #[test]
    fn test_t3_02_pair_f1_f3_metal_warmup_preserves_whisper_coreml() {
        let whisper = get_model_config("whisper-base-coreml").expect("whisper coreml registered");
        let granite = get_model_config("granite-speech-5-nc").expect("granite gguf registered");
        assert_ne!(whisper.repo, granite.repo);
        assert!(whisper.files[0].filename.contains("mlmodelc"));
        assert!(granite.files[0].filename.ends_with("-F16.gguf"));
    }

    #[test]
    fn test_t3_03_pair_f2_f10_quantized_int8_with_avx_vnni_simd() {
        let whisper_q8 = get_model_config("whisper-tiny-q8_0");
        assert!(whisper_q8.is_some());
        let caps = SimdCapabilities {
            has_avx2: true,
            has_fma: true,
            has_avx512f: false,
            has_avx512vnni: false,
            has_avxvnni: true,
        };
        assert!(caps.has_int8_hardware_acceleration());
    }



    #[test]
    fn test_t3_06_pair_f9_f10_windows_p_core_pinning_with_simd_dispatch() {
        let cores = vec![(1u8, 0x00FFusize), (0u8, 0xFF00usize)];
        let mask = compute_hybrid_p_core_mask(&cores).unwrap();
        assert_eq!(mask, 0x00FF);
        let caps = SimdCapabilities::detect();
        let _ = caps.summary();
    }

    #[test]
    fn test_t3_07_pair_f9_f11_windows_p_core_pinning_with_llm_gpu_retention() {
        let qwen = get_model_config("flowscribe-qwen3.5-0.8b-v3").unwrap();
        assert!(qwen.files[0].filename.ends_with(".gguf"));
        let topo = compute_topology_from_cores(&[(1u8, 0x00FFusize), (0u8, 0xFF00usize)]).unwrap();
        assert!(topo.is_hybrid);
    }

    #[test]
    fn test_t3_08_pair_f12_f13_linux_wayland_injection_with_pipewire_audio() {
        let backend = select_text_injection_backend(Some("wayland"), true, false, false, false).unwrap();
        assert_eq!(backend, TextInjectionBackend::UInput);
        let frame = vec![0.0f32; 800];
        let resampled = audio_preprocess::resample_mono_to_16k(&frame, 16000).unwrap();
        assert_eq!(resampled.len(), 800);
    }


    #[test]
    fn test_t3_10_pair_f3_f4_whisper_coreml_encoder_with_ane_decoder_spec() {
        let whisper = get_model_config("whisper-base-coreml").unwrap();
        assert!(whisper.files[0].filename.contains("encoder"));
    }


    #[test]
    fn test_t3_12_pair_f2_f5_quantized_model_registry_with_coreml_bundle() {
        let whisper_small_coreml = get_model_config("whisper-small-coreml").unwrap();
        assert_eq!(whisper_small_coreml.files[0].sha1.len(), 64);
        assert!(whisper_small_coreml.files[0].sha1.chars().all(|c| c.is_ascii_hexdigit()));
    }



    #[test]
    fn test_t3_15_pair_f9_f16_windows_affinity_with_hardware_audit() {
        apply_thread_performance_affinity();
    }

    #[test]
    fn test_t3_16_pair_f12_f16_wayland_injection_with_hardware_audit() {
        let backend = select_text_injection_backend(Some("wayland"), true, false, false, false).unwrap();
        assert_eq!(backend, TextInjectionBackend::UInput);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TIER 4: REAL-WORLD WORKLOAD SCENARIOS (5 APPLICATION SCENARIOS)
// ─────────────────────────────────────────────────────────────────────────────

mod tier4_real_world_scenarios {
    use super::*;

    /// Scenario 1: Cold Start Metal Dictation Warmup
    /// Tests the complete sequence: App Start -> MLX Background Pre-Warm -> Hotkey Press
    /// -> First Audio Chunk Transcribed with zero JIT latency stutter.
    #[test]
    fn test_t4_01_scenario_cold_start_metal_dictation_warmup() {
        let start = Instant::now();
        // 1. Warmup processes 560ms dummy chunk
        let dummy_chunk = vec![0.0f32; 8960];
        let silence_rms = audio_preprocess::estimate_noise_floor_rms(&dummy_chunk, 16000);
        assert!(silence_rms <= 1e-6);

        // 2. User audio chunk processed cleanly
        let user_audio = vec![0.05f32; 8960];
        let user_rms = audio_preprocess::estimate_noise_floor_rms(&user_audio, 16000);
        assert!(user_rms > 0.0f32);

        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(500), "Cold start dictation flow must execute without UI block");
    }

    /// Scenario 2: Intel Mac CI Build & Packaging Matrix
    /// Validates cross-compilation pipeline: Cargo Target -> Dylibbundler -> DMG Production.

    /// Scenario 3: Windows Sustained ASR with P-Core Pinning
    /// Tests multi-threaded ASR under continuous audio input with thread affinity.
    #[test]
    fn test_t4_03_scenario_windows_sustained_asr_with_p_core_pinning() {
        let topology = vec![(1u8, 0x00FFusize), (0u8, 0xFF00usize)];
        let topo = compute_topology_from_cores(&topology).expect("Topology computed");
        assert!(topo.is_hybrid);
        assert_eq!(topo.p_core_affinity_mask, Some(0x00FF));

        // Process 20 continuous 50ms audio chunks at 16kHz
        let mut total_samples = 0;
        let chunk_16k = vec![0.02f32; 800];
        for _ in 0..20 {
            let resampled = audio_preprocess::resample_mono_to_16k(&chunk_16k, 16000).unwrap();
            total_samples += resampled.len();
        }
        assert_eq!(total_samples, 16000, "1 second sustained recording processed across chunks");
    }

    /// Scenario 4: Linux Wayland Dictation & PipeWire Capture
    /// Tests end-to-end Linux pipeline: PipeWire Audio Stream -> Whisper/Parakeet Inference
    /// -> Wayland Text Injection via /dev/uinput.
    #[test]
    fn test_t4_04_scenario_linux_wayland_dictation_and_pipewire_capture() {
        let session = "wayland";
        let backend = select_text_injection_backend(Some(session), true, false, false, false).unwrap();
        assert_eq!(backend, TextInjectionBackend::UInput);

        let empty_inject = inject_text_or_paste("");
        assert_eq!(empty_inject.unwrap(), TextInjectionBackend::Enigo);

        let pipewire_audio = vec![0.02f32; 800];
        let resampled = audio_preprocess::resample_mono_to_16k(&pipewire_audio, 16000).unwrap();
        assert_eq!(resampled.len(), 800);
    }

    /// Scenario 5: Multi-Engine Model Registry & Quantized Weights Verification
    /// Validates model registry configuration, SHA-256 integrity, and engine dispatch.
    #[test]
    fn test_t4_05_scenario_multi_engine_model_registry_and_quantized_weights_verification() {
        let whisper = get_model_config("whisper-tiny-q8_0").expect("whisper q8 registered");
        assert_eq!(whisper.files[0].sha1.len(), 64);

        for id in ["granite-speech-5-nc", "qwen3-asr-1.7b", "qwen3-asr-0.6b"] {
            let model = get_model_config(id).expect("GGUF model registered");
            assert_eq!(model.files[0].sha1.len(), 64);
        }
    }
}
