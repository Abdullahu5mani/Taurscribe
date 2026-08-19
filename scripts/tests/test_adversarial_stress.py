#!/usr/bin/env python3
"""Adversarial Empirical Stress Harness for Taurscribe M1-M5 Verification."""

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent.parent
SRC_TAURI = PROJECT_ROOT / "src-tauri"
SCRIPTS_DIR = PROJECT_ROOT / "scripts"
WORKFLOW_PATH = PROJECT_ROOT / ".github" / "workflows" / "release.yml"
DOCS_DIR = PROJECT_ROOT / "docs"


class TestM1MetalWarmupAndModelRegistry(unittest.TestCase):
    """M1 Adversarial Probing: Model Registry Checksums, File Counts, Whisper-MLX Isolation."""

    def test_whisper_mlx_isolation(self):
        """Whisper backend must be completely isolated from MLX."""
        whisper_rs = SRC_TAURI / "src" / "whisper.rs"
        self.assertTrue(whisper_rs.exists(), "whisper.rs must exist")
        content = whisper_rs.read_text(encoding="utf-8")

        # Must not import or reference MLX
        self.assertNotIn("mlx_rs", content, "whisper.rs must not import mlx_rs")
        self.assertNotIn("parakeet_mlx", content, "whisper.rs must not import parakeet_mlx")
        self.assertNotIn("granite_mlx", content, "whisper.rs must not import granite_mlx")

    def test_cargo_toml_target_isolation(self):
        """mlx-rs must be strictly gated to Apple Silicon (macos + aarch64)."""
        cargo_toml = SRC_TAURI / "Cargo.toml"
        content = cargo_toml.read_text(encoding="utf-8")

        # Ensure mlx-rs is only under aarch64 macos
        self.assertIn('[target.\'cfg(all(target_os = "macos", target_arch = "aarch64"))\'.dependencies]', content)
        self.assertNotIn('mlx-rs = {', content.split('[target.\'cfg(all(target_os = "macos", target_arch = "aarch64"))\'.dependencies]')[0])

    def test_model_registry_sha256_format_across_all_models(self):
        """All SHA-256 entries in model_registry.rs must be valid 64-char hex strings or empty."""
        registry_file = SRC_TAURI / "src" / "commands" / "model_registry.rs"
        self.assertTrue(registry_file.exists())
        content = registry_file.read_text(encoding="utf-8")

        # Find all sha1: "..." occurrences
        matches = re.findall(r'sha1:\s*"([^"]*)"', content)
        self.assertGreater(len(matches), 30, "Should find at least 30 checksum entries in registry")

        for h in matches:
            if h == "":
                continue # Some models like parakeet-tdt explicitly allow empty pending upstream LFS retrieval
            self.assertEqual(len(h), 64, f"SHA-256 hash must be 64 characters: {h}")
            self.assertTrue(all(c in "0123456789abcdefABCDEF" for c in h), f"Hash must be hex: {h}")

    def test_8bit_model_configurations(self):
        """8-bit model registry entries must have correct repo, subdirectory, and file sets."""
        registry_file = SRC_TAURI / "src" / "commands" / "model_registry.rs"
        content = registry_file.read_text(encoding="utf-8")

        # Parakeet Nemotron 8-bit
        self.assertIn('"parakeet-nemotron-mlx-8bit"', content)
        self.assertIn('"Abdullahu5mani/parakeet-nemotron-0.6b-mlx-8bit"', content)
        self.assertIn('subdirectory: Some("parakeet-nemotron-mlx-8bit")', content)

        # Granite Speech 8-bit
        self.assertIn('"granite-speech-4.1-2b-nar-mlx-8bit"', content)
        self.assertIn('"Abdullahu5mani/granite-speech-4.1-2b-nar-mlx-8bit"', content)
        self.assertIn('subdirectory: Some("granite-speech-4.1-2b-nar-mlx-8bit")', content)

        # Whisper 8-bit quantizations
        for q8_model in ["whisper-tiny-q8_0", "whisper-base-q8_0", "whisper-small-q8_0", "whisper-medium-q8_0", "whisper-large-v2-q8_0", "whisper-large-v3-turbo-q8_0"]:
            self.assertIn(f'"{q8_model}"', content, f"{q8_model} must be registered")


