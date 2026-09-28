#!/usr/bin/env bun
/**
 * Windows Build Script for Taurscribe
 *
 * Problem: DLLs from llama-cpp-2 (dynamic-link) and ONNX Runtime only exist
 * after Cargo compiles. Tauri validates resource paths during its build, so
 * the DLLs must physically exist in a location Tauri can find them.
 *
 * Solution (from community / GitHub issues):
 *   Resource paths are resolved relative to src-tauri/.
 *   DLLs must be INSIDE src-tauri/ (not ../target/release/).
 *   Use object notation {"dll": "."} to place them next to the exe.
 *
 * This script:
 *   1. Builds the Rust binary via `cargo build --release`
 *   2. Copies llama.cpp DLLs from its Cargo build output into target/release/
 *      and detects the remaining runtime DLLs there
 *   3. Copies DLLs into src-tauri/ (so Tauri can find them)
 *   4. Writes resource map into tauri.windows.conf.json
 *   5. Runs `tauri build` (cargo is already up-to-date, so it's fast)
 *   6. Cleans up: removes copied DLLs and restores config
 */

import {
  copyFileSync,
  existsSync,
  readdirSync,
  readFileSync,
  statSync,
  unlinkSync,
  writeFileSync,
} from "fs";
import { join } from "path";
import { spawnSync } from "child_process";

const root = join(import.meta.dir, "..");
const srcTauri = join(root, "src-tauri");

// Parse CLI args to get target triple (e.g. --target x86_64-pc-windows-msvc)
const args = process.argv.slice(2);
const targetIdx = args.indexOf("--target");
const targetTriple = targetIdx !== -1 && args[targetIdx + 1] ? args[targetIdx + 1] : null;
const featuresIdx = args.indexOf("--features");
const features = featuresIdx !== -1 ? args[featuresIdx + 1] : null;

// Determine release directory based on --target and CARGO_TARGET_DIR
const targetDir = process.env.CARGO_TARGET_DIR || join(srcTauri, "target");
const releaseDir = targetTriple
  ? join(targetDir, targetTriple, "release")
  : join(targetDir, "release");

// llama-cpp-sys-2 0.1.157 builds these DLLs in out/bin but its Windows
// build script looks for *.lib there when copying shared libraries to Cargo's
// release directory. The app links, but an installer without llama.dll cannot
// start it.
const requiredLlamaDlls = ["llama.dll", "ggml.dll", "ggml-base.dll", "ggml-cpu.dll"];

function stageLlamaDlls() {
  const buildDir = join(releaseDir, "build");
  if (!existsSync(buildDir)) {
    failAndExit(`Missing Cargo build directory: ${buildDir}`);
  }

  const candidates = readdirSync(buildDir)
    .filter((name) => name.startsWith("llama-cpp-sys-2-"))
    .map((name) => join(buildDir, name, "out", "bin"))
    .filter((dir) =>
      existsSync(dir) && requiredLlamaDlls.every((dll) => existsSync(join(dir, dll)))
    );
  if (candidates.length === 0) {
    failAndExit(
      `llama.cpp runtime DLLs were not found under ${buildDir}. ` +
      `Expected ${requiredLlamaDlls.join(", ")} in one out/bin directory.`
    );
  }

  // Multiple Cargo build hashes can remain after a feature change. The most
  // recently built llama.dll belongs to the build that just completed.
  const sourceDir = candidates.sort((a, b) =>
    statSync(join(b, "llama.dll")).mtimeMs - statSync(join(a, "llama.dll")).mtimeMs
  )[0];
  for (const dll of readdirSync(sourceDir).filter((name) =>
    /^llama\.dll$|^ggml.*\.dll$/i.test(name)
  )) {
    const source = join(sourceDir, dll);
    const destination = join(releaseDir, dll);
    // Cargo may already have hard-linked the same DLL here. Remove only the
    // release-directory link before copying so the build output stays intact.
    if (existsSync(destination)) unlinkSync(destination);
    copyFileSync(source, destination);
    console.log(`   ✓ ${dll} (from ${sourceDir})`);
  }
}

const windowsConfPath = join(srcTauri, "tauri.windows.conf.json");

// DLL patterns to bundle
const dllPatterns = [
  /^llama\.dll$/,
  /^ggml.*\.dll$/,
  /^DirectML\.dll$/,
  /^onnxruntime.*\.dll$/,
  // NVIDIA flavor: the CUDA runtime and cuBLAS (redistributable per the CUDA EULA).
  // nvcuda.dll itself comes with the NVIDIA driver.
  /^cudart64_.*\.dll$/,
  /^cublas64_.*\.dll$/,
  /^cublasLt64_.*\.dll$/,
];

