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
 *   2. Detects DLLs in target/release/
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
  unlinkSync,
  writeFileSync,
} from "fs";
import { join } from "path";
import { spawnSync } from "child_process";

const root = join(import.meta.dir, "..");
const srcTauri = join(root, "src-tauri");
const releaseDir = join(srcTauri, "target", "release");
const windowsConfPath = join(srcTauri, "tauri.windows.conf.json");

// DLL patterns to bundle
const dllPatterns = [
  /^llama\.dll$/,
  /^ggml.*\.dll$/,
  /^DirectML\.dll$/,
  /^onnxruntime.*\.dll$/,
];

// Track copied DLLs for cleanup
const copiedDlls: string[] = [];

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

// Save original config before any changes
const originalWindowsConf = readFileSync(windowsConfPath, "utf8");

// Handle interrupts gracefully
process.on("SIGINT", () => {
  console.log("\n🧹 Interrupted, cleaning up...");
  cleanup();
  process.exit(1);
});

// ── Step 1: Build Rust binary ────────────────────────────────
console.log("\n🔨 Step 1: Building Rust binary...\n");
const cargoBuild = spawnSync("cargo", ["build", "--release"], {
  stdio: "inherit",
  cwd: srcTauri,
  shell: true,
});
if (cargoBuild.status !== 0) {
  console.error("❌ Cargo build failed");
  process.exit(1);
}

// ── Step 2: Detect DLLs ──────────────────────────────────────
console.log("\n📦 Step 2: Detecting DLLs...\n");
if (!existsSync(releaseDir)) {
  console.error(`❌ Release directory not found: ${releaseDir}`);
  process.exit(1);
}

const allFiles = readdirSync(releaseDir);
const foundDlls = allFiles.filter((file: string) =>
  dllPatterns.some((pattern) => pattern.test(file))
);

if (foundDlls.length === 0) {
  console.warn("⚠ No DLLs found to bundle");
} else {
  console.log(`Found ${foundDlls.length} DLL(s):`);
  foundDlls.forEach((dll: string) => console.log(`   ✓ ${dll}`));
}

// ── Step 3: Copy DLLs into src-tauri/ ────────────────────────
console.log("\n� Step 3: Copying DLLs into src-tauri/...\n");
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
const tauriBuild = spawnSync("bunx", ["tauri", "build", ...extraArgs], {
  stdio: "inherit",
  cwd: root,
  shell: true,
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
