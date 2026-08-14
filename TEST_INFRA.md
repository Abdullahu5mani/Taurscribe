# E2E Test Infra: Taurscribe Cross-Platform & Performance Stabilization

## Test Philosophy
- Opaque-box, requirement-driven verification derived directly from `ORIGINAL_REQUEST.md`.
- No reliance on internal implementation hacks or non-reproducible mocking.
- Systematic 4-tier coverage methodology:
  - **Tier 1: Feature Coverage** (≥5 tests per feature covering isolated happy-paths).
  - **Tier 2: Boundary & Corner Cases** (≥5 tests per feature covering limits, missing hardware, fallback paths, empty inputs).
  - **Tier 3: Cross-Feature Combinations** (pairwise coverage of feature interactions, e.g. INT8 quantization + AVX-VNNI dispatch, Wayland injection + PipeWire audio).
  - **Tier 4: Real-World Workload Scenarios** (realistic dictation sessions, CI workflow execution, multi-core sustained processing).

---

## Feature Inventory & Test Coverage Goals
| # | Feature | Requirement | Tier 1 | Tier 2 | Tier 3 |
|---|---------|-------------|:------:|:------:|:------:|
| 1 | Metal Shader Cache Warmup | ORIGINAL_REQUEST §R1 | 5 | 5 | ✓ |
| 2 | Model Registry Quantization | ORIGINAL_REQUEST §R1 | 5 | 5 | ✓ |
| 3 | Whisper CoreML Preservation | ORIGINAL_REQUEST §R1 | 5 | 5 | ✓ |
| 4 | CoreML ANE Decoder Graph Investigation | ORIGINAL_REQUEST §R2 | 5 | 5 | ✓ |
| 5 | CoreML Decoder Generation & Benchmarks | ORIGINAL_REQUEST §R2 | 5 | 5 | ✓ |
| 6 | Intel macOS CI Release Matrix | ORIGINAL_REQUEST §R3 | 5 | 5 | ✓ |
| 7 | Intel macOS Dylib Bundling | ORIGINAL_REQUEST §R3 | 5 | 5 | ✓ |
| 8 | Taurscribe_x64.dmg Release Artifact | ORIGINAL_REQUEST §R3 | 5 | 5 | ✓ |
| 9 | Windows Hybrid P-Core Affinity | ORIGINAL_REQUEST §R4 | 5 | 5 | ✓ |
| 10 | Windows SIMD Runtime Dispatch | ORIGINAL_REQUEST §R4 | 5 | 5 | ✓ |
| 11 | Windows GPU LLM Retention | ORIGINAL_REQUEST §R4 | 5 | 5 | ✓ |
| 12 | Linux Wayland Input Injection | ORIGINAL_REQUEST §R5 | 5 | 5 | ✓ |
| 13 | Linux PipeWire Audio Pipeline | ORIGINAL_REQUEST §R5 | 5 | 5 | ✓ |
| 14 | Linux CI Build Re-enablement | ORIGINAL_REQUEST §R5 | 5 | 5 | ✓ |
| 15 | Cross-Target Compilation Check | Acceptance Criteria | 5 | 5 | ✓ |
| 16 | Leftover Items Hardware Audit | Acceptance Criteria | 5 | 5 | ✓ |

---

## Test Architecture
- **Test Runner**: Custom Rust and script harness located in `src-tauri/tests/` and `scripts/tests/`.
- **Invocation**:
  - `cargo test --test platform_optimizations`
  - `python3 scripts/tests/test_ci_release_matrix.py`
  - `python3 scripts/tests/test_coreml_decoder_feasibility.py`
- **Output Format**: Standard test runner exit code 0 on pass, non-zero on failure with descriptive assertions.

---

## Real-World Application Scenarios (Tier 4)
| # | Scenario | Features Exercised | Target Environment |
|---|----------|--------------------|-------------------|
| 1 | Cold Start Metal Dictation Warmup | F1, F2, F3 | macOS Apple Silicon |
| 2 | Intel Mac CI Build & Packaging Matrix | F6, F7, F8, F14 | GitHub Actions / macOS x86_64 |
| 3 | Windows Sustained ASR with P-Core Pinning | F9, F10, F11 | Windows Hybrid CPU |
| 4 | Linux Wayland Dictation & PipeWire Capture | F12, F13, F14 | Linux GNOME/KDE Wayland |
| 5 | Multi-Engine Model Registry & Quantized Weights Verification | F2, F3, F4, F5 | Cross-Platform |

---

## Coverage Thresholds
- **Tier 1**: ≥80 tests (16 features × 5)
- **Tier 2**: ≥80 tests (16 features × 5)
- **Tier 3**: Pairwise coverage of all major platform-feature combinations
- **Tier 4**: ≥5 end-to-end workload test cases
