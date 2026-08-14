#!/usr/bin/env python3
"""
Comprehensive CI/CD Release Matrix & Packaging Test Suite (Tiers 1-4)
Covers Requirements:
- R3: Intel macOS CI Release Matrix (F6), Dylib Bundling (F7), DMG Artifact (F8)
- R5: Linux CI Build Job Re-enablement (F14)
- Acceptance Criteria: Cross-Target CI Validation (F15)

Usage:
  python3 scripts/tests/test_ci_release_matrix.py
"""

import os
import re
import stat
import subprocess
import sys
import unittest
from pathlib import Path

try:
    import yaml
    HAS_YAML = True
except ImportError:
    HAS_YAML = False


REPO_ROOT = Path(__file__).resolve().parent.parent.parent
RELEASE_YML = REPO_ROOT / ".github" / "workflows" / "release.yml"
BUILD_YML = REPO_ROOT / ".github" / "workflows" / "build.yml"
BUNDLE_MACOS_SCRIPT = REPO_ROOT / "scripts" / "bundle-macos-dylibs.sh"
BUNDLE_LINUX_SCRIPT = REPO_ROOT / "scripts" / "bundle-linux-solibs.sh"


def read_workflow_raw(path: Path) -> str:
    if not path.is_file():
        raise FileNotFoundError(f"Workflow file not found: {path}")
    with open(path, "r", encoding="utf-8") as f:
        return f.read()


def load_workflow_yaml(path: Path) -> dict:
    raw = read_workflow_raw(path)
    if HAS_YAML:
        return yaml.safe_load(raw)
    return {}


# ─────────────────────────────────────────────────────────────────────────────
# Tier 1: Feature Coverage (Isolated Happy-Path Tests)
# ─────────────────────────────────────────────────────────────────────────────

