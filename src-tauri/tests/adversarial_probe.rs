//! Adversarial Probe & Empirical Stress Harness for Taurscribe
//! Tests M1-M5 core contracts, edge cases, fallback ladders, and hardware paths.

use taurscribe_lib::cpu_features::SimdCapabilities;
use taurscribe_lib::platform_tuning::{compute_hybrid_p_core_mask, compute_topology_from_cores};
use taurscribe_lib::sort_audio_devices_by_priority;
use taurscribe_lib::text_injection::{inject_text_or_paste, select_text_injection_backend, TextInjectionBackend};

#[test]
fn test_m4_p_core_empty_topology() {
    let cores: Vec<(u8, usize)> = vec![];
    assert_eq!(compute_hybrid_p_core_mask(&cores), None);
    assert_eq!(compute_topology_from_cores(&cores), None);
}

#[test]
fn test_m4_p_core_single_core_homogeneous() {
    let cores_0 = vec![(0u8, 1usize)];
    assert_eq!(compute_hybrid_p_core_mask(&cores_0), None);
    let topo_0 = compute_topology_from_cores(&cores_0).unwrap();
    assert!(!topo_0.is_hybrid);
    assert_eq!(topo_0.p_core_affinity_mask, None);
    assert_eq!(topo_0.p_core_count, 1);
    assert_eq!(topo_0.e_core_count, 0);
    assert_eq!(topo_0.total_cores, 1);

    let cores_1 = vec![(1u8, 1usize)];
    assert_eq!(compute_hybrid_p_core_mask(&cores_1), None);
}

#[test]
fn test_m4_p_core_multi_core_homogeneous() {
    // 8-core AMD Ryzen or standard Intel non-hybrid
    let cores: Vec<(u8, usize)> = (0..8).map(|i| (0u8, 1usize << i)).collect();
    assert_eq!(compute_hybrid_p_core_mask(&cores), None);
    let topo = compute_topology_from_cores(&cores).unwrap();
    assert!(!topo.is_hybrid);
    assert_eq!(topo.p_core_count, 8);
    assert_eq!(topo.e_core_count, 0);
    assert_eq!(topo.total_cores, 8);
}

#[test]
fn test_m4_p_core_alder_lake_heterogeneous() {
    // Alder Lake: 8 P-cores (class 1, masks 0x01..0x80) + 8 E-cores (class 0, masks 0x100..0x8000)
    let mut cores = Vec::new();
    for i in 0..8 {
        cores.push((1u8, 1usize << i));
    }
    for i in 8..16 {
        cores.push((0u8, 1usize << i));
    }

    let mask = compute_hybrid_p_core_mask(&cores).unwrap();
    assert_eq!(mask, 0x00FF, "Alder Lake P-core mask must be 0x00FF");

    let topo = compute_topology_from_cores(&cores).unwrap();
    assert!(topo.is_hybrid);
    assert_eq!(topo.p_core_affinity_mask, Some(0x00FF));
    assert_eq!(topo.p_core_count, 8);
    assert_eq!(topo.e_core_count, 8);
    assert_eq!(topo.total_cores, 16);
}

#[test]
fn test_m4_p_core_3_tier_meteor_lake() {
    // 3 efficiency classes: Class 2 (P-cores), Class 1 (E-cores), Class 0 (LP E-cores)
    let mut cores = Vec::new();
    // 6 P-cores
    for i in 0..6 {
        cores.push((2u8, 1usize << i));
    }
    // 8 E-cores
    for i in 6..14 {
        cores.push((1u8, 1usize << i));
    }
    // 2 LP-E cores
    for i in 14..16 {
        cores.push((0u8, 1usize << i));
    }

    let mask = compute_hybrid_p_core_mask(&cores).unwrap();
    assert_eq!(mask, 0x003F, "Meteor Lake P-core mask must encompass only highest efficiency class (0x003F)");

    let topo = compute_topology_from_cores(&cores).unwrap();
    assert!(topo.is_hybrid);
    assert_eq!(topo.p_core_count, 6);
    assert_eq!(topo.e_core_count, 10);
    assert_eq!(topo.total_cores, 16);
}

#[test]
fn test_m4_p_core_all_masks_zero_corner_case() {
    let cores = vec![(1u8, 0usize), (0u8, 0usize)];
    assert_eq!(compute_hybrid_p_core_mask(&cores), None);
    let topo = compute_topology_from_cores(&cores).unwrap();
    assert!(topo.is_hybrid);
    assert_eq!(topo.p_core_affinity_mask, None);
}

