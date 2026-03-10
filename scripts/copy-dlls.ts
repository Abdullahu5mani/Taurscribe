#!/usr/bin/env bun
// Windows DLL bundling script - runs BEFORE Tauri bundles the installer
// Ensures all runtime DLLs are next to the .exe so Tauri includes them

import { existsSync, copyFileSync } from "fs";
import { join } from "path";

// Platform check - only run on Windows
const isWindows = process.platform === "win32";
if (!isWindows) {
  console.log("⊘ Skipping DLL bundling (not Windows)");
  process.exit(0);
}

const releaseDir = join(process.cwd(), "src-tauri", "target", "release");
const exePath = join(releaseDir, "taurscribe.exe");

// Only run if the exe has been built
if (!existsSync(exePath)) {
  console.log("⊘ Skipping DLL bundling (taurscribe.exe not built yet)");
  process.exit(0);
}

// List of DLLs to bundle
const dlls = [
  "llama.dll",
  "ggml.dll",
  "ggml-base.dll",
  "ggml-cpu.dll",
  "DirectML.dll",
  "onnxruntime_providers_shared.dll",
  "onnxruntime_providers_cuda.dll",
  "onnxruntime_providers_tensorrt.dll",
];

console.log("📦 Ensuring DLLs are bundled with taurscribe.exe...\n");

// All DLLs should already be in releaseDir (built by cargo)
// Just verify they exist - Tauri will automatically include them
let found = 0;
let missing = 0;

dlls.forEach((dll) => {
  const dllPath = join(releaseDir, dll);
  if (existsSync(dllPath)) {
    console.log(`✓ Found ${dll}`);
    found++;
  } else {
    console.warn(`⚠ Missing ${dll} (will not be bundled)`);
    missing++;
  }
});

console.log(`\n✓ DLL check complete: ${found} found, ${missing} missing`);
if (missing > 0) {
  console.warn("\n⚠ Some DLLs are missing. The app may not work correctly.");
}

process.exit(0);
