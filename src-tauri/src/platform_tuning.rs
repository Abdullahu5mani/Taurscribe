//! Platform-specific CPU and thread performance tuning.
//!
//! On Windows, provides thread affinity pinning (`SetThreadAffinityMask`) for Intel hybrid
//! architecture (Alder Lake / Raptor Lake / Arrow Lake) performance cores (P-cores),
//! elevated thread priority (`THREAD_PRIORITY_ABOVE_NORMAL`), and disabling of
//! EcoQoS power throttling (`ThreadPowerThrottling`).
//!
//! On non-Windows platforms (macOS, Linux), operations safely degrade to no-ops.

/// Hybrid CPU topology description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HybridCpuTopology {
    /// True if the CPU possesses heterogeneous cores (e.g. Intel P-cores and E-cores).
    pub is_hybrid: bool,
    /// Bitmask for Performance Cores (P-cores) if hybrid, or None if homogeneous.
    pub p_core_affinity_mask: Option<usize>,
    /// Number of identified P-cores.
    pub p_core_count: usize,
    /// Number of identified E-cores.
    pub e_core_count: usize,
    /// Total number of physical cores discovered.
    pub total_cores: usize,
}

/// Pure algorithmic core for computing P-core affinity mask from core records.
///
/// Each core record is `(efficiency_class, affinity_mask)`.
/// Returns `Some(p_core_mask)` if heterogeneous classes are found, or `None` if homogeneous.
pub fn compute_hybrid_p_core_mask(cores: &[(u8, usize)]) -> Option<usize> {
    if cores.is_empty() {
        return None;
    }

    let max_eff = cores.iter().map(|(eff, _)| *eff).max().unwrap_or(0);
    let min_eff = cores.iter().map(|(eff, _)| *eff).min().unwrap_or(0);

    // If all cores share the same efficiency class, this CPU is homogeneous (e.g. AMD Ryzen, older Intel).
    // Do not restrict thread affinity.
    if max_eff <= min_eff {
        return None;
    }

    let mut p_mask = 0usize;
    for (eff, mask) in cores {
        if *eff == max_eff {
            p_mask |= *mask;
        }
    }

    if p_mask == 0 {
        None
    } else {
        Some(p_mask)
    }
}

/// Analyzes a slice of core records `(efficiency_class, affinity_mask)` and returns topology details.
pub fn compute_topology_from_cores(cores: &[(u8, usize)]) -> Option<HybridCpuTopology> {
    if cores.is_empty() {
        return None;
    }

    let max_eff = cores.iter().map(|(eff, _)| *eff).max().unwrap_or(0);
    let min_eff = cores.iter().map(|(eff, _)| *eff).min().unwrap_or(0);
    let is_hybrid = max_eff > min_eff;

    let mut p_mask = 0usize;
    let mut p_count = 0usize;
    let mut e_count = 0usize;

    for (eff, mask) in cores {
        if is_hybrid && *eff == max_eff {
            p_mask |= *mask;
            p_count += 1;
        } else if is_hybrid {
            e_count += 1;
        } else {
            p_count += 1;
        }
    }

    Some(HybridCpuTopology {
        is_hybrid,
        p_core_affinity_mask: if is_hybrid && p_mask != 0 {
            Some(p_mask)
        } else {
            None
        },
        p_core_count: if is_hybrid { p_count } else { cores.len() },
        e_core_count: if is_hybrid { e_count } else { 0 },
        total_cores: cores.len(),
    })
}
/// Serializes or parses raw binary buffer representation of Windows `SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX` records.
pub fn parse_processor_info_bytes(buffer: &[u8]) -> Option<Vec<(u8, usize)>> {
    let mut offset = 0usize;
    let mut cores = Vec::new();

    while offset + 8 <= buffer.len() {
        let rel = u32::from_ne_bytes(buffer[offset..offset + 4].try_into().ok()?);
        let size = u32::from_ne_bytes(buffer[offset + 4..offset + 8].try_into().ok()?);
        if size == 0 || offset + (size as usize) > buffer.len() {
            break;
        }

        // RelationProcessorCore = 0
        if rel == 0 && size >= 36 {
            let eff = buffer[offset + 9];
            let group_count = u16::from_ne_bytes(buffer[offset + 30..offset + 32].try_into().ok()?);
            if group_count > 0 && offset + 32 + std::mem::size_of::<usize>() <= buffer.len() {
                let mask = usize::from_ne_bytes(
                    buffer[offset + 32..offset + 32 + std::mem::size_of::<usize>()].try_into().ok()?,
                );
                cores.push((eff, mask));
            }
        }
        offset += size as usize;
    }

    if cores.is_empty() {
        None
    } else {
        Some(cores)
    }
}