#[test]
fn test_m4_p_core_64_bit_boundary() {
    // 64-bit mask with highest bit set (bit 63)
    let cores = vec![(1u8, 1usize << 63), (0u8, 1usize)];
    let mask = compute_hybrid_p_core_mask(&cores).unwrap();
    assert_eq!(mask, 1usize << 63);
}

#[test]
fn test_m4_simd_capabilities_runtime() {
    let caps = SimdCapabilities::detect();
    let _summary = caps.summary();

    #[cfg(target_arch = "aarch64")]
    {
        assert!(!caps.has_avx2);
        assert!(!caps.has_fma);
        assert!(!caps.has_avx512f);
        assert!(!caps.has_avx512vnni);
        assert!(!caps.has_avxvnni);
        assert!(!caps.has_int8_hardware_acceleration());
        assert!(!caps.has_any_avx512());
        assert_eq!(caps.summary(), "None / Non-x86 (Baseline)");
    }
}

#[test]
fn test_m4_simd_capabilities_permutations() {
    // Stress test the boolean predicates across all combinations
    for &avx2 in &[false, true] {
        for &fma in &[false, true] {
            for &avx512f in &[false, true] {
                for &avx512vnni in &[false, true] {
                    for &avxvnni in &[false, true] {
                        let c = SimdCapabilities {
                            has_avx2: avx2,
                            has_fma: fma,
                            has_avx512f: avx512f,
                            has_avx512vnni: avx512vnni,
                            has_avxvnni: avxvnni,
                        };
                        assert_eq!(c.has_int8_hardware_acceleration(), avxvnni || avx512vnni);
                        assert_eq!(c.has_any_avx512(), avx512f || avx512vnni);
                    }
                }
            }
        }
    }
}

#[test]
fn test_m5_wayland_selection_exhaustive_truth_table() {
    // Test all 16 permutations of (has_uinput, has_ydotool, has_wtype, has_portal)
    for u in &[false, true] {
        for y in &[false, true] {
            for w in &[false, true] {
                for p in &[false, true] {
                    let res = select_text_injection_backend(Some("wayland"), *u, *y, *w, *p);
                    if *u {
                        assert_eq!(res.unwrap(), TextInjectionBackend::UInput);
                    } else if *y {
                        assert_eq!(res.unwrap(), TextInjectionBackend::Ydotool);
                    } else if *w {
                        assert_eq!(res.unwrap(), TextInjectionBackend::Wtype);
                    } else if *p {
                        assert_eq!(res.unwrap(), TextInjectionBackend::RemoteDesktopPortal);
                    } else {
                        assert!(res.is_err(), "All tools absent must return Err");
                    }
                }
            }
        }
    }
}

#[test]
fn test_m5_x11_selection_ignores_wayland_tools() {
    // Under X11, regardless of tool availability, backend is Enigo
    let res = select_text_injection_backend(Some("x11"), true, true, true, true).unwrap();
    assert_eq!(res, TextInjectionBackend::Enigo);

    let res_none = select_text_injection_backend(None, true, true, true, true).unwrap();
    assert_eq!(res_none, TextInjectionBackend::Enigo);
}