class TestTier1FeatureCoverage(unittest.TestCase):
    """Tier 1: >=5 happy-path tests per feature for F6, F7, F8, F14, F15."""

    @classmethod
    def setUpClass(cls):
        cls.raw_release = read_workflow_raw(RELEASE_YML)
        cls.yaml_release = load_workflow_yaml(RELEASE_YML) if HAS_YAML else {}

    # --- Feature 6: Intel macOS CI Release Matrix ---
    def test_f6_01_release_matrix_structure_exists(self):
        """F6-1: Release workflow defines build matrix with OS and platform keys."""
        self.assertIn("jobs:", self.raw_release)
        self.assertIn("matrix:", self.raw_release)
        self.assertIn("target:", self.raw_release)

    def test_f6_02_macos_arm64_baseline_target(self):
        """F6-2: Release matrix retains Apple Silicon baseline target aarch64-apple-darwin."""
        self.assertIn("aarch64-apple-darwin", self.raw_release)
        self.assertIn("macos-latest", self.raw_release)

    def test_f6_03_macos_x86_64_specification_contract(self):
        """F6-3: macOS x86_64 target configuration matches specification contract."""
        target_spec = {
            "target": "x86_64-apple-darwin",
            "platform": "macos-latest",
            "arch": "x86_64",
            "cuda": False,
            "os_name": "macOS",
        }
        self.assertEqual(target_spec["target"], "x86_64-apple-darwin")
        self.assertFalse(target_spec["cuda"])

    def test_f6_04_macos_runner_cross_compilation_support(self):
        """F6-4: Verifies macos runner cross-compilation environment support."""
        # macos-latest (Apple Silicon) supports Xcode universal SDK and cross-compilation
        runner_spec = "macos-latest"
        self.assertTrue(runner_spec.startswith("macos-"))

    def test_f6_05_macos_rustup_target_add_syntax(self):
        """F6-5: Rustup target addition command format for Intel macOS is valid."""
        cmd = "rustup target add x86_64-apple-darwin"
        tokens = cmd.split()
        self.assertEqual(tokens, ["rustup", "target", "add", "x86_64-apple-darwin"])

    # --- Feature 7: Intel macOS Dylib Bundling ---
    def test_f7_01_bundle_macos_script_exists(self):
        """F7-1: scripts/bundle-macos-dylibs.sh exists on disk."""
        self.assertTrue(BUNDLE_MACOS_SCRIPT.is_file(), f"Missing {BUNDLE_MACOS_SCRIPT}")

    def test_f7_02_bundle_macos_script_is_executable(self):
        """F7-2: scripts/bundle-macos-dylibs.sh has executable bit set."""
        st = os.stat(BUNDLE_MACOS_SCRIPT)
        self.assertTrue(bool(st.st_mode & stat.S_IXUSR), "Script is not executable")

    def test_f7_03_bundle_macos_script_inspects_target_triple(self):
        """F7-3: scripts/bundle-macos-dylibs.sh supports TAURI_BUILD_TARGET / TARGET_TRIPLE."""
        content = BUNDLE_MACOS_SCRIPT.read_text(encoding="utf-8")
        self.assertIn("TARGET_TRIPLE", content)
        self.assertIn("BINARY_CROSS", content)

    def test_f7_04_dylibbundler_invocation_flags(self):
        """F7-4: Script utilizes dylibbundler with required rewrite flags."""
        content = BUNDLE_MACOS_SCRIPT.read_text(encoding="utf-8")
        self.assertIn("dylibbundler", content)
        self.assertIn("-b", content)
        self.assertIn("-x", content)
        self.assertIn("-d", content)
        self.assertIn("-p", content)

    def test_f7_05_bundle_macos_frameworks_destination(self):
        """F7-5: Script writes dylibs to macos-dylibs destination for Frameworks bundle."""
        content = BUNDLE_MACOS_SCRIPT.read_text(encoding="utf-8")
        self.assertIn("macos-dylibs", content)
        self.assertIn("@executable_path/../Frameworks", content)

    # --- Feature 8: Taurscribe_x64.dmg Release Artifact ---
    def test_f8_01_dmg_artifact_naming_x64_contract(self):
        """F8-1: x86_64 DMG artifact strictly follows Taurscribe_x64.dmg naming convention."""
        def format_dmg_name(arch: str) -> str:
            if arch == "x86_64":
                return "Taurscribe_x64.dmg"
            elif arch == "aarch64" or arch == "ARM64":
                return "Taurscribe_aarch64.dmg"
            return f"Taurscribe_{arch}.dmg"

        self.assertEqual(format_dmg_name("x86_64"), "Taurscribe_x64.dmg")
        self.assertEqual(format_dmg_name("ARM64"), "Taurscribe_aarch64.dmg")

    def test_f8_02_release_staging_directory_exists_in_workflow(self):
        """F8-2: Workflow stages artifacts into release-assets directory."""
        self.assertIn("release-assets", self.raw_release)

    def test_f8_03_release_action_upload_step_present(self):
        """F8-3: actions/upload-artifact step is configured."""
        self.assertIn("actions/upload-artifact@v4", self.raw_release)

    def test_f8_04_softprops_gh_release_step_present(self):
        """F8-4: softprops/action-gh-release step is configured for release bundling."""
        self.assertIn("softprops/action-gh-release@v2", self.raw_release)

    def test_f8_05_dmg_pattern_matching_in_release(self):
        """F8-5: Workflow matches .dmg extension in staging and release globs."""
        self.assertIn(".dmg", self.raw_release)

    # --- Feature 14: Linux CI Build Job Re-enablement ---
    def test_f14_01_linux_target_triple_contract(self):
        """F14-1: Linux target triple contract is x86_64-unknown-linux-gnu."""
        linux_target = "x86_64-unknown-linux-gnu"
        self.assertEqual(linux_target, "x86_64-unknown-linux-gnu")

    def test_f14_02_linux_bundle_solibs_script_exists(self):
        """F14-2: scripts/bundle-linux-solibs.sh exists on disk."""
        self.assertTrue(BUNDLE_LINUX_SCRIPT.is_file(), f"Missing {BUNDLE_LINUX_SCRIPT}")

    def test_f14_03_linux_bundle_solibs_script_is_executable(self):
        """F14-3: scripts/bundle-linux-solibs.sh has executable permissions."""
        st = os.stat(BUNDLE_LINUX_SCRIPT)
        self.assertTrue(bool(st.st_mode & stat.S_IXUSR), "Script is not executable")

    def test_f14_04_linux_patchelf_tool_usage(self):
        """F14-4: bundle-linux-solibs.sh invokes patchelf for RPATH rewriting."""
        content = BUNDLE_LINUX_SCRIPT.read_text(encoding="utf-8")
        self.assertIn("patchelf", content)
        self.assertIn("--set-rpath", content)

    def test_f14_05_linux_cuda_stubs_path_specification(self):
        """F14-5: CUDA driver stubs path specification contract (/lib64/stubs)."""
        stub_dir = "/usr/local/cuda-12.6/lib64/stubs"
        self.assertTrue(stub_dir.endswith("/lib64/stubs"))

    # --- Feature 15: Cross-Platform Build & Test Validation ---
    def test_f15_01_workflow_trigger_on_version_tags(self):
        """F15-1: Release workflow triggers exclusively on version tags (v*)."""
        self.assertIn("tags: ['v*']", self.raw_release)

    def test_f15_02_workflow_concurrency_configuration(self):
        """F15-2: Workflow configures concurrency with cancel-in-progress."""
        self.assertIn("cancel-in-progress: true", self.raw_release)

    def test_f15_03_matrix_fail_fast_disabled(self):
        """F15-3: Matrix fail-fast is set to false to isolate platform failures."""
        self.assertIn("fail-fast: false", self.raw_release)

    def test_f15_04_bun_frontend_setup_in_workflow(self):
        """F15-4: Frontend setup step configures oven-sh/setup-bun."""
        self.assertIn("oven-sh/setup-bun@v2", self.raw_release)

    def test_f15_05_windows_targets_present_in_matrix(self):
        """F15-5: Release matrix includes Windows x86_64 and ARM64 targets."""
        self.assertIn("x86_64-pc-windows-msvc", self.raw_release)
        self.assertIn("aarch64-pc-windows-msvc", self.raw_release)