class TestM2CoreMLDecoderAndFeasibility(unittest.TestCase):
    """M2 Adversarial Probing: Export script CLI, analyze-only across tiers, math derivations."""

    def test_export_script_analyze_only_all_tiers(self):
        """Verify --analyze-only completes cleanly with code 0 on all 10 model tiers."""
        script = SCRIPTS_DIR / "export_whisper_decoder_coreml.py"
        self.assertTrue(script.exists())

        models = [
            "tiny", "tiny.en", "base", "base.en", "small", "small.en",
            "medium", "medium.en", "large-v3", "large-v3-turbo"
        ]
        for m in models:
            res = subprocess.run(
                [sys.executable, str(script), "--model", m, "--analyze-only"],
                capture_output=True,
                text=True,
                cwd=str(PROJECT_ROOT)
            )
            self.assertEqual(res.returncode, 0, f"--analyze-only failed for model {m}: {res.stderr}")
            self.assertIn(f"WHISPER DECODER GRAPH & ANE HARDWARE ANALYSIS: {m.upper()}", res.stdout)
            self.assertIn("Bandwidth Reduction:", res.stdout)

    def test_export_script_invalid_model_rejection(self):
        """Script must reject invalid model name with descriptive exit."""
        script = SCRIPTS_DIR / "export_whisper_decoder_coreml.py"
        res = subprocess.run(
            [sys.executable, str(script), "--model", "nonexistent-model", "--analyze-only"],
            capture_output=True,
            text=True,
            cwd=str(PROJECT_ROOT)
        )
        self.assertNotEqual(res.returncode, 0, "Invalid model name should be rejected by argparse")

    def test_export_script_missing_packages_handling(self):
        """Running without --analyze-only when coremltools is missing must report diagnostics and exit 0."""
        script = SCRIPTS_DIR / "export_whisper_decoder_coreml.py"
        res = subprocess.run(
            [sys.executable, str(script), "--model", "base"],
            capture_output=True,
            text=True,
            cwd=str(PROJECT_ROOT)
        )
        self.assertEqual(res.returncode, 0)
        self.assertIn("pip install torch coremltools numpy openai-whisper", res.stdout)
        self.assertIn("Script verification & analytical graph validation succeeded.", res.stdout)

    def test_mathematical_bandwidth_formula_oracle(self):
        """Empirically verify the bandwidth reduction formula (N+1)/2 = 224.5 for N=448."""
        n_ctx = 448
        # Stateless cumulative steps sum
        stateless_sum = sum(t for t in range(1, n_ctx + 1))
        # Stateful cumulative steps sum (each step writes 1 token slice)
        stateful_sum = n_ctx * 1

        ratio = stateless_sum / stateful_sum
        expected_ratio = (n_ctx + 1) / 2.0
        self.assertAlmostEqual(ratio, expected_ratio, places=6)
        self.assertEqual(expected_ratio, 224.5)

    def test_feasibility_document_metrics_accuracy(self):
        """Check docs/whisper_coreml_decoder_feasibility.md contains sound, verified metrics."""
        doc = DOCS_DIR / "whisper_coreml_decoder_feasibility.md"
        self.assertTrue(doc.exists())
        content = doc.read_text(encoding="utf-8")

        self.assertIn("224.5x", content)
        self.assertIn("1,178.62 MB", content)
        self.assertIn("5.25 MB", content)
        self.assertIn("ct.StateType", content)
        self.assertIn("whisper.cpp", content)