// Retired with the Parakeet MLX runtime. The GGUF engine's real-hardware
// warmup/inference path is covered by the file and meeting harness instead.
#[cfg(any())]
#[test]
fn test_m1_real_parakeet_mlx_metal_warmup_and_reset() {
    use std::path::PathBuf;
    let home = std::env::var("HOME").unwrap_or_default();
    let model_dir = PathBuf::from(home)
        .join("Library/Application Support/Taurscribe/models/parakeet-nemotron-mlx");

    if !model_dir.exists() {
        eprintln!("[SKIP] Parakeet MLX model weights not found at {}", model_dir.display());
        return;
    }

    println!("[TEST] Loading real Parakeet MLX weights for adversarial stress testing...");
    let mut model = taurscribe_lib::parakeet_mlx::ParakeetNemotronMlx::load(&model_dir)
        .expect("Failed to load real Parakeet Nemotron MLX model");

    // 1. Cold warmup
    let start = std::time::Instant::now();
    let res = model.warmup();
    let dur = start.elapsed();
    assert!(res.is_ok(), "Warmup must succeed on Apple Silicon Metal: {:?}", res);
    println!("[TEST] Cold Metal warmup completed in {:.3}s", dur.as_secs_f32());
    let max_warmup_secs = if std::env::var("MTL_SHADER_VALIDATION").as_deref() == Ok("1") {
        4.5
    } else {
        1.5
    };
    assert!(
        dur.as_secs_f32() < max_warmup_secs,
        "Cold warmup must complete in under {:.1}s (validation={}), took {:?}",
        max_warmup_secs,
        std::env::var("MTL_SHADER_VALIDATION").as_deref() == Ok("1"),
        dur
    );
    assert_eq!(model.cache_len(), 0, "Cache length after warmup must be 0");
    assert!(model.is_clean_state(), "Model state must be clean after warmup");

    // 2. Sequential warmups (idempotency + no memory explosion)
    for i in 2..=4 {
        let t = std::time::Instant::now();
        let res_sub = model.warmup();
        assert!(res_sub.is_ok(), "Sequential warmup #{} failed: {:?}", i, res_sub);
        println!("[TEST] Warmup #{} completed in {:.3}s", i, t.elapsed().as_secs_f32());
    }

    // 3. Test real forward pass with 560ms synthetic sine audio (440Hz at 16kHz)
    let mut synth_chunk = Vec::with_capacity(8960);
    for i in 0..8960 {
        let t = i as f32 / 16000.0;
        synth_chunk.push((2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.5);
    }
    let chunk_start = std::time::Instant::now();
    let trans_res = model.transcribe_chunk(&synth_chunk);
    let chunk_dur = chunk_start.elapsed();
    assert!(trans_res.is_ok(), "Inference after warmup must succeed: {:?}", trans_res);
    let chunk_ms = chunk_dur.as_secs_f32() * 1000.0;
    println!("[TEST] Subsequent chunk inference completed in {:.2}ms", chunk_ms);
    let max_chunk_ms = if std::env::var("MTL_SHADER_VALIDATION").as_deref() == Ok("1") {
        250.0
    } else {
        125.0
    };
    assert!(
        chunk_ms < max_chunk_ms,
        "Subsequent chunk must run in <{:.0}ms, took {:.2}ms",
        max_chunk_ms,
        chunk_ms
    );

    // 4. Test state reset clean guarantee
    model.reset();
    assert_eq!(model.cache_len(), 0, "Cache length after manual reset must be 0");
    assert!(model.is_clean_state(), "Model state must be clean after manual reset");
    let trans_after_reset = model.transcribe_chunk(&synth_chunk);
    assert!(trans_after_reset.is_ok(), "Inference after reset must succeed");
}

#[test]
fn test_adversarial_audio_device_sorting_empty_and_single() {
    // 1. Empty list
    let mut empty: Vec<String> = vec![];
    sort_audio_devices_by_priority(&mut empty);
    assert!(empty.is_empty(), "Sorting empty list must remain empty");

    // 2. Single virtual PCM
    let mut single_default = vec!["default".to_string()];
    sort_audio_devices_by_priority(&mut single_default);
    assert_eq!(single_default, vec!["default"]);

    // 3. Single hardware device
    let mut single_hw = vec!["hw:0,0".to_string()];
    sort_audio_devices_by_priority(&mut single_hw);
    assert_eq!(single_hw, vec!["hw:0,0"]);

    // 4. Single arbitrary device
    let mut single_custom = vec!["External USB Mic".to_string()];
    sort_audio_devices_by_priority(&mut single_custom);
    assert_eq!(single_custom, vec!["External USB Mic"]);
}

#[test]
fn test_adversarial_audio_device_sorting_boundary_inputs() {
    // Edge cases: empty strings, whitespace, Unicode emoji, CJK, control chars, symbols
    let mut devices = vec![
        "hw:0,0".to_string(),
        "".to_string(),
        "   ".to_string(),
        "🎙️ Studio Microphone".to_string(),
        "麦克风 输入".to_string(),
        "\0\n\t".to_string(),
        "!@#$%^&*()_+".to_string(),
        "pipewire-stream".to_string(),
        "sysdefault".to_string(),
    ];
    sort_audio_devices_by_priority(&mut devices);

    // Tier 0: "sysdefault"
    assert_eq!(devices[0], "sysdefault");
    // Tier 1: "pipewire-stream"
    assert_eq!(devices[1], "pipewire-stream");
    // Tier 4: "hw:0,0" must be last (index 8)
    assert_eq!(devices[8], "hw:0,0");
}

#[test]
fn test_adversarial_audio_device_sorting_duplicates_and_case() {
    let mut devices = vec![
        "HW:1,0".to_string(),
        "DEFAULT".to_string(),
        "default".to_string(),
        "SysDefault".to_string(),
        "PipeWire-Output".to_string(),
        "pipewire-input".to_string(),
        "PulseAudio Jack".to_string(),
        "pulse".to_string(),
        "hw:0,0".to_string(),
    ];
    sort_audio_devices_by_priority(&mut devices);

    // Tier 0: DEFAULT, default, SysDefault (indices 0..3)
    for dev in &devices[0..3] {
        let n = dev.to_lowercase();
        assert!(n == "default" || n == "sysdefault", "Must be tier 0: {}", dev);
    }

    // Tier 1: PipeWire-Output, pipewire-input (indices 3..5)
    for dev in &devices[3..5] {
        assert!(dev.to_lowercase().contains("pipewire"), "Must be tier 1: {}", dev);
    }

    // Tier 2: PulseAudio Jack, pulse (indices 5..7)
    for dev in &devices[5..7] {
        assert!(dev.to_lowercase().contains("pulse"), "Must be tier 2: {}", dev);
    }

    // Tier 4: HW:1,0, hw:0,0 (indices 7..9)
    for dev in &devices[7..9] {
        assert!(dev.to_lowercase().contains("hw:"), "Must be tier 4: {}", dev);
    }
}

#[test]
fn test_adversarial_audio_device_sorting_stability() {
    // Verify that relative order among equal-priority devices is strictly preserved
    let mut devices = vec![
        "Custom Device Alpha".to_string(),
        "Custom Device Beta".to_string(),
        "Custom Device Gamma".to_string(),
        "hw:3,0".to_string(),
        "hw:1,0".to_string(),
        "hw:2,0".to_string(),
    ];
    sort_audio_devices_by_priority(&mut devices);

    // Priority 3 items (Alpha, Beta, Gamma) should preserve original sequence
    assert_eq!(devices[0], "Custom Device Alpha");
    assert_eq!(devices[1], "Custom Device Beta");
    assert_eq!(devices[2], "Custom Device Gamma");

    // Priority 4 items (hw:3,0, hw:1,0, hw:2,0) should preserve original sequence
    assert_eq!(devices[3], "hw:3,0");
    assert_eq!(devices[4], "hw:1,0");
    assert_eq!(devices[5], "hw:2,0");
}

#[test]
fn test_adversarial_audio_device_sorting_high_volume() {
    let mut devices = Vec::with_capacity(5000);
    for i in 0..1000 {
        devices.push("hw:9,9".to_string());
        devices.push(format!("Generic Microphone #{}", i));
        devices.push("pulseaudio-device".to_string());
        devices.push("pipewire-virtual-sink".to_string());
        devices.push(if i % 2 == 0 { "default".to_string() } else { "sysdefault".to_string() });
    }

    let start = std::time::Instant::now();
    sort_audio_devices_by_priority(&mut devices);
    let elapsed = start.elapsed();

    assert!(elapsed.as_millis() < 100, "Sorting 5,000 devices must finish in under 100ms: {:?}", elapsed);

    let priority = |name: &str| -> usize {
        let n = name.to_lowercase();
        if n == "default" || n == "sysdefault" { 0 }
        else if n.contains("pipewire") { 1 }
        else if n.contains("pulse") { 2 }
        else if n.contains("hw:") { 4 }
        else { 3 }
    };

    for i in 1..devices.len() {
        assert!(priority(&devices[i - 1]) <= priority(&devices[i]),
            "Priority violation at index {}: {} vs {}", i, devices[i-1], devices[i]);
    }
}

#[test]
#[ignore = "pastes into the frontmost app; run with --ignored only with a disposable text field focused"]
fn test_adversarial_clipboard_multithreaded_concurrency() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    let success_count = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    // Spawn 5 concurrent threads attempting to inject text
    for i in 0..5 {
        let sc = Arc::clone(&success_count);
        let handle = thread::spawn(move || {
            let res = inject_text_or_paste(&format!("concurrency test payload {}", i));
            // On macOS / desktop with pasteboard access, this returns Ok(Enigo).
            // Even if an OS environment restricts pasteboard, it must return a Result without panic or SIGSEGV.
            if res.is_ok() {
                sc.fetch_add(1, Ordering::SeqCst);
            }
        });
        handles.push(handle);
    }

    for h in handles {
        h.join().expect("Worker thread panicked during concurrent clipboard injection!");
    }

    println!("[TEST] All 5 concurrent clipboard worker threads completed cleanly. Success count: {}",
        success_count.load(Ordering::SeqCst));
}
