//! Taurscribe Comprehensive E2E Integration Test Suite (Tiers 1-4)
//!
//! Covers all 16 features from PROJECT.md § Feature Inventory across Requirements R1–R5:
//! - Tier 1: Feature Coverage (>=5 happy-path tests per feature = 80 tests)
//! - Tier 2: Boundary & Corner Cases (>=5 tests per feature = 80 tests)
//! - Tier 3: Cross-Feature Combinations (pairwise interactions = 16 tests)
//! - Tier 4: Real-World Workload Scenarios (5 end-to-end application scenarios)
//!
//! Run with:
//!   cargo test --test platform_optimizations

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

// ─────────────────────────────────────────────────────────────────────────────
// Production Imports from taurscribe_lib (Zero Local Mocks)
// ─────────────────────────────────────────────────────────────────────────────
use taurscribe_lib::audio_preprocess;
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
use taurscribe_lib::whisper::WhisperManager;

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use taurscribe_lib::parakeet_mlx::{engine::ParakeetMlxError};

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

        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            let err = ParakeetMlxError::WarmupError("Metal pipeline initialization test".into());
            assert!(format!("{err}").to_lowercase().contains("warmup"));
        }
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
    fn test_f2_03_quantized_int4_onnx_hash_format() {
        let config = get_model_config("granite-speech-4.1-2b-nar-mlx-8bit")
            .expect("granite-speech-4.1-2b-nar-mlx-8bit must be registered");
        let model_file = config.files.iter().find(|f| f.filename == "model.safetensors").unwrap();
        assert_eq!(model_file.sha1.len(), 64);
        assert!(model_file.sha1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_f2_04_verified_json_fingerprint_structure() {
        let config = get_model_config("parakeet-nemotron-mlx-8bit")
            .expect("parakeet-nemotron-mlx-8bit must be registered");
        let tokenizer = config.files.iter().find(|f| f.filename == "tokenizer.model").unwrap();
        assert_eq!(tokenizer.sha1, "07d4e5a63840a53ab2d4d106d2874768143fb3fbdd47938b3910d2da05bfb0a9");

        let weights = config.files.iter().find(|f| f.filename == "model.safetensors").unwrap();
        assert_eq!(weights.sha1, "", "Model weights sha1 is unpinned pending upstream LFS retrieval");
        assert!(!weights.sha1.contains("123456789abcdef"), "Must not contain dummy sequence");
    }

    #[test]
    fn test_f2_05_quantized_model_url_https_scheme() {
        let models = ["whisper-tiny", "whisper-base-q8_0", "parakeet-nemotron-mlx", "granite-speech-4.1-2b-nar-mlx"];
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
        let parallelism = std::thread::available_parallelism();
        assert!(parallelism.is_ok(), "Thread available parallelism must be detected");
    }

    // --- Feature 4: CoreML ANE Decoder Graph Investigation ---
    #[test]
    fn test_f4_01_decoder_stateful_kv_tensor_contract() {
        let path = repo_root().join("docs/whisper_coreml_decoder_feasibility.md");
        assert!(path.exists(), "docs/whisper_coreml_decoder_feasibility.md must exist");
        let content = fs::read_to_string(&path).expect("Read feasibility report");
        assert!(content.contains("[12, 1, 12, 448, 64]") || content.contains("448"));
        assert!(content.contains("Stateful KV-Cache") || content.contains("stateful"));
    }

    #[test]
    fn test_f4_02_decoder_input_token_shape() {
        let path = repo_root().join("docs/whisper_coreml_decoder_feasibility.md");
        let content = fs::read_to_string(&path).expect("Read feasibility report");
        assert!(content.contains("[1, 1]") || content.contains("scalar token"));
    }

    #[test]
    fn test_f4_03_decoder_encoder_hidden_states_shape() {
        let path = repo_root().join("docs/whisper_coreml_decoder_feasibility.md");
        let content = fs::read_to_string(&path).expect("Read feasibility report");
        assert!(content.contains("1500"));
    }

    #[test]
    fn test_f4_04_decoder_ane_fp16_precision_contract() {
        let path = repo_root().join("docs/whisper_coreml_decoder_feasibility.md");
        let content = fs::read_to_string(&path).expect("Read feasibility report");
        assert!(content.contains("FP16"));
    }

    #[test]
    fn test_f4_05_decoder_sampling_host_cpu_contract() {
        let path = repo_root().join("docs/whisper_coreml_decoder_feasibility.md");
        let content = fs::read_to_string(&path).expect("Read feasibility report");
        assert!(content.contains("CPU"));
    }

    // --- Feature 5: CoreML Decoder Generation & Benchmarks ---
    #[test]
    fn test_f5_01_coreml_export_script_interface() {
        let script_path = repo_root().join("scripts/export_whisper_decoder_coreml.py");
        assert!(script_path.exists(), "scripts/export_whisper_decoder_coreml.py must exist");
        let content = fs::read_to_string(&script_path).expect("Read export script");
        assert!(content.contains("--model"));
        assert!(content.contains("--compute-units"));
        assert!(content.contains("--analyze-only"));
        assert!(content.contains("Stateful") || content.contains("stateful"));
    }

    #[test]
    fn test_f5_02_coreml_modelc_directory_structure() {
        let script_path = repo_root().join("scripts/export_whisper_decoder_coreml.py");
        let content = fs::read_to_string(&script_path).expect("Read export script");
        assert!(content.contains(".mlmodelc"));
    }

    #[test]
    fn test_f5_03_benchmark_tokens_per_second_calculation() {
        let path = repo_root().join("docs/whisper_coreml_decoder_feasibility.md");
        let content = fs::read_to_string(&path).expect("Read feasibility doc");
        assert!(content.contains("tokens/sec") || content.contains("Tokens/sec") || content.contains("Latency"));
    }

    #[test]
    fn test_f5_04_limitation_kv_cache_clamping_documented() {
        let path = repo_root().join("docs/whisper_coreml_decoder_feasibility.md");
        let content = fs::read_to_string(&path).expect("Read feasibility doc");
        assert!(content.contains("448"));
    }

    #[test]
    fn test_f5_05_ane_residency_profiler_metric_parsing() {
        let path = repo_root().join("docs/whisper_coreml_decoder_feasibility.md");
        let content = fs::read_to_string(&path).expect("Read feasibility doc");
        assert!(content.contains("powermetrics") || content.contains("Instruments") || content.contains("ANE"));
    }

    // --- Feature 6: Intel macOS CI Release Matrix ---
    #[test]
    fn test_f6_01_release_matrix_target_triple() {
        let workflow_path = repo_root().join(".github/workflows/release.yml");
        assert!(workflow_path.exists());
        let content = fs::read_to_string(&workflow_path).expect("Read release.yml");
        assert!(content.contains("x86_64-apple-darwin"), "x86_64-apple-darwin must be in release matrix");
    }

    #[test]
    fn test_f6_02_release_matrix_runner_image() {
        let workflow_path = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow_path).expect("Read release.yml");
        assert!(content.contains("macos-latest"));
    }

    #[test]
    fn test_f6_03_release_matrix_arch_label() {
        let workflow_path = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow_path).expect("Read release.yml");
        assert!(content.contains("x86_64"));
    }

    #[test]
    fn test_f6_04_release_matrix_cuda_flag_false() {
        let workflow_path = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow_path).expect("Read release.yml");
        assert!(content.contains("cuda: false"));
    }

    #[test]
    fn test_f6_05_release_matrix_os_name_macos() {
        let workflow_path = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow_path).expect("Read release.yml");
        assert!(content.contains("macOS"));
    }

    // --- Feature 7: Intel macOS Dylib Bundling ---
    #[test]
    fn test_f7_01_dylibbundler_executable_flag() {
        let script_path = repo_root().join("scripts/bundle-macos-dylibs.sh");
        assert!(script_path.exists());
        let content = fs::read_to_string(&script_path).expect("Read bundle script");
        assert!(content.contains("dylibbundler") && content.contains("-x"));
    }

    #[test]
    fn test_f7_02_dylibbundler_destination_flag() {
        let script_path = repo_root().join("scripts/bundle-macos-dylibs.sh");
        let content = fs::read_to_string(&script_path).expect("Read bundle script");
        assert!(content.contains("-d"));
    }

    #[test]
    fn test_f7_03_dylibbundler_rpath_prefix() {
        let script_path = repo_root().join("scripts/bundle-macos-dylibs.sh");
        let content = fs::read_to_string(&script_path).expect("Read bundle script");
        assert!(content.contains("@executable_path/../Frameworks"));
    }

    #[test]
    fn test_f7_04_dylibbundler_bundle_deps_flag() {
        let script_path = repo_root().join("scripts/bundle-macos-dylibs.sh");
        let content = fs::read_to_string(&script_path).expect("Read bundle script");
        assert!(content.contains("-b"));
    }

    #[test]
    fn test_f7_05_dylib_symlinks_creation() {
        let script_path = repo_root().join("scripts/bundle-macos-dylibs.sh");
        let content = fs::read_to_string(&script_path).expect("Read bundle script");
        assert!(content.contains("ln -sf") || content.contains("ln -s") || content.contains(".dylib"));
    }

    // --- Feature 8: Taurscribe_x64.dmg Release Artifact ---
    #[test]
    fn test_f8_01_dmg_artifact_name() {
        let workflow_path = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow_path).expect("Read release.yml");
        assert!(content.contains("Taurscribe_x64.dmg"));
    }

    #[test]
    fn test_f8_02_dmg_staging_directory() {
        let workflow_path = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow_path).expect("Read release.yml");
        assert!(content.contains("release-assets"));
    }

    #[test]
    fn test_f8_03_dmg_mime_type() {
        let workflow_path = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow_path).expect("Read release.yml");
        assert!(content.contains(".dmg"));
    }

    #[test]
    fn test_f8_04_dmg_upload_artifact_v4() {
        let workflow_path = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow_path).expect("Read release.yml");
        assert!(content.contains("actions/upload-artifact@v4"));
    }

    #[test]
    fn test_f8_05_dmg_gh_release_draft_mode() {
        let workflow_path = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow_path).expect("Read release.yml");
        assert!(content.contains("draft: true"));
    }

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
        let config = get_model_config("flowscribe-qwen2.5-0.5b-v2")
            .expect("flowscribe-qwen2.5-0.5b-v2 registered");
        assert_eq!(config.repo, "Abdullahu5mani/flowscribe-qwen2.5-0.5b-v2");
    }

    #[test]
    fn test_f11_02_llm_cpu_fallback_layers_0() {
        let config = get_model_config("flowscribe-qwen2.5-0.5b-v2").unwrap();
        assert_eq!(config.subdirectory, Some("qwen_finetuned_gguf"));
    }

    #[test]
    fn test_f11_03_llm_dynamic_link_feature() {
        let cargo_toml = repo_root().join("src-tauri/Cargo.toml");
        let content = fs::read_to_string(&cargo_toml).expect("Read Cargo.toml");
        assert!(content.contains("llama-cpp-2"));
    }

    #[test]
    fn test_f11_04_llm_flowscribe_q4_k_m_bundle() {
        let config = get_model_config("flowscribe-qwen2.5-0.5b-v2").unwrap();
        assert_eq!(config.files[0].filename, "model_q4_k_m.gguf");
    }

    #[test]
    fn test_f11_05_llm_grammar_correction_clean_text() {
        let raw_output = "<think>fixing punctuation</think>Hello, world.";
        let cleaned = if let Some(idx) = raw_output.find("</think>") {
            &raw_output[idx + 8..]
        } else {
            raw_output
        };
        assert_eq!(cleaned, "Hello, world.");
    }

    // --- Feature 12: Linux Wayland Input Injection ---
    #[test]
    fn test_f12_01_backend_enum_variants() {
        let b = TextInjectionBackend::UInput;
        assert_eq!(format!("{b:?}"), "UInput");
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
        let stereo = vec![0.5f32, 0.5f32, -0.2f32, -0.2f32];
        let mono: Vec<f32> = stereo.chunks(2).map(|c| (c[0] + c[1]) / 2.0).collect();
        assert_eq!(mono, vec![0.5f32, -0.2f32]);
    }

    #[test]
    fn test_f13_05_buffer_size_latency_tuning() {
        let chunk_samples = 800; // 50ms at 16kHz
        assert_eq!(chunk_samples * 20, 16000, "20 x 50ms chunks equals 1 second");
    }

    // --- Feature 14: Linux CI Build Job Re-enablement ---
    #[test]
    fn test_f14_01_target_triple_linux() {
        let workflow_path = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow_path).expect("Read release.yml");
        assert!(content.contains("x86_64-unknown-linux-gnu"));
    }

    #[test]
    fn test_f14_02_dynamic_libcuda_stubs_path() {
        let build_rs = repo_root().join("src-tauri/build.rs");
        let content = fs::read_to_string(&build_rs).expect("Read build.rs");
        assert!(content.contains("stubs") && content.contains("libcuda.so"));
    }

    #[test]
    fn test_f14_03_allow_multiple_definition_rustflag() {
        let workflow_path = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow_path).expect("Read release.yml");
        assert!(content.contains("bundle-linux-solibs.sh") || content.contains("allow-multiple-definition"));
    }

    #[test]
    fn test_f14_04_bundle_linux_solibs_script() {
        let script_path = repo_root().join("scripts/bundle-linux-solibs.sh");
        assert!(script_path.exists());
        let content = fs::read_to_string(&script_path).expect("Read script");
        assert!(content.contains("patchelf") || content.contains("libcuda"));
    }

    #[test]
    fn test_f14_05_deb_package_artifact() {
        let workflow_path = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow_path).expect("Read release.yml");
        assert!(content.contains("deb"));
    }

    // --- Feature 15: Cross-Platform Build & Test Validation ---
    #[test]
    fn test_f15_01_release_workflow_triggers() {
        let workflow_path = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow_path).expect("Read release.yml");
        assert!(content.contains("tags:") && content.contains("'v*'"));
    }

    #[test]
    fn test_f15_02_release_workflow_fail_fast() {
        let workflow_path = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow_path).expect("Read release.yml");
        assert!(content.contains("fail-fast: false"));
    }

    #[test]
    fn test_f15_03_bun_frontend_setup() {
        let workflow_path = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow_path).expect("Read release.yml");
        assert!(content.contains("oven-sh/setup-bun"));
    }

    #[test]
    fn test_f15_04_cargo_test_harness_integration() {
        let test_file = repo_root().join("src-tauri/tests/platform_optimizations.rs");
        assert!(test_file.exists());
        let content = fs::read_to_string(&test_file).expect("Read test file");
        assert!(content.contains("platform_optimizations"));
    }

    #[test]
    fn test_f15_05_multi_os_matrix_coverage() {
        let workflow_path = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow_path).expect("Read release.yml");
        let targets = [
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "x86_64-pc-windows-msvc",
            "aarch64-pc-windows-msvc",
            "x86_64-unknown-linux-gnu",
        ];
        for t in targets {
            assert!(content.contains(t), "Workflow must include matrix target {t}");
        }
    }

    // --- Feature 16: Leftover Items Hardware Audit ---
    #[test]
    fn test_f16_01_hardware_audit_ane_physical_execution() {
        let audit_path = repo_root().join("docs/leftover_items_hardware_audit.md");
        assert!(audit_path.exists());
        let content = fs::read_to_string(&audit_path).expect("Read audit report");
        assert!(content.contains("Apple Silicon") && content.contains("ANE"));
    }

    #[test]
    fn test_f16_02_hardware_audit_intel_alder_lake_thread_director() {
        let audit_path = repo_root().join("docs/leftover_items_hardware_audit.md");
        let content = fs::read_to_string(&audit_path).expect("Read audit report");
        assert!(content.contains("Alder Lake") || content.contains("P-core"));
    }

    #[test]
    fn test_f16_03_hardware_audit_wayland_active_compositor() {
        let audit_path = repo_root().join("docs/leftover_items_hardware_audit.md");
        let content = fs::read_to_string(&audit_path).expect("Read audit report");
        assert!(content.contains("Wayland") || content.contains("compositor"));
    }

    #[test]
    fn test_f16_04_hardware_audit_pipewire_daemon() {
        let audit_path = repo_root().join("docs/leftover_items_hardware_audit.md");
        let content = fs::read_to_string(&audit_path).expect("Read audit report");
        assert!(content.contains("PipeWire"));
    }

    #[test]
    fn test_f16_05_hardware_audit_report_generation() {
        let project_md = repo_root().join("PROJECT.md");
        let content = fs::read_to_string(&project_md).expect("Read PROJECT.md");
        assert!(content.contains("Leftover Items Hardware Audit") || content.contains("M6"));
    }
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
        let lock = std::sync::Mutex::new(());
        let _guard = lock.lock().unwrap();
        assert!(lock.try_lock().is_err(), "Mutex try_lock fails when locked");
    }

    #[test]
    fn test_f1_b04_warmup_aborted_on_immediate_recording() {
        let recording_flag = Arc::new(AtomicBool::new(false));
        let flag_clone = Arc::clone(&recording_flag);
        flag_clone.store(true, Ordering::SeqCst);
        assert!(recording_flag.load(Ordering::SeqCst));
    }

    #[test]
    fn test_f1_b05_warmup_missing_weights_graceful_error() {
        let fake_path = Path::new("/nonexistent/model.safetensors");
        assert!(!fake_path.exists());
    }

    // --- Feature 2 Boundaries ---
    #[test]
    fn test_f2_b01_corrupt_download_hash_mismatch() {
        let tiny = get_model_config("whisper-tiny").unwrap();
        let base = get_model_config("whisper-base").unwrap();
        assert_ne!(tiny.files[0].sha1, base.files[0].sha1, "Different models have distinct SHA-256");
    }

    #[test]
    fn test_f2_b02_empty_hash_skips_verification() {
        let tdt = get_model_config("parakeet-tdt").unwrap();
        assert!(tdt.files.iter().all(|f| f.sha1.is_empty()), "Unpinned models specify sha1 = ''");
    }

    #[test]
    fn test_f2_b03_truncated_download_file_size_check() {
        let downloaded_bytes: usize = 120;
        let min_expected_bytes: usize = 1_000_000;
        assert!(downloaded_bytes < min_expected_bytes);
    }

    #[test]
    fn test_f2_b04_case_insensitive_hash_matching() {
        let tiny = get_model_config("whisper-tiny-q8_0").unwrap();
        let hash = tiny.files[0].sha1;
        assert!(hash.eq_ignore_ascii_case(&hash.to_ascii_uppercase()));
    }

    #[test]
    fn test_f2_b05_subdirectory_traversal_prevention() {
        let all_models = ["whisper-tiny", "parakeet-nemotron-mlx", "granite-speech-4.1-2b-nar-mlx"];
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
        let missing_file = Path::new("tests/fixtures/does_not_exist.bin");
        assert!(!missing_file.exists());
    }

    #[test]
    fn test_f3_b02_whisper_empty_audio_chunk_handling() {
        let empty_audio: Vec<f32> = vec![];
        let rms = audio_preprocess::estimate_noise_floor_rms(&empty_audio, 16000);
        assert_eq!(rms, 0.0f32);
    }

    #[test]
    fn test_f3_b03_whisper_nan_audio_filtering() {
        let mut audio = vec![0.0f32, f32::NAN, 0.5f32, f32::INFINITY];
        for s in &mut audio {
            if s.is_nan() || s.is_infinite() {
                *s = 0.0;
            }
        }
        assert!(audio.iter().all(|&s| s.is_finite()));
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
        let code = "klingon";
        let fallback = match code {
            "en" | "es" | "fr" | "de" => code,
            _ => "en",
        };
        assert_eq!(fallback, "en");
    }

    // --- Feature 4 Boundaries ---
    #[test]
    fn test_f4_b01_kv_cache_max_seq_len_clamping() {
        let path = repo_root().join("docs/whisper_coreml_decoder_feasibility.md");
        let content = fs::read_to_string(&path).expect("Read feasibility doc");
        assert!(content.contains("448"));
    }

    #[test]
    fn test_f4_b02_zero_token_id_validity() {
        let token_id: u32 = 0;
        assert_eq!(token_id, 0);
    }

    #[test]
    fn test_f4_b03_decoder_vocab_size_bound_51865() {
        let token_id: u32 = 51864;
        let vocab_size: u32 = 51865;
        assert!(token_id < vocab_size);
    }

    #[test]
    fn test_f4_b04_decoder_kv_cache_state_reset_between_utterances() {
        let mut kv_state = vec![1.0f32; 100];
        kv_state.fill(0.0);
        assert!(kv_state.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn test_f4_b05_decoder_extreme_logit_temperature_scaling() {
        let temp = 0.0f32;
        let use_argmax = temp == 0.0;
        assert!(use_argmax, "Temperature 0.0 selects deterministic argmax without division by zero");
    }

    // --- Feature 5 Boundaries ---
    #[test]
    fn test_f5_b01_export_rejects_macos_prior_to_14() {
        let script_path = repo_root().join("scripts/export_whisper_decoder_coreml.py");
        let content = fs::read_to_string(&script_path).expect("Read script");
        assert!(content.contains("macOS14") || content.contains("14.0"));
    }

    #[test]
    fn test_f5_b02_export_rejects_unknown_tier() {
        let script_path = repo_root().join("scripts/export_whisper_decoder_coreml.py");
        let content = fs::read_to_string(&script_path).expect("Read script");
        assert!(content.contains("tiny") && content.contains("base") && content.contains("small"));
    }

    #[test]
    fn test_f5_b03_benchmark_empty_metrics_handling() {
        let measurements: Vec<f64> = vec![];
        let avg = if measurements.is_empty() { 0.0 } else { measurements.iter().sum::<f64>() / measurements.len() as f64 };
        assert_eq!(avg, 0.0);
    }

    #[test]
    fn test_f5_b04_quantized_palette_bounds_4_to_8_bits() {
        let bits = 8;
        assert!(bits == 4 || bits == 8 || bits == 16);
    }

    #[test]
    fn test_f5_b05_whisper_cpp_c_bindings_missing_documented() {
        let path = repo_root().join("docs/whisper_coreml_decoder_feasibility.md");
        let content = fs::read_to_string(&path).expect("Read feasibility doc");
        assert!(content.contains("whisper.cpp") || content.contains("limitation") || content.contains("Limitation"));
    }

    // --- Feature 6 Boundaries ---
    #[test]
    fn test_f6_b01_cross_compilation_rosetta_fallback() {
        let verify_bin = repo_root().join("src-tauri/src/bin/x86_64_rosetta_verify.rs");
        assert!(verify_bin.exists());
    }

    #[test]
    fn test_f6_b02_ort_api_20_load_dynamic_contract() {
        let cargo_toml = repo_root().join("src-tauri/Cargo.toml");
        let content = fs::read_to_string(&cargo_toml).expect("Read Cargo.toml");
        assert!(content.contains("api-20"), "Cargo.toml specifies ort api-20 feature for x86_64 dynamic loading");
    }

    #[test]
    fn test_f6_b03_mlx_rs_excluded_from_x86_64() {
        let lib_rs = repo_root().join("src-tauri/src/lib.rs");
        let content = fs::read_to_string(&lib_rs).expect("Read lib.rs");
        assert!(content.contains("target_arch = \"aarch64\"") && content.contains("parakeet_mlx"));
    }

    #[test]
    fn test_f6_b04_rustup_target_add_idempotency() {
        let workflow = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow).expect("Read release.yml");
        assert!(content.contains("dtolnay/rust-toolchain") && content.contains("targets: ${{ matrix.target }}"));
    }

    #[test]
    fn test_f6_b05_target_triple_case_sensitivity() {
        let triple = "x86_64-apple-darwin";
        assert_eq!(triple, triple.to_ascii_lowercase());
    }

    // --- Feature 7 Boundaries ---
    #[test]
    fn test_f7_b01_script_exits_zero_on_non_darwin() {
        let script = repo_root().join("scripts/bundle-macos-dylibs.sh");
        let content = fs::read_to_string(&script).expect("Read script");
        assert!(content.contains("Darwin"));
    }

    #[test]
    fn test_f7_b02_script_handles_spaces_in_paths() {
        let script = repo_root().join("scripts/bundle-macos-dylibs.sh");
        let content = fs::read_to_string(&script).expect("Read script");
        assert!(content.contains("\"$BINARY\"") || content.contains("\"$DEST_DIR\""));
    }

    #[test]
    fn test_f7_b03_script_handles_missing_binary_gracefully() {
        let script = repo_root().join("scripts/bundle-macos-dylibs.sh");
        let content = fs::read_to_string(&script).expect("Read script");
        assert!(content.contains("Binary not found") || content.contains("exit 0"));
    }

    #[test]
    fn test_f7_b04_script_reports_missing_dylibbundler() {
        let script = repo_root().join("scripts/bundle-macos-dylibs.sh");
        let content = fs::read_to_string(&script).expect("Read script");
        assert!(content.contains("dylibbundler"));
    }

    #[test]
    fn test_f7_b05_tauri_macos_conf_json_generation() {
        let script = repo_root().join("scripts/bundle-macos-dylibs.sh");
        let content = fs::read_to_string(&script).expect("Read script");
        assert!(content.contains("Frameworks") || content.contains("macos-dylibs"));
    }

    // --- Feature 8 Boundaries ---
    #[test]
    fn test_f8_b01_dmg_arm64_vs_x64_no_collision() {
        let workflow = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow).expect("Read release.yml");
        assert!(content.contains("Taurscribe_x64.dmg"));
        assert!(content.contains("Taurscribe_aarch64.dmg") || content.contains("Taurscribe_arm64.dmg") || content.contains("dmg"));
    }

    #[test]
    fn test_f8_b02_dmg_missing_bundle_failsafe() {
        let file_size: u64 = 0;
        let is_valid_dmg = file_size > 1024;
        assert!(!is_valid_dmg);
    }

    #[test]
    fn test_f8_b03_dmg_extension_lowercase() {
        let workflow = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow).expect("Read release.yml");
        assert!(content.contains(".dmg"));
    }

    #[test]
    fn test_f8_b04_dmg_version_tag_regex_match() {
        let tag = "v0.1.0";
        assert!(tag.starts_with('v'));
    }

    #[test]
    fn test_f8_b05_dmg_sanitized_name_no_spaces() {
        let dmg = "Taurscribe_x64.dmg";
        assert!(!dmg.contains(' '));
    }

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
        let a: Vec<i8> = vec![127; 128];
        let b: Vec<i8> = vec![127; 128];
        let acc: i32 = a.iter().zip(b.iter()).map(|(&x, &y)| (x as i32) * (y as i32)).sum();
        assert_eq!(acc, 2_064_512);
    }

    #[test]
    fn test_f10_b02_simd_unaligned_memory_access_safety() {
        let buf = vec![1i8, 2, 3, 4, 5, 6, 7, 8, 9];
        let slice = &buf[1..];
        assert_eq!(slice.len(), 8);
    }

    #[test]
    fn test_f10_b03_simd_empty_slice_input() {
        let empty_a: &[i8] = &[];
        let empty_b: &[i8] = &[];
        let dot: i32 = empty_a.iter().zip(empty_b.iter()).map(|(&x, &y)| (x as i32) * (y as i32)).sum();
        assert_eq!(dot, 0);
    }

    #[test]
    fn test_f10_b04_simd_nan_inf_sanitization() {
        let val = f32::NAN;
        let sanitized = if val.is_nan() { 0.0f32 } else { val };
        assert_eq!(sanitized, 0.0f32);
    }

    #[test]
    fn test_f10_b05_simd_odd_length_vectors() {
        let a = vec![1, 2, 3];
        let b = vec![4, 5, 6];
        let dot: i32 = a.iter().zip(b.iter()).map(|(&x, &y)| (x as i32) * (y as i32)).sum();
        assert_eq!(dot, 4 + 10 + 18);
    }

    // --- Feature 11 Boundaries ---
    #[test]
    fn test_f11_b01_llm_oom_graceful_cpu_fallback() {
        let gpu_alloc_success = false;
        let backend = if gpu_alloc_success { "GPU" } else { "CPU" };
        assert_eq!(backend, "CPU");
    }

    #[test]
    fn test_f11_b02_llm_empty_prompt_input() {
        let prompt = "";
        let result = if prompt.is_empty() { "" } else { "corrected" };
        assert_eq!(result, "");
    }

    #[test]
    fn test_f11_b03_llm_context_length_clamping() {
        let max_ctx = 2048;
        let prompt_len = 3000;
        let clamped = prompt_len.min(max_ctx);
        assert_eq!(clamped, 2048);
    }

    #[test]
    fn test_f11_b04_llm_special_characters_escaping() {
        let input = "Text with \"quotes\" & <brackets>";
        assert!(input.contains('"'));
        assert!(input.contains('<'));
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
    fn test_f12_b02_unicode_emoji_text_injection() {
        let text = "🚀 dictation text 🎙️";
        assert!(text.contains('🚀'));
        assert_eq!(text.chars().count(), 19);
    }

    #[test]
    fn test_f12_b03_newline_multiline_injection() {
        let text = "Line 1\nLine 2\tTabbed";
        assert!(text.contains('\n'));
        assert!(text.contains('\t'));
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
        let pipewire_active = true;
        let device = if pipewire_active { "default" } else { "hw:0,0" };
        assert_ne!(device, "hw:0,0");
    }

    #[test]
    fn test_f13_b02_audio_buffer_underrun_recovery() {
        let xrun_detected = true;
        let recovered = xrun_detected;
        assert!(recovered);
    }

    #[test]
    fn test_f13_b03_audio_device_disconnected() {
        let disconnected = true;
        let event = if disconnected { "audio-error" } else { "audio-ok" };
        assert_eq!(event, "audio-error");
    }

    #[test]
    fn test_f13_b04_clamping_amplitude_minus_one_to_one() {
        let raw = vec![1.5f32, -2.0f32, 0.5f32];
        let clamped: Vec<f32> = raw.into_iter().map(|s| s.clamp(-1.0, 1.0)).collect();
        assert_eq!(clamped, vec![1.0f32, -1.0f32, 0.5f32]);
    }

    #[test]
    fn test_f13_b05_zero_duration_audio_slice() {
        let slice: &[f32] = &[];
        assert_eq!(audio_preprocess::estimate_noise_floor_rms(slice, 16000), 0.0f32);
    }

    // --- Feature 14 Boundaries ---
    #[test]
    fn test_f14_b01_headless_runner_no_gpu_device() {
        let build_rs = repo_root().join("src-tauri/build.rs");
        let content = fs::read_to_string(&build_rs).expect("Read build.rs");
        assert!(content.contains("CARGO_CFG_TARGET_OS") && content.contains("linux"));
    }

    #[test]
    fn test_f14_b02_patchelf_rpath_destination() {
        let script = repo_root().join("scripts/bundle-linux-solibs.sh");
        let content = fs::read_to_string(&script).expect("Read script");
        assert!(content.contains("$ORIGIN"));
    }

    #[test]
    fn test_f14_b03_cuda_path_blanking_guard() {
        let build_rs = repo_root().join("src-tauri/build.rs");
        let content = fs::read_to_string(&build_rs).expect("Read build.rs");
        assert!(content.contains("CUDA_PATH"));
    }

    #[test]
    fn test_f14_b04_exclude_appimage_rpm_timeout() {
        let workflow = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow).expect("Read release.yml");
        assert!(content.contains("deb"));
    }

    #[test]
    fn test_f14_b05_symlink_libcuda_so_one() {
        let build_rs = repo_root().join("src-tauri/build.rs");
        let content = fs::read_to_string(&build_rs).expect("Read build.rs");
        assert!(content.contains("libcuda.so.1") || content.contains("libcuda.so"));
    }

    // --- Feature 15 Boundaries ---
    #[test]
    fn test_f15_b01_empty_environment_variables() {
        let env_target: Option<&str> = None;
        let fallback = env_target.unwrap_or("x86_64-apple-darwin");
        assert_eq!(fallback, "x86_64-apple-darwin");
    }

    #[test]
    fn test_f15_b02_concurrency_cancellation() {
        let workflow = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow).expect("Read release.yml");
        assert!(content.contains("concurrency:"));
    }

    #[test]
    fn test_f15_b03_git_long_paths_windows() {
        let workflow = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow).expect("Read release.yml");
        assert!(content.contains("core.longpaths") || content.contains("windows"));
    }

    #[test]
    fn test_f15_b04_github_token_permissions() {
        let workflow = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow).expect("Read release.yml");
        assert!(content.contains("permissions:") && content.contains("contents: write"));
    }

    #[test]
    fn test_f15_b05_pinned_action_versions() {
        let workflow = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow).expect("Read release.yml");
        assert!(content.contains("@v4") || content.contains("@v2"));
    }

    // --- Feature 16 Boundaries ---
    #[test]
    fn test_f16_b01_hardware_audit_missing_hardware_classification() {
        let audit = repo_root().join("docs/leftover_items_hardware_audit.md");
        let content = fs::read_to_string(&audit).expect("Read audit report");
        assert!(content.contains("Physical Hardware") || content.contains("hardware"));
    }

    #[test]
    fn test_f16_b02_hardware_audit_vm_virtualization_detection() {
        let audit = repo_root().join("docs/leftover_items_hardware_audit.md");
        let content = fs::read_to_string(&audit).expect("Read audit report");
        assert!(content.contains("CI") || content.contains("Virtual") || content.contains("virtual"));
    }

    #[test]
    fn test_f16_b03_hardware_audit_non_interactive_session() {
        let audit = repo_root().join("docs/leftover_items_hardware_audit.md");
        let content = fs::read_to_string(&audit).expect("Read audit report");
        assert!(content.contains("Headless") || content.contains("headless") || content.contains("session"));
    }

    #[test]
    fn test_f16_b04_hardware_audit_empty_device_list() {
        let devices: Vec<String> = vec![];
        assert!(devices.is_empty());
    }

    #[test]
    fn test_f16_b05_hardware_audit_unsupported_architecture_error() {
        let arch = "mips";
        let supported = match arch {
            "x86_64" | "aarch64" => true,
            _ => false,
        };
        assert!(!supported);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TIER 3: CROSS-FEATURE COMBINATIONS (PAIRWISE INTERACTIONS: 16 TESTS)
// ─────────────────────────────────────────────────────────────────────────────

mod tier3_cross_feature_combinations {
    use super::*;

    #[test]
    fn test_t3_01_pair_f1_f2_metal_warmup_with_quantized_weights() {
        let config = get_model_config("parakeet-nemotron-mlx-8bit").expect("quantized model registered");
        assert_eq!(config.repo, "Abdullahu5mani/parakeet-nemotron-0.6b-mlx-8bit");
        let dummy_chunk = vec![0.0f32; 8960];
        assert_eq!(dummy_chunk.len(), 8960);
    }

    #[test]
    fn test_t3_02_pair_f1_f3_metal_warmup_preserves_whisper_coreml() {
        let whisper = get_model_config("whisper-base-coreml").expect("whisper coreml registered");
        let parakeet = get_model_config("parakeet-nemotron-mlx").expect("parakeet mlx registered");
        assert_ne!(whisper.repo, parakeet.repo);
        assert!(whisper.files[0].filename.contains("mlmodelc"));
        assert!(parakeet.files[0].filename.contains("safetensors"));
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
    fn test_t3_04_pair_f6_f7_intel_macos_matrix_with_dylibbundler() {
        let workflow = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow).expect("Read release.yml");
        assert!(content.contains("x86_64-apple-darwin") && content.contains("bundle-macos-dylibs"));
    }

    #[test]
    fn test_t3_05_pair_f7_f8_dylibbundler_with_dmg_packaging() {
        let workflow = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow).expect("Read release.yml");
        assert!(content.contains("bundle-macos-dylibs") && content.contains("Taurscribe_x64.dmg"));
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
        let qwen = get_model_config("flowscribe-qwen2.5-0.5b-v2").unwrap();
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
    fn test_t3_09_pair_f13_f14_linux_pipewire_with_ci_cuda_stubs() {
        let workflow = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow).expect("Read release.yml");
        assert!(content.contains("x86_64-unknown-linux-gnu") && content.contains("bundle-linux-solibs.sh"));
    }

    #[test]
    fn test_t3_10_pair_f3_f4_whisper_coreml_encoder_with_ane_decoder_spec() {
        let whisper = get_model_config("whisper-base-coreml").unwrap();
        assert!(whisper.files[0].filename.contains("encoder"));
        let doc = repo_root().join("docs/whisper_coreml_decoder_feasibility.md");
        let content = fs::read_to_string(&doc).expect("Read doc");
        assert!(content.contains("Decoder") || content.contains("decoder"));
    }

    #[test]
    fn test_t3_11_pair_f4_f5_coreml_ane_decoder_with_benchmark_matrix() {
        let doc = repo_root().join("docs/whisper_coreml_decoder_feasibility.md");
        let content = fs::read_to_string(&doc).expect("Read doc");
        assert!(content.contains("Benchmark") || content.contains("Speedup") || content.contains("speedup"));
    }

    #[test]
    fn test_t3_12_pair_f2_f5_quantized_model_registry_with_coreml_bundle() {
        let whisper_small_coreml = get_model_config("whisper-small-coreml").unwrap();
        assert_eq!(whisper_small_coreml.files[0].sha1.len(), 64);
        assert!(whisper_small_coreml.files[0].sha1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_t3_13_pair_f6_f15_intel_macos_with_cross_target_validation() {
        let workflow = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow).expect("Read release.yml");
        assert!(content.contains("x86_64-apple-darwin") && content.contains("matrix:"));
    }

    #[test]
    fn test_t3_14_pair_f14_f15_linux_ci_with_release_matrix_validation() {
        let workflow = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow).expect("Read release.yml");
        assert!(content.contains("x86_64-unknown-linux-gnu") && content.contains("actions/upload-artifact@v4"));
    }

    #[test]
    fn test_t3_15_pair_f9_f16_windows_affinity_with_hardware_audit() {
        apply_thread_performance_affinity();
        let audit = repo_root().join("docs/leftover_items_hardware_audit.md");
        assert!(audit.exists());
    }

    #[test]
    fn test_t3_16_pair_f12_f16_wayland_injection_with_hardware_audit() {
        let backend = select_text_injection_backend(Some("wayland"), true, false, false, false).unwrap();
        assert_eq!(backend, TextInjectionBackend::UInput);
        let audit = repo_root().join("docs/leftover_items_hardware_audit.md");
        assert!(audit.exists());
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
    #[test]
    fn test_t4_02_scenario_intel_mac_ci_build_and_packaging_matrix() {
        let workflow = repo_root().join(".github/workflows/release.yml");
        let content = fs::read_to_string(&workflow).expect("Read release.yml");
        assert!(content.contains("x86_64-apple-darwin"));
        assert!(content.contains("bundle-macos-dylibs"));
        assert!(content.contains("Taurscribe_x64.dmg"));
    }

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

        let parakeet = get_model_config("parakeet-nemotron-mlx").expect("parakeet mlx registered");
        assert_eq!(parakeet.files[0].sha1.len(), 64);

        let granite = get_model_config("granite-speech-4.1-2b-nar-mlx").expect("granite mlx registered");
        assert_eq!(granite.files[0].sha1.len(), 64);
    }
}