/** The NVIDIA flavor links CUDA's runtime DLLs at startup; ship them next to the exe. */
function stageGpuRuntimeDlls() {
  const nvidia = !!features && /\b(gpu-nvidia|windows-nvidia)\b/.test(features);
  if (!nvidia) return;
  const cudaPath = process.env.CUDA_PATH;
  if (!cudaPath) failAndExit("❌ gpu-nvidia build: CUDA_PATH is not set, so the CUDA runtime DLLs can't be bundled");
  const binDir = join(cudaPath, "bin");
  const wanted = [/^cudart64_\d+\.dll$/i, /^cublas64_\d+\.dll$/i, /^cublasLt64_\d+\.dll$/i];
  const found = readdirSync(binDir).filter((f) => wanted.some((w) => w.test(f)));
  for (const w of wanted) {
    if (!found.some((f) => w.test(f))) failAndExit(`❌ gpu-nvidia build: no ${w} in ${binDir}`);
  }
  for (const dll of found) {
    copyFileSync(join(binDir, dll), join(releaseDir, dll));
    console.log(`   ✓ ${dll} (CUDA runtime)`);
  }
}

// Track copied DLLs for cleanup
const copiedDlls: string[] = [];

// Save original config before any changes
const originalWindowsConf = readFileSync(windowsConfPath, "utf8");

function cleanup() {
  // Remove copied DLLs from src-tauri/
  for (const dll of copiedDlls) {
    const dest = join(srcTauri, dll);
    try {
      if (existsSync(dest)) unlinkSync(dest);
    } catch {}
  }
  // Restore original config
  try {
    writeFileSync(windowsConfPath, originalWindowsConf);
  } catch {}
}

function failAndExit(message: string): never {
  console.error(message);
  cleanup();
  process.exit(1);
}

// Handle interrupts gracefully
process.on("SIGINT", () => {
  console.log("\n🧹 Interrupted, cleaning up...");
  cleanup();
  process.exit(1);
});

// Bootstrap: first cargo build must not depend on preexisting DLL resource files.
const bootstrapWindowsConf = JSON.parse(originalWindowsConf);
bootstrapWindowsConf.bundle = bootstrapWindowsConf.bundle || {};
bootstrapWindowsConf.bundle.resources = [];
writeFileSync(windowsConfPath, JSON.stringify(bootstrapWindowsConf, null, 2) + "\n");

// ── Step 1: Build Rust binary ────────────────────────────────
console.log("\n🔨 Step 1: Building Rust binary...\n");
const cargoArgs = ["build", "--release"];
if (targetTriple) {
  cargoArgs.push("--target", targetTriple);
  console.log(`Building for target: ${targetTriple}`);
}
if (features) {
  cargoArgs.push("--features", features);
}
const cargoBuild = spawnSync("cargo", cargoArgs, {
  stdio: "inherit",
  cwd: srcTauri,
  shell: true,
});
if (cargoBuild.status !== 0) {
  failAndExit("❌ Cargo build failed");
}

// ── Step 2: Detect DLLs ──────────────────────────────────────
console.log("\n📦 Step 2: Detecting DLLs...\n");
if (!existsSync(releaseDir)) {
  failAndExit(`❌ Release directory not found: ${releaseDir}`);
}
stageLlamaDlls();

stageGpuRuntimeDlls();

const allFiles = readdirSync(releaseDir);
const foundDlls = allFiles.filter((file: string) =>
  dllPatterns.some((pattern) => pattern.test(file))
);

if (foundDlls.length === 0) {
  failAndExit("No runtime DLLs found to bundle");
} else {
  console.log(`Found ${foundDlls.length} DLL(s):`);
  foundDlls.forEach((dll: string) => console.log(`   ✓ ${dll}`));
}

// ── Step 3: Copy DLLs into src-tauri/ ────────────────────────
console.log("\n📁 Step 3: Copying DLLs into src-tauri/...\n");
for (const dll of foundDlls) {
  const src = join(releaseDir, dll);
  const dest = join(srcTauri, dll);
  copyFileSync(src, dest);
  copiedDlls.push(dll);
  console.log(`   ✓ ${dll}`);
}

// ── Step 4: Write resource map to tauri.windows.conf.json ────
console.log("\n📝 Step 4: Updating tauri.windows.conf.json...\n");

// Use array notation - plain filenames with no path prefix means they
// end up in the root of the resource dir, which on Windows NSIS installs
// is the same directory as the exe.
const resources: string[] = foundDlls.map((dll: string) => dll);

const windowsConf = JSON.parse(originalWindowsConf);
windowsConf.bundle = windowsConf.bundle || {};
windowsConf.bundle.resources = resources;
writeFileSync(windowsConfPath, JSON.stringify(windowsConf, null, 2) + "\n");
console.log(`Updated with ${foundDlls.length} resource(s)`);

// ── Step 5: Run tauri build ──────────────────────────────────
console.log("\n🚀 Step 5: Running tauri build...\n");

// Get CLI args passed to this script (e.g. --bundles nsis)
const extraArgs = process.argv.slice(2);
// shell:false — on Windows, shell:true mangles --config JSON (quotes stripped → invalid JSON).
const tauriBuild = spawnSync("bunx", ["tauri", "build", ...extraArgs], {
  stdio: "inherit",
  cwd: root,
  shell: false,
});

// ── Step 6: Clean up ─────────────────────────────────────────
console.log("\n🧹 Step 6: Cleaning up...\n");
cleanup();
console.log("Removed copied DLLs and restored config");

if (tauriBuild.status !== 0) {
  console.error("❌ Tauri build failed");
  process.exit(1);
}

console.log("\n✅ Windows build complete!\n");
