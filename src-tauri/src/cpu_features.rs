//! CPU feature detection for runtime SIMD dispatch.
//!
//! Provides runtime detection for x86/x86_64 vector extensions (AVX2, FMA,
//! AVX-512F, AVX-512VNNI, AVX-VNNI) used by quantized neural network inference kernels
//! (ONNX Runtime MLAS, whisper.cpp, llama.cpp).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SimdCapabilities {
    pub has_avx2: bool,
    pub has_fma: bool,
    pub has_avx512f: bool,
    pub has_avx512vnni: bool,
    pub has_avxvnni: bool,
}

impl SimdCapabilities {
    /// Detect SIMD capabilities of the running CPU at runtime.
    pub fn detect() -> Self {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            Self {
                has_avx2: is_x86_feature_detected!("avx2"),
                has_fma: is_x86_feature_detected!("fma"),
                has_avx512f: is_x86_feature_detected!("avx512f"),
                has_avx512vnni: is_x86_feature_detected!("avx512vnni"),
                has_avxvnni: is_x86_feature_detected!("avxvnni"),
            }
        }
        #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
        {
            // Non-x86 architectures (e.g. ARM64) do not have x86 SIMD extensions.
            Self {
                has_avx2: false,
                has_fma: false,
                has_avx512f: false,
                has_avx512vnni: false,
                has_avxvnni: false,
            }
        }
    }

    /// Returns true if hardware-accelerated INT8 vector dot-products are supported
    /// (either via 256-bit AVX-VNNI on Intel Alder/Raptor Lake or 512-bit AVX-512 VNNI on Xeon / Zen 4/5).
    pub fn has_int8_hardware_acceleration(&self) -> bool {
        self.has_avxvnni || self.has_avx512vnni
    }

    /// Returns true if any 512-bit AVX-512 vector extension is supported.
    pub fn has_any_avx512(&self) -> bool {
        self.has_avx512f || self.has_avx512vnni
    }

    /// Returns a human-readable list of detected SIMD instruction sets.
    pub fn summary(&self) -> String {
        let mut features = Vec::new();
        if self.has_avx2 {
            features.push("AVX2");
        }
        if self.has_fma {
            features.push("FMA");
        }
        if self.has_avx512f {
            features.push("AVX-512F");
        }
        if self.has_avx512vnni {
            features.push("AVX-512VNNI");
        }
        if self.has_avxvnni {
            features.push("AVX-VNNI");
        }
        if features.is_empty() {
            "None / Non-x86 (Baseline)".to_string()
        } else {
            features.join(", ")
        }
    }
}

/// Logs the detected SIMD capabilities at startup.
pub fn log_simd_capabilities() {
    let caps = SimdCapabilities::detect();
    println!(
        "[CPU] SIMD Feature Detection: {} (INT8 Acceleration: {})",
        caps.summary(),
        if caps.has_int8_hardware_acceleration() {
            "Hardware VPDPBUSD Active"
        } else {
            "Emulated / Baseline"
        }
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simd_detection_does_not_panic() {
        let caps = SimdCapabilities::detect();
        let _ = caps.summary();
        let _ = caps.has_int8_hardware_acceleration();
        let _ = caps.has_any_avx512();
    }

    #[test]
    fn test_int8_acceleration_logic() {
        let none = SimdCapabilities::default();
        assert!(!none.has_int8_hardware_acceleration());

        let avx_vnni = SimdCapabilities {
            has_avxvnni: true,
            ..Default::default()
        };
        assert!(avx_vnni.has_int8_hardware_acceleration());

        let avx512_vnni = SimdCapabilities {
            has_avx512vnni: true,
            ..Default::default()
        };
        assert!(avx512_vnni.has_int8_hardware_acceleration());
    }

    #[test]
    fn test_any_avx512_logic() {
        let none = SimdCapabilities::default();
        assert!(!none.has_any_avx512());

        let avx512f = SimdCapabilities {
            has_avx512f: true,
            ..Default::default()
        };
        assert!(avx512f.has_any_avx512());

        let avx512vnni = SimdCapabilities {
            has_avx512vnni: true,
            ..Default::default()
        };
        assert!(avx512vnni.has_any_avx512());
    }

    #[test]
    fn test_simd_summary_formatting() {
        let caps = SimdCapabilities {
            has_avx2: true,
            has_fma: true,
            has_avx512f: false,
            has_avx512vnni: false,
            has_avxvnni: true,
        };
        assert_eq!(caps.summary(), "AVX2, FMA, AVX-VNNI");
        assert!(caps.has_int8_hardware_acceleration());
        assert!(!caps.has_any_avx512());
    }
}