/// Helper to synthesize a raw binary buffer for a processor core record.
pub fn build_synthetic_core_record(efficiency_class: u8, mask: usize) -> Vec<u8> {
    let mut rec = Vec::with_capacity(48);
    rec.extend_from_slice(&0u32.to_ne_bytes()); // RelationProcessorCore = 0
    let size = (32 + std::mem::size_of::<usize>() + 8) as u32;
    rec.extend_from_slice(&size.to_ne_bytes()); // Size
    rec.push(0u8); // Flags
    rec.push(efficiency_class); // EfficiencyClass
    rec.extend_from_slice(&[0u8; 20]); // Reserved[20]
    rec.extend_from_slice(&1u16.to_ne_bytes()); // GroupCount = 1
    rec.extend_from_slice(&mask.to_ne_bytes()); // GroupMask[0].Mask
    rec.extend_from_slice(&0u16.to_ne_bytes()); // Group
    rec.extend_from_slice(&[0u8; 6]); // Reserved2[3]
    rec
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::*;
    use windows::Win32::System::SystemInformation::{
        GetLogicalProcessorInformationEx, RelationProcessorCore,
        SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
    };
    use windows::Win32::System::Threading::{
        GetCurrentThread, SetThreadAffinityMask, SetThreadInformation, SetThreadPriority,
        ThreadPowerThrottling, THREAD_POWER_THROTTLING_CURRENT_VERSION,
        THREAD_POWER_THROTTLING_EXECUTION_SPEED, THREAD_POWER_THROTTLING_STATE,
        THREAD_PRIORITY_ABOVE_NORMAL,
    };

    /// Queries Windows system information for all physical core relationships.
    ///
    /// Returns a list of `(EfficiencyClass, AffinityMask)` tuples.
    pub fn query_logical_processor_cores() -> Option<Vec<(u8, usize)>> {
        unsafe {
            let mut length: u32 = 0;
            let _ = GetLogicalProcessorInformationEx(RelationProcessorCore, None, &mut length);
            if length == 0 {
                return None;
            }

            let mut buffer = vec![0u8; length as usize];
            let p_info = buffer.as_mut_ptr() as *mut SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX;

            if GetLogicalProcessorInformationEx(RelationProcessorCore, Some(p_info), &mut length).is_err() {
                return None;
            }

            let mut offset = 0usize;
            let mut cores = Vec::new();

            while offset < length as usize {
                let item = &*(buffer.as_ptr().add(offset) as *const SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX);
                if item.Relationship == RelationProcessorCore {
                    let proc = &item.Anonymous.Processor;
                    let eff = proc.EfficiencyClass;
                    let mask = if proc.GroupCount > 0 {
                        proc.GroupMask[0].Mask
                    } else {
                        0
                    };
                    cores.push((eff, mask));
                }
                if item.Size == 0 {
                    break;
                }
                offset += item.Size as usize;
            }

            if cores.is_empty() {
                None
            } else {
                Some(cores)
            }
        }
    }

    /// Queries the P-core affinity mask for Windows hybrid processors.
    ///
    /// Returns `Some(mask)` if running on a hybrid CPU (e.g. Alder Lake, Raptor Lake),
    /// or `None` on homogeneous CPUs or if detection fails.
    pub fn get_performance_core_affinity_mask() -> Option<usize> {
        let cores = query_logical_processor_cores()?;
        compute_hybrid_p_core_mask(&cores)
    }

    /// Applies thread performance optimizations to the calling thread:
    /// 1. Pins affinity to Intel P-cores on hybrid architectures to avoid E-core latency stalls.
    /// 2. Sets thread priority to `THREAD_PRIORITY_ABOVE_NORMAL`.
    /// 3. Disables Windows EcoQoS power throttling (`ThreadPowerThrottling`).
    pub fn apply_thread_performance_affinity() {
        unsafe {
            let thread = GetCurrentThread();

            // 1. Thread affinity pinning (if hybrid CPU detected)
            if let Some(mask) = get_performance_core_affinity_mask() {
                let prev_mask = SetThreadAffinityMask(thread, mask);
                println!(
                    "[PERF] Windows P-core thread affinity applied (mask={:#x}, previous={:#x})",
                    mask, prev_mask
                );
            }

            // 2. Thread priority elevation
            if let Err(e) = SetThreadPriority(thread, THREAD_PRIORITY_ABOVE_NORMAL) {
                eprintln!("[WARN] Failed to set thread priority ABOVE_NORMAL: {:?}", e);
            } else {
                println!("[PERF] Windows thread priority set to ABOVE_NORMAL");
            }

            // 3. Disable EcoQoS power throttling
            let mut throttle = THREAD_POWER_THROTTLING_STATE {
                Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
                ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
                StateMask: 0,
            };
            let res = SetThreadInformation(
                thread,
                ThreadPowerThrottling,
                &mut throttle as *mut _ as *const _,
                std::mem::size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
            );
            if let Err(e) = res {
                eprintln!("[WARN] Failed to disable Windows thread power throttling: {:?}", e);
            } else {
                println!("[PERF] Windows EcoQoS power throttling disabled for worker thread");
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub use windows_impl::{
    apply_thread_performance_affinity, get_performance_core_affinity_mask,
    query_logical_processor_cores,
};

#[cfg(not(target_os = "windows"))]
pub fn get_performance_core_affinity_mask() -> Option<usize> {
    None
}

#[cfg(not(target_os = "windows"))]
pub fn apply_thread_performance_affinity() {
    // Safe no-op on non-Windows platforms (macOS, Linux).
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_homogeneous_cpu_no_restriction() {
        // Simulating AMD Ryzen 8 cores / 16 threads, all EfficiencyClass = 0
        let cores = vec![
            (0, 0x0003),
            (0, 0x000C),
            (0, 0x0030),
            (0, 0x00C0),
            (0, 0x0300),
            (0, 0x0C00),
            (0, 0x3000),
            (0, 0xC000),
        ];
        let mask = compute_hybrid_p_core_mask(&cores);
        assert_eq!(mask, None);

        let topo = compute_topology_from_cores(&cores).unwrap();
        assert!(!topo.is_hybrid);
        assert_eq!(topo.p_core_count, 8);
        assert_eq!(topo.e_core_count, 0);
        assert_eq!(topo.p_core_affinity_mask, None);
    }

    #[test]
    fn test_intel_alder_lake_hybrid_detection() {
        // Simulating Intel i7-12700K: 8 P-cores (hyperthreaded, class 1) + 4 E-cores (single-threaded, class 0)
        let cores = vec![
            // 8 P-cores
            (1, 0x0000_0003),
            (1, 0x0000_000C),
            (1, 0x0000_0030),
            (1, 0x0000_00C0),
            (1, 0x0000_0300),
            (1, 0x0000_0C00),
            (1, 0x0000_3000),
            (1, 0x0000_C000),
            // 4 E-cores
            (0, 0x0001_0000),
            (0, 0x0002_0000),
            (0, 0x0004_0000),
            (0, 0x0008_0000),
        ];

        let mask = compute_hybrid_p_core_mask(&cores);
        assert_eq!(mask, Some(0x0000_FFFF));

        let topo = compute_topology_from_cores(&cores).unwrap();
        assert!(topo.is_hybrid);
        assert_eq!(topo.p_core_count, 8);
        assert_eq!(topo.e_core_count, 4);
        assert_eq!(topo.total_cores, 12);
        assert_eq!(topo.p_core_affinity_mask, Some(0x0000_FFFF));
    }

    #[test]
    fn test_intel_meteor_lake_three_classes() {
        // Simulating 3 efficiency classes: LP-E cores (0), E-cores (1), P-cores (2)
        let cores = vec![
            (0, 0x0001), // LP-E
            (0, 0x0002), // LP-E
            (1, 0x0004), // E
            (1, 0x0008), // E
            (2, 0x0030), // P (HT)
            (2, 0x00C0), // P (HT)
        ];

        let mask = compute_hybrid_p_core_mask(&cores);
        assert_eq!(mask, Some(0x00F0));

        let topo = compute_topology_from_cores(&cores).unwrap();
        assert!(topo.is_hybrid);
        assert_eq!(topo.p_core_count, 2);
        assert_eq!(topo.e_core_count, 4);
        assert_eq!(topo.p_core_affinity_mask, Some(0x00F0));
    }

    #[test]
    fn test_case_a_intel_i9_13900k_hybrid() {
        // Case A: Intel Core i9-13900K (Hybrid)
        // 8 P-cores (16 threads with SMT, EfficiencyClass = 1) + 16 E-cores (16 threads without SMT, EfficiencyClass = 0)
        let mut raw_buffer = Vec::new();
        let mut cores = Vec::new();

        // 8 P-cores (each has 2 threads)
        for i in 0..8 {
            let mask = 0x3 << (i * 2);
            cores.push((1u8, mask));
            raw_buffer.extend_from_slice(&build_synthetic_core_record(1, mask));
        }
        // 16 E-cores (threads 16..31)
        for i in 0..16 {
            let mask = 0x1 << (16 + i);
            cores.push((0u8, mask));
            raw_buffer.extend_from_slice(&build_synthetic_core_record(0, mask));
        }

        // Test algorithmic core
        let mask = compute_hybrid_p_core_mask(&cores);
        assert_eq!(mask, Some(0x0000_FFFF), "P-core mask must isolate threads 0..15");

        let topo = compute_topology_from_cores(&cores).unwrap();
        assert!(topo.is_hybrid);
        assert_eq!(topo.p_core_count, 8);
        assert_eq!(topo.e_core_count, 16);
        assert_eq!(topo.total_cores, 24);

        // Test raw binary buffer parsing (simulating Windows Win32 API return buffer)
        let parsed_cores = parse_processor_info_bytes(&raw_buffer).expect("Parse raw buffer");
        assert_eq!(parsed_cores.len(), 24);
        let parsed_mask = compute_hybrid_p_core_mask(&parsed_cores);
        assert_eq!(parsed_mask, Some(0x0000_FFFF));
    }

    #[test]
    fn test_case_b_intel_core_ultra_7_155h_3tier() {
        // Case B: Intel Core Ultra 7 155H (3-Tier Hybrid)
        // 6 P-cores (eff=2) + 8 E-cores (eff=1) + 2 LP E-cores (eff=0)
        let mut raw_buffer = Vec::new();
        let mut cores = Vec::new();

        // 2 LP E-cores (threads 0..1)
        for i in 0..2 {
            let mask = 1 << i;
            cores.push((0u8, mask));
            raw_buffer.extend_from_slice(&build_synthetic_core_record(0, mask));
        }
        // 8 E-cores (threads 2..9)
        for i in 0..8 {
            let mask = 1 << (2 + i);
            cores.push((1u8, mask));
            raw_buffer.extend_from_slice(&build_synthetic_core_record(1, mask));
        }
        // 6 P-cores with SMT (threads 10..21)
        let mut p_expected_mask = 0usize;
        for i in 0..6 {
            let mask = 0x3 << (10 + i * 2);
            p_expected_mask |= mask;
            cores.push((2u8, mask));
            raw_buffer.extend_from_slice(&build_synthetic_core_record(2, mask));
        }

        let mask = compute_hybrid_p_core_mask(&cores);
        assert_eq!(mask, Some(p_expected_mask), "Must isolate only highest EfficiencyClass (eff=2)");

        let topo = compute_topology_from_cores(&cores).unwrap();
        assert!(topo.is_hybrid);
        assert_eq!(topo.p_core_count, 6);
        assert_eq!(topo.e_core_count, 10); // 8 E + 2 LP-E
        assert_eq!(topo.total_cores, 16);

        // Test raw binary buffer parsing
        let parsed_cores = parse_processor_info_bytes(&raw_buffer).expect("Parse raw buffer");
        assert_eq!(parsed_cores.len(), 16);
        let parsed_mask = compute_hybrid_p_core_mask(&parsed_cores);
        assert_eq!(parsed_mask, Some(p_expected_mask));
    }

    #[test]
    fn test_case_c_amd_ryzen_9_7950x_homogeneous() {
        // Case C: AMD Ryzen 9 7950X (Homogeneous)
        // 16 cores / 32 threads, all EfficiencyClass = 0
        let mut raw_buffer = Vec::new();
        let mut cores = Vec::new();

        for i in 0..16 {
            let mask = 0x3 << (i * 2);
            cores.push((0u8, mask));
            raw_buffer.extend_from_slice(&build_synthetic_core_record(0, mask));
        }

        let mask = compute_hybrid_p_core_mask(&cores);
        assert_eq!(mask, None, "Homogeneous CPU must return None to avoid core starvation");

        let topo = compute_topology_from_cores(&cores).unwrap();
        assert!(!topo.is_hybrid);
        assert_eq!(topo.p_core_count, 16);
        assert_eq!(topo.e_core_count, 0);
        assert_eq!(topo.p_core_affinity_mask, None);

        // Test raw binary buffer parsing
        let parsed_cores = parse_processor_info_bytes(&raw_buffer).expect("Parse raw buffer");
        assert_eq!(parsed_cores.len(), 16);
        let parsed_mask = compute_hybrid_p_core_mask(&parsed_cores);
        assert_eq!(parsed_mask, None);
    }

    #[test]
    fn test_empty_cores_slice() {
        assert_eq!(compute_hybrid_p_core_mask(&[]), None);
        assert_eq!(compute_topology_from_cores(&[]), None);
        assert_eq!(parse_processor_info_bytes(&[]), None);
        assert_eq!(parse_processor_info_bytes(&[0u8; 10]), None);
    }

    #[test]
    fn test_non_windows_degradation() {
        #[cfg(not(target_os = "windows"))]
        {
            assert_eq!(get_performance_core_affinity_mask(), None);
            apply_thread_performance_affinity();
        }
    }
}