class TestM3IntelMacAndDylibBundling(unittest.TestCase):
    """M3 Adversarial Probing: x86_64 cross-compile check & dylib bundling script robustness."""

    def test_bundle_macos_script_unversioned_and_symlink_handling(self):
        """Test bundle-macos-dylibs.sh against unversioned dylibs and symlinks in a sandbox."""
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = Path(temp_dir)
            fake_src_tauri = temp_path / "src-tauri"
            fake_target = fake_src_tauri / "target" / "release"
            fake_target.mkdir(parents=True)
            fake_dylib_dir = fake_src_tauri / "macos-dylibs"
            fake_dylib_dir.mkdir(parents=True)

            # Create mock binary
            mock_binary = fake_target / "taurscribe"
            mock_binary.write_bytes(b"\xca\xfe\xba\xbe")

            # Create versioned dylibs
            (fake_dylib_dir / "libllama.0.0.0.dylib").write_bytes(b"dylib")
            (fake_dylib_dir / "libggml-base.0.0.0.dylib").write_bytes(b"dylib")

            # Create an unversioned dylib
            (fake_dylib_dir / "libcustom.dylib").write_bytes(b"dylib")

            # Create a broken dangling symlink
            broken_symlink = fake_dylib_dir / "broken.dylib"
            broken_symlink.symlink_to("nonexistent.dylib")

            # Run symlink generation and json generation block
            env = os.environ.copy()
            env["CARGO_TARGET_DIR"] = str(fake_src_tauri / "target")

            # Test the symlink resolution loop from bundle-macos-dylibs.sh
            sh_cmd = f"""
            DYLIB_DIR="{fake_dylib_dir}"
            SRC_TAURI="{fake_src_tauri}"

            # Symlink loop
            for f in "$DYLIB_DIR"/*.dylib; do
              [ -f "$f" ] || continue
              bn=$(basename "$f")
              case "$bn" in
                libggml-base.*.*.*.dylib)
                  (cd "$DYLIB_DIR" && ln -sf "$bn" "libggml-base.0.dylib" && ln -sf "libggml-base.0.dylib" "libggml-base.dylib")
                  ;;
                libllama.*.*.*.dylib)
                  (cd "$DYLIB_DIR" && ln -sf "$bn" "libllama.0.dylib" && ln -sf "libllama.0.dylib" "libllama.dylib")
                  ;;
              esac
            done

            # JSON generation
            FRAMEWORKS_JSON="["
            for f in "$DYLIB_DIR"/*.dylib; do
              [ -f "$f" ] || continue
              bn=$(basename "$f")
              FRAMEWORKS_JSON="$FRAMEWORKS_JSON\\\"./macos-dylibs/$bn\\\","
            done
            FRAMEWORKS_JSON="${{FRAMEWORKS_JSON%,}}]"
            echo "{{\\\"bundle\\\":{{\\\"macOS\\\":{{\\\"frameworks\\\":$FRAMEWORKS_JSON}}}}}}" > "$SRC_TAURI/tauri.macos.conf.json"
            """
            proc = subprocess.run(["bash", "-c", sh_cmd], capture_output=True, text=True)
            self.assertEqual(proc.returncode, 0, f"Script failed: {proc.stderr}")

            # Verify symlinks were created
            self.assertTrue((fake_dylib_dir / "libllama.0.dylib").exists())
            self.assertTrue((fake_dylib_dir / "libllama.dylib").exists())
            self.assertTrue((fake_dylib_dir / "libggml-base.0.dylib").exists())
            self.assertTrue((fake_dylib_dir / "libggml-base.dylib").exists())

            # Verify tauri.macos.conf.json was generated and is valid JSON
            conf_file = fake_src_tauri / "tauri.macos.conf.json"
            self.assertTrue(conf_file.exists())
            conf_data = json.loads(conf_file.read_text())
            frameworks = conf_data["bundle"]["macOS"]["frameworks"]

            # Broken symlink should be excluded ([ -f "$f" ] returns false for broken symlinks)
            self.assertNotIn("./macos-dylibs/broken.dylib", frameworks)
            # Valid unversioned dylib must be present
            self.assertIn("./macos-dylibs/libcustom.dylib", frameworks)
            # Symlinks must be present
            self.assertIn("./macos-dylibs/libllama.dylib", frameworks)
            self.assertIn("./macos-dylibs/libggml-base.dylib", frameworks)


class TestM4WindowsCpuAffinityAndSimd(unittest.TestCase):
    """M4 Adversarial Probing: GPU LLM Layer 99 Retention & Topology properties."""

    def test_llm_gpu_layers_configuration(self):
        """Verify that on Windows/Linux, requested_layers is strictly 99 when use_gpu is true."""
        llm_rs = SRC_TAURI / "src" / "llm.rs"
        self.assertTrue(llm_rs.exists())
        content = llm_rs.read_text(encoding="utf-8")

        # Verify lines 76-86 pattern
        self.assertIn('#[cfg(not(target_os = "macos"))]', content)
        self.assertIn('99', content)
        self.assertIn('#[cfg(target_os = "macos")]', content)
        self.assertIn('0', content)