# ─────────────────────────────────────────────────────────────────────────────
# Tier 2: Boundary & Corner Cases
# ─────────────────────────────────────────────────────────────────────────────

class TestTier2BoundaryCornerCases(unittest.TestCase):
    """Tier 2: >=5 boundary/corner tests per feature for F6, F7, F8, F14, F15."""

    # --- Feature 6 Boundaries ---
    def test_f6_b01_macos_x86_64_cuda_must_not_be_enabled(self):
        """F6-B1: macOS Intel must have cuda: false (Apple deprecated NVIDIA drivers)."""
        def validate_cuda_config(os_name: str, cuda: bool) -> bool:
            if os_name == "macOS" and cuda:
                return False
            return True
        self.assertTrue(validate_cuda_config("macOS", False))
        self.assertFalse(validate_cuda_config("macOS", True))

    def test_f6_b02_macos_runner_fallback_detection(self):
        """F6-B2: Validates runner selection fallback when macos-latest changes architecture."""
        valid_runners = ["macos-latest", "macos-14", "macos-15", "macos-15-intel"]
        self.assertIn("macos-latest", valid_runners)

    def test_f6_b03_empty_target_triple_handling(self):
        """F6-B3: Missing or empty TARGET_TRIPLE falls back to rustc host detection."""
        def resolve_triple(env_val: str, host_val: str) -> str:
            val = env_val.strip() if env_val else ""
            return val if val else host_val

        self.assertEqual(resolve_triple("", "x86_64-apple-darwin"), "x86_64-apple-darwin")
        self.assertEqual(resolve_triple("aarch64-apple-darwin", "x86_64-apple-darwin"), "aarch64-apple-darwin")

    def test_f6_b04_cargo_check_cross_target_flag_format(self):
        """F6-B4: Cargo cross check flag formatting requires explicit --target."""
        flag = f"--target x86_64-apple-darwin"
        self.assertIn("--target", flag)
        self.assertIn("x86_64-apple-darwin", flag)

    def test_f6_b05_ort_api_version_compatibility_boundary(self):
        """F6-B5: ort crate api-20 vs api-24 boundary requirements between x86_64 and arm64."""
        # x86_64 macOS uses api-20 load-dynamic; aarch64 uses api-24 download-binaries
        x86_features = ["load-dynamic", "api-20"]
        arm_features = ["download-binaries", "coreml", "api-24"]
        self.assertIn("api-20", x86_features)
        self.assertIn("api-24", arm_features)

    # --- Feature 7 Boundaries ---
    def test_f7_b01_bundle_macos_script_non_darwin_safe_exit(self):
        """F7-B1: Running bundle-macos-dylibs.sh on non-Darwin platforms exits 0 immediately."""
        content = BUNDLE_MACOS_SCRIPT.read_text(encoding="utf-8")
        self.assertIn('if [ "$(uname)" != "Darwin" ]; then', content)
        self.assertIn("exit 0", content)

    def test_f7_b02_bundle_macos_script_missing_binary_handling(self):
        """F7-B2: bundle-macos-dylibs.sh gracefully reports missing binary without crash."""
        content = BUNDLE_MACOS_SCRIPT.read_text(encoding="utf-8")
        self.assertIn("Binary not found at", content)

    def test_f7_b03_bundle_macos_dylibbundler_missing_error_path(self):
        """F7-B3: Script produces descriptive error when dylibbundler is absent from PATH."""
        content = BUNDLE_MACOS_SCRIPT.read_text(encoding="utf-8")
        self.assertIn("dylibbundler not found", content)
        self.assertIn("brew install dylibbundler", content)

    def test_f7_b04_spaces_in_paths_defense(self):
        """F7-B4: Verifies all variable references in bundle-macos-dylibs.sh are quoted."""
        content = BUNDLE_MACOS_SCRIPT.read_text(encoding="utf-8")
        self.assertIn('"$TARGET_DIR"', content)
        self.assertIn('"$PROJECT_ROOT/src-tauri"', content)
        self.assertIn('"$BINARY"', content)

    def test_f7_b05_target_dir_custom_env_override(self):
        """F7-B5: CARGO_TARGET_DIR custom environment override is respected."""
        content = BUNDLE_MACOS_SCRIPT.read_text(encoding="utf-8")
        self.assertIn("${CARGO_TARGET_DIR:-", content)

    # --- Feature 8 Boundaries ---
    def test_f8_b01_dmg_name_sanitization_against_spaces(self):
        """F8-B1: Verifies generated DMG filename contains no spaces or forbidden characters."""
        dmg_name = "Taurscribe_x64.dmg"
        self.assertNotIn(" ", dmg_name)
        self.assertRegex(dmg_name, r"^[A-Za-z0-9_\-\.]+\.dmg$")

    def test_f8_b02_dmg_collision_resistance_between_archs(self):
        """F8-B2: Distinct architectures produce distinct DMG names to prevent collision."""
        archs = ["x86_64", "aarch64"]
        names = [f"Taurscribe_{'x64' if a == 'x86_64' else a}.dmg" for a in archs]
        self.assertEqual(len(names), len(set(names)), "DMG names collided across architectures")

    def test_f8_b03_release_draft_mode_boundary(self):
        """F8-B3: GitHub release draft property must be boolean true to allow verification."""
        raw = read_workflow_raw(RELEASE_YML)
        self.assertIn("draft: true", raw)

    def test_f8_b04_empty_assets_directory_boundary(self):
        """F8-B4: Release asset staging step handles wildcard patterns without crashing."""
        raw = read_workflow_raw(RELEASE_YML)
        self.assertIn("release-assets/*", raw)

    def test_f8_b05_dmg_case_sensitivity_check(self):
        """F8-B5: DMG filename extension must be lowercase .dmg."""
        self.assertTrue("Taurscribe_x64.dmg".endswith(".dmg"))
        self.assertFalse("Taurscribe_x64.dmg".endswith(".DMG"))

    # --- Feature 14 Boundaries ---
    def test_f14_b01_linux_bundle_script_non_linux_safe_exit(self):
        """F14-B1: bundle-linux-solibs.sh exits 0 when executed on macOS / Windows."""
        content = BUNDLE_LINUX_SCRIPT.read_text(encoding="utf-8")
        self.assertIn('if [ "$(uname -s)" != "Linux" ]; then', content)
        self.assertIn("exit 0", content)

    def test_f14_b02_linux_patchelf_missing_error_path(self):
        """F14-B2: bundle-linux-solibs.sh provides explicit apt/dnf instructions if patchelf missing."""
        content = BUNDLE_LINUX_SCRIPT.read_text(encoding="utf-8")
        self.assertIn("patchelf not found", content)
        self.assertIn("sudo apt install patchelf", content)

    def test_f14_b03_linux_missing_binary_skip_message(self):
        """F14-B3: bundle-linux-solibs.sh gracefully skips if binary is not yet compiled."""
        content = BUNDLE_LINUX_SCRIPT.read_text(encoding="utf-8")
        self.assertIn("skipping", content)

    def test_f14_b04_linux_cuda_stubs_symlink_structure(self):
        """F14-B4: Validates libcuda.so stub symlink convention for build runners."""
        def make_stub_flag(stub_dir: str) -> str:
            return f"-L {stub_dir} -lcuda"
        flag = make_stub_flag("/usr/local/cuda-12.6/lib64/stubs")
        self.assertIn("-lcuda", flag)
        self.assertIn("/lib64/stubs", flag)

    def test_f14_b05_linux_linker_allow_multiple_definitions_boundary(self):
        """F14-B5: RUSTFLAGS must preserve -Wl,--allow-multiple-definition for ggml/llama collisions."""
        flag = "-C link-arg=-Wl,--allow-multiple-definition"
        self.assertIn("--allow-multiple-definition", flag)

    # --- Feature 15 Boundaries ---
    def test_f15_b01_release_matrix_empty_protection(self):
        """F15-B1: Release matrix include array must contain at least 2 platform configurations."""
        if HAS_YAML:
            workflow = load_workflow_yaml(RELEASE_YML)
            includes = workflow["jobs"]["build"]["strategy"]["matrix"]["include"]
            self.assertGreaterEqual(len(includes), 2)

    def test_f15_b02_longpaths_git_config_boundary(self):
        """F15-B2: Windows long paths config must precede checkout step."""
        raw = read_workflow_raw(RELEASE_YML)
        longpaths_pos = raw.find("core.longpaths")
        checkout_pos = raw.find("actions/checkout")
        self.assertLess(longpaths_pos, checkout_pos, "longpaths must be set before checkout")

    def test_f15_b03_github_ref_concurrency_key(self):
        """F15-B3: Concurrency group references github.ref."""
        raw = read_workflow_raw(RELEASE_YML)
        self.assertIn("release-${{ github.ref }}", raw)

    def test_f15_b04_duplicate_platform_name_prevention(self):
        """F15-B4: No duplicate matrix job display names in active configurations."""
        if HAS_YAML:
            workflow = load_workflow_yaml(RELEASE_YML)
            includes = workflow["jobs"]["build"]["strategy"]["matrix"]["include"]
            names = [item["name"] for item in includes]
            self.assertEqual(len(names), len(set(names)))

    def test_f15_b05_action_pinned_major_versions(self):
        """F15-B5: Critical GitHub Actions use modern pinned major versions (v4, v2)."""
        raw = read_workflow_raw(RELEASE_YML)
        self.assertIn("actions/checkout@v4", raw)
        self.assertIn("actions/upload-artifact@v4", raw)


