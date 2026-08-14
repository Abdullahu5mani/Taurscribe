//! Adversarial Probe & Empirical Stress Harness for Taurscribe
//! Tests M1-M5 core contracts, edge cases, fallback ladders, and hardware paths.

use taurscribe_lib::cpu_features::SimdCapabilities;
use taurscribe_lib::platform_tuning::{compute_hybrid_p_core_mask, compute_topology_from_cores};
use taurscribe_lib::text_injection::{select_text_injection_backend, TextInjectionBackend};

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

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
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
    let trans_res = model.transcribe_chunk(&synth_chunk);
    assert!(trans_res.is_ok(), "Inference after warmup must succeed: {:?}", trans_res);

    // 4. Test state reset clean guarantee
    model.reset();
    let trans_after_reset = model.transcribe_chunk(&synth_chunk);
    assert!(trans_after_reset.is_ok(), "Inference after reset must succeed");
}
