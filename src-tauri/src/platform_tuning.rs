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
    fn test_empty_cores_slice() {
        assert_eq!(compute_hybrid_p_core_mask(&[]), None);
        assert_eq!(compute_topology_from_cores(&[]), None);
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