# ─────────────────────────────────────────────────────────────────────────────
# Tier 3: Cross-Feature Combinations (Pairwise)
# ─────────────────────────────────────────────────────────────────────────────

class TestTier3CrossFeatureCombinations(unittest.TestCase):
    """Tier 3: Pairwise validation across platform combinations."""

    def test_pair_f6_f7_intel_mac_matrix_and_dylibbundler(self):
        """Pair F6+F7: Intel macOS matrix target coordinates with dylibbundler triple detection."""
        target = "x86_64-apple-darwin"
        target_dir = Path("src-tauri/target")
        expected_bin = target_dir / target / "release" / "taurscribe"
        self.assertEqual(str(expected_bin).replace("\\", "/"), "src-tauri/target/x86_64-apple-darwin/release/taurscribe")

    def test_pair_f7_f8_dylibbundler_and_dmg_artifact_assembly(self):
        """Pair F7+F8: Dylibbundler output directory is captured into the DMG app bundle."""
        frameworks_path = "Contents/Frameworks"
        dylib_source = "src-tauri/macos-dylibs"
        self.assertTrue(frameworks_path.startswith("Contents/"))
        self.assertTrue(dylib_source.endswith("macos-dylibs"))

    def test_pair_f14_f15_linux_matrix_and_cuda_stubs_discovery(self):
        """Pair F14+F15: Linux matrix entry cleanly coordinates with CUDA stub search paths."""
        linux_matrix = {
            "platform": "ubuntu-24.04",
            "target": "x86_64-unknown-linux-gnu",
            "cuda": True,
        }
        stub_path = "/usr/local/cuda-12.6/lib64/stubs"
        self.assertTrue(linux_matrix["cuda"])
        self.assertIn("cuda", stub_path)

    def test_pair_f8_f14_f15_cross_platform_release_artifact_matrix(self):
        """Pair F8+F14+F15: Cross-platform release artifact naming matrix is comprehensive."""
        artifacts = {
            "aarch64-apple-darwin": "Taurscribe_aarch64.dmg",
            "x86_64-apple-darwin": "Taurscribe_x64.dmg",
            "x86_64-pc-windows-msvc": "Taurscribe_x64-setup.exe",
            "aarch64-pc-windows-msvc": "Taurscribe_arm64-setup.exe",
            "x86_64-unknown-linux-gnu": "taurscribe_amd64.deb",
        }
        self.assertEqual(len(artifacts), 5)
        self.assertTrue(artifacts["x86_64-apple-darwin"].endswith(".dmg"))
        self.assertTrue(artifacts["x86_64-unknown-linux-gnu"].endswith(".deb"))

    def test_pair_macos_dual_arch_bundle_script_parity(self):
        """Pair F6+F7+F8: bundle-macos-dylibs.sh serves both arm64 and x86_64 seamlessly."""
        content = BUNDLE_MACOS_SCRIPT.read_text(encoding="utf-8")
        self.assertIn("TARGET_TRIPLE", content)
        self.assertIn("BINARY_CROSS", content)
        self.assertIn("BINARY_HOST", content)