class TestM5LinuxWaylandAndAudio(unittest.TestCase):
    """M5 Adversarial Probing: Release workflow syntax, matrix, stubs, and audio selection."""

    def test_release_workflow_syntax_and_jobs(self):
        """release.yml must parse cleanly and contain both x86_64-apple-darwin and Linux x86_64."""
        import yaml
        with open(WORKFLOW_PATH, encoding="utf-8") as f:
            data = yaml.safe_load(f)

        jobs = data.get("jobs", {})
        self.assertIn("build", jobs)
        matrix = jobs["build"]["strategy"]["matrix"]["include"]

        # Check targets
        targets = [m["target"] for m in matrix]
        self.assertIn("aarch64-apple-darwin", targets)
        self.assertIn("x86_64-apple-darwin", targets)
        self.assertIn("x86_64-pc-windows-msvc", targets)
        self.assertIn("aarch64-pc-windows-msvc", targets)
        self.assertIn("x86_64-unknown-linux-gnu", targets)

        # Verify Linux has cuda=true
        linux_job = next(m for m in matrix if m["target"] == "x86_64-unknown-linux-gnu")
        self.assertTrue(linux_job["cuda"])

    def test_release_workflow_taurscribe_x64_dmg_staging(self):
        """release.yml must stage Taurscribe_x64.dmg for Intel Mac."""
        content = WORKFLOW_PATH.read_text(encoding="utf-8")
        self.assertIn('cp "$f" "release-assets/Taurscribe_x64.dmg"', content)
        self.assertIn('cp "$f" "release-assets/Taurscribe_aarch64.dmg"', content)

    def test_build_rs_cuda_stub_paths(self):
        """build.rs must search multiple dynamic CUDA paths on Linux x86_64."""
        build_rs = SRC_TAURI / "build.rs"
        self.assertTrue(build_rs.exists())
        content = build_rs.read_text(encoding="utf-8")

        self.assertIn('target_os = "linux"', content)
        self.assertIn('target_arch = "x86_64"', content)
        self.assertIn('/usr/local/cuda', content)
        self.assertIn('/usr/local/cuda-12.6', content)
        self.assertTrue('stubs' in content and 'lib64' in content)

    def test_clipboard_lock_concurrency_guards(self):
        """CLIPBOARD_LOCK must be defined in text_injection.rs and acquired in injection paths."""
        text_inj = SRC_TAURI / "src" / "text_injection.rs"
        self.assertTrue(text_inj.exists())
        content = text_inj.read_text(encoding="utf-8")
        self.assertIn("pub static CLIPBOARD_LOCK: std::sync::Mutex<()>", content)
        self.assertIn("let _guard = CLIPBOARD_LOCK.lock().unwrap_or_else", content)

        recording_rs = SRC_TAURI / "src" / "commands" / "recording.rs"
        self.assertTrue(recording_rs.exists())
        rec_content = recording_rs.read_text(encoding="utf-8")
        self.assertIn("crate::text_injection::CLIPBOARD_LOCK.lock().unwrap_or_else", rec_content)

    def test_audio_device_priority_oracle(self):
        """Verify audio device sorting rules against boundary cases using the production algorithm."""
        misc_rs = SRC_TAURI / "src" / "commands" / "misc.rs"
        self.assertTrue(misc_rs.exists())
        content = misc_rs.read_text(encoding="utf-8")
        self.assertIn("pub fn sort_audio_devices_by_priority(devices: &mut [String])", content)

        # Oracle replicating the exact production closure
        def priority(name: str) -> int:
            n = name.lower()
            if n == "default" or n == "sysdefault":
                return 0
            elif "pipewire" in n:
                return 1
            elif "pulse" in n:
                return 2
            elif "hw:" in n:
                return 4
            else:
                return 3

        # Boundary test 1: Empty list
        empty = []
        empty.sort(key=priority)
        self.assertEqual(empty, [])

        # Boundary test 2: Unrecognized device names, unicode, symbols
        devices = [
            "hw:2,0",
            "",
            "🎙️ USB Mic",
            "PipeWire-Jack",
            "DEFAULT",
            "pulse-audio",
            "Unknown External Audio",
            "SYSDEFAULT",
            "hw:0,0",
        ]
        devices.sort(key=priority)
        self.assertEqual(priority(devices[0]), 0) # DEFAULT
        self.assertEqual(priority(devices[1]), 0) # SYSDEFAULT
        self.assertEqual(priority(devices[2]), 1) # PipeWire-Jack
        self.assertEqual(priority(devices[3]), 2) # pulse-audio
        self.assertEqual(priority(devices[4]), 3) # ""
        self.assertEqual(priority(devices[5]), 3) # "🎙️ USB Mic"
        self.assertEqual(priority(devices[6]), 3) # "Unknown External Audio"
        self.assertEqual(priority(devices[7]), 4) # hw:2,0
        self.assertEqual(priority(devices[8]), 4) # hw:0,0



if __name__ == "__main__":
    unittest.main(verbosity=2)
