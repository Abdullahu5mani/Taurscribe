#!/usr/bin/env node
import { existsSync, readdirSync, writeFileSync } from "fs";
import { join } from "path";

// Only run on Windows
const platform = process.platform;
if (platform !== "win32") {
  console.log("⏭ Skipping DLL bundling (not Windows)");
  process.exit(0);
}

const releaseDir = join(process.cwd(), "src-tauri", "target", "release");
const configPath = join(process.cwd(), "src-tauri", "tauri.conf.json");

// Expected DLL patterns to bundle
const dllPatterns = [
  /^llama\.dll$/,
  /^ggml.*\.dll$/,
  /^DirectML\.dll$/,
  /^onnxruntime.*\.dll$/,
];

// Scan release directory for matching DLLs
if (!existsSync(releaseDir)) {
  console.error(`❌ Release directory not found: ${releaseDir}`);
  console.error("   Run 'cargo build --release' first");
  process.exit(1);
}

const allFiles = readdirSync(releaseDir);
const foundDlls = allFiles.filter((file) =>
  dllPatterns.some((pattern) => pattern.test(file))
);

if (foundDlls.length === 0) {
  console.warn("⚠ No DLLs found to bundle");
  process.exit(0);
}

console.log(`📦 Found ${foundDlls.length} DLL(s) to bundle:`);
foundDlls.forEach((dll) => console.log(`   - ${dll}`));

// Read current tauri.conf.json
const config = JSON.parse(
  require("fs").readFileSync(configPath, "utf8")
);

// Build resources map: { "src": "dest" }
const resources = {};
foundDlls.forEach((dll) => {
  resources[`../target/release/${dll}`] = ".";
});

// Update config
config.bundle.resources = resources;

// Write back to tauri.conf.json
writeFileSync(configPath, JSON.stringify(config, null, 2) + "\n");

console.log(`✓ Updated tauri.conf.json with ${foundDlls.length} resource(s)`);
console.log("✓ DLL bundling configuration complete");