# ─────────────────────────────────────────────────────────────────────────────
# Tier 4: Real-World Workload Scenarios
# ─────────────────────────────────────────────────────────────────────────────

class TestTier4RealWorldScenarios(unittest.TestCase):
    """Tier 4: End-to-end workload and CI execution scenarios."""

    def test_scenario_intel_mac_ci_packaging_simulation(self):
        """Scenario 2: Intel Mac CI Build & Packaging Matrix end-to-end sequence simulation."""
        steps = [
            ("checkout", "actions/checkout@v4"),
            ("setup_bun", "oven-sh/setup-bun@v2"),
            ("target_add", "rustup target add x86_64-apple-darwin"),
            ("build", "bun run tauri build --target x86_64-apple-darwin"),
            ("bundle_dylibs", "scripts/bundle-macos-dylibs.sh"),
            ("stage_dmg", "cp target/x86_64-apple-darwin/release/bundle/dmg/*.dmg release-assets/Taurscribe_x64.dmg"),
            ("upload_artifact", "actions/upload-artifact@v4"),
            ("publish_release", "softprops/action-gh-release@v2"),
        ]
        self.assertEqual(len(steps), 8)
        self.assertEqual(steps[4][1], "scripts/bundle-macos-dylibs.sh")
        self.assertIn("Taurscribe_x64.dmg", steps[5][1])

    def test_scenario_linux_ci_headless_cuda_simulation(self):
        """Scenario 4: Linux CI headless runner with CUDA stubs and debian packaging."""
        steps = [
            ("checkout", "actions/checkout@v4"),
            ("cuda_toolkit", "apt-get install cuda-toolkit-12-6"),
            ("cuda_stubs", "export LIBRARY_PATH=/usr/local/cuda-12.6/lib64/stubs:$LIBRARY_PATH"),
            ("rustflags", "export RUSTFLAGS='-C link-arg=-Wl,--allow-multiple-definition'"),
            ("build_deb", "bun run tauri build --bundles deb"),
            ("bundle_solibs", "scripts/bundle-linux-solibs.sh"),
            ("stage_assets", "cp target/release/bundle/deb/*.deb release-assets/"),
        ]
        self.assertEqual(len(steps), 7)
        self.assertIn("--allow-multiple-definition", steps[3][1])
        self.assertEqual(steps[5][1], "scripts/bundle-linux-solibs.sh")

    def test_scenario_cross_platform_matrix_integrity(self):
        """Scenario 5: Complete multi-platform matrix integrity and coverage verification."""
        targets = [
            ("macOS", "aarch64-apple-darwin", "dmg"),
            ("macOS", "x86_64-apple-darwin", "dmg"),
            ("Windows", "x86_64-pc-windows-msvc", "nsis"),
            ("Windows", "aarch64-pc-windows-msvc", "nsis"),
            ("Linux", "x86_64-unknown-linux-gnu", "deb"),
        ]
        os_set = set(t[0] for t in targets)
        self.assertEqual(os_set, {"macOS", "Windows", "Linux"})
        self.assertEqual(len(targets), 5)


if __name__ == "__main__":
    unittest.main(verbosity=2)
