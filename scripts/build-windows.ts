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
  mkdtempSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  unlinkSync,
  writeFileSync,
} from "fs";
import { join, resolve } from "path";
import { createHash } from "crypto";
import { spawnSync } from "child_process";
import { ggmlStackEnv } from "./ggml-stack";

const root = join(import.meta.dir, "..");
const srcTauri = join(root, "src-tauri");

// Parse CLI args to get target triple (e.g. --target x86_64-pc-windows-msvc)
const args = process.argv.slice(2);
const targetIdx = args.indexOf("--target");
const targetTriple = targetIdx !== -1 && args[targetIdx + 1] ? args[targetIdx + 1] : null;
const featuresIdx = args.indexOf("--features");
const features = featuresIdx !== -1 ? args[featuresIdx + 1] : null;
const onlineCuda = args.includes("--online-cuda");
const bundleOnly = args.includes("--bundle-only");
const redistIdx = args.indexOf("--cuda-redist");
const redistPath = resolve(root, redistIdx !== -1
  ? args[redistIdx + 1] ?? ""
  : "src-tauri/windows/cuda-redist-13.3.json");

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
    // llama.dll, llama-common.dll (split out of llama.dll in llama.cpp b8xxx+), ggml*.dll
    /^llama.*\.dll$|^ggml.*\.dll$/i.test(name)
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
  /^llama.*\.dll$/,
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
  // CUDA 13 moved the runtime DLLs from bin\ to bin\x64\ (CUDA 12: bin\).
  const wanted = [/^cudart64_\d+\.dll$/i, /^cublas64_\d+\.dll$/i, /^cublasLt64_\d+\.dll$/i];
  const binDir = [join(cudaPath, "bin", "x64"), join(cudaPath, "bin")].find(
    (dir) => existsSync(dir) && readdirSync(dir).some((f) => wanted[0].test(f))
  ) ?? join(cudaPath, "bin");
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
let launcherBuildDir: string | null = null;
let originalBinaryBackup: string | null = null;

// Save original config before any changes
const originalWindowsConf = readFileSync(windowsConfPath, "utf8");
const runtimeConfigPath = join(srcTauri, "windows", "runtime-config.nsh");
const originalRuntimeConfig = readFileSync(runtimeConfigPath, "utf8");

function cleanup() {
  if (originalBinaryBackup) {
    try {
      const binary = join(releaseDir, "taurscribe.exe");
      if (existsSync(binary)) unlinkSync(binary);
      renameSync(originalBinaryBackup, binary);
      originalBinaryBackup = null;
    } catch (error) {
      // Keep the backup if restoration fails; never delete the original app.
      console.error(`Restore the original executable from ${originalBinaryBackup}: ${error}`);
    }
  }
  if (launcherBuildDir && !originalBinaryBackup) {
    try { rmSync(launcherBuildDir, { recursive: true, force: true }); } catch {}
  }
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
    writeFileSync(runtimeConfigPath, originalRuntimeConfig);
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
process.on("uncaughtException", (error) => failAndExit(error.message));

type CudaRedist = {
  schemaVersion: number;
  url: string;
  archiveSha256: string;
  files: { name: string; bytes: number; sha256: string }[];
};
let cudaRedist: CudaRedist | null = null;
const nsisString = (value: string) => value.replace(/\$/g, () => "$$").replace(/"/g, '$\\"');
if (onlineCuda) {
  const bundlesIdx = args.indexOf("--bundles");
  if (bundlesIdx !== -1 && args[bundlesIdx + 1] !== "nsis") {
    failAndExit("--online-cuda supports only --bundles nsis");
  }
  if (!features || !/\b(gpu-nvidia|windows-nvidia)\b/.test(features)) {
    failAndExit("--online-cuda requires the NVIDIA flavor");
  }
  if (!existsSync(redistPath)) failAndExit(`CUDA redistributable manifest not found: ${redistPath}`);
  cudaRedist = JSON.parse(readFileSync(redistPath, "utf8"));
  const names = cudaRedist!.files?.map((file) => file.name) ?? [];
  if (cudaRedist!.schemaVersion !== 1 ||
      !/^https:\/\/developer\.download\.nvidia\.com\//.test(cudaRedist!.url) ||
      !/^[a-f\d]{64}$/i.test(cudaRedist!.archiveSha256) || names.length !== 2 ||
      new Set(names).size !== 2 ||
      !names.some((name) => /^cublas64_\d+\.dll$/.test(name)) ||
      !names.some((name) => /^cublasLt64_\d+\.dll$/.test(name)) ||
      cudaRedist!.files.some((file) => !Number.isSafeInteger(file.bytes) || file.bytes <= 0 || !/^[a-f\d]{64}$/i.test(file.sha256))) {
    failAndExit("Invalid CUDA redistributable manifest");
  }
  const deletes = names.map((name) => `  Delete "$INSTDIR\\${nsisString(name)}"`).join("\n");
  const runtimeKiB = Math.ceil(cudaRedist!.files.reduce((sum, file) => sum + file.bytes, 0) / 1024);
  writeFileSync(runtimeConfigPath,
    `!define TAURSCRIBE_ONLINE_CUDA\n!define TAURSCRIBE_CUDA_RUNTIME_KIB ${runtimeKiB}\n` +
    `!define TAURSCRIBE_CUDA_MANIFEST "${nsisString(redistPath)}"\n` +
    `!macro TAURSCRIBE_DELETE_CUDA_DLLS\n${deletes}\n!macroend\n`);
} else {
  writeFileSync(runtimeConfigPath, "; Offline installer: CUDA DLLs are bundled.\n");
}

// Bootstrap: first cargo build must not depend on preexisting DLL resource files.
const bootstrapWindowsConf = JSON.parse(originalWindowsConf);
bootstrapWindowsConf.bundle = bootstrapWindowsConf.bundle || {};
bootstrapWindowsConf.bundle.resources = [];
writeFileSync(windowsConfPath, JSON.stringify(bootstrapWindowsConf, null, 2) + "\n");

// NVIDIA flavor: ggml-cuda links nvcuda.dll, which only the NVIDIA driver installs.
// Delay-load it so the app still starts (and runs on the CPU) on a PC without the
// driver. Set on process.env so `tauri build` sees the same env and doesn't
// rebuild llama.cpp without it. build.rs does the same for the exe itself.
if (features && /\b(gpu-nvidia|windows-nvidia)\b/.test(features)) {
  process.env.CMAKE_SHARED_LINKER_FLAGS = "/DELAYLOAD:nvcuda.dll delayimp.lib";
}

// One ggml for whisper.cpp and transcribe.cpp (scripts/ggml-stack.ts): two
// static copies of different versions otherwise end up mixed in the exe.
// x64 only: the ARM64 builds are cross-compiled with their own toolchain setup.
if (!bundleOnly && (!targetTriple || targetTriple.startsWith("x86_64"))) {
  Object.assign(process.env, ggmlStackEnv(features, targetTriple));
}

// ── Step 1: Build Rust binary ────────────────────────────────
console.log(bundleOnly ? "\n📦 Step 1: Using the prebuilt Rust binary...\n" : "\n🔨 Step 1: Building Rust binary...\n");
const cargoArgs = ["build", "--release"];
if (targetTriple) {
  cargoArgs.push("--target", targetTriple);
  console.log(`Building for target: ${targetTriple}`);
}
if (features) {
  cargoArgs.push("--features", features);
}
if (bundleOnly) {
  const binary = join(releaseDir, "taurscribe.exe");
  if (!existsSync(binary) || statSync(binary).size === 0) {
    failAndExit(`--bundle-only requires an already built executable: ${binary}`);
  }
} else {
  const cargoBuild = spawnSync("cargo", cargoArgs, {
    stdio: "inherit",
    cwd: srcTauri,
    shell: true,
  });
  if (cargoBuild.status !== 0) {
    failAndExit("❌ Cargo build failed");
  }
}

// ── Step 2: Detect DLLs ──────────────────────────────────────
console.log("\n📦 Step 2: Detecting DLLs...\n");
if (!existsSync(releaseDir)) {
  failAndExit(`❌ Release directory not found: ${releaseDir}`);
}
stageLlamaDlls();

stageGpuRuntimeDlls();

// An online installer must fetch byte-identical libraries to those this app
// linked against. A toolkit upgrade cannot silently use an older manifest.
if (cudaRedist) {
  for (const file of cudaRedist.files) {
    const path = join(releaseDir, file.name);
    if (!existsSync(path) || statSync(path).size !== file.bytes ||
        createHash("sha256").update(readFileSync(path)).digest("hex") !== file.sha256.toLowerCase()) {
      failAndExit(`CUDA manifest does not match ${file.name}. Supply a matching --cuda-redist manifest or build offline.`);
    }
  }
}

const allFiles = readdirSync(releaseDir);
const foundDlls = allFiles.filter((file: string) =>
  dllPatterns.some((pattern) => pattern.test(file)) &&
  !(cudaRedist && /^cublas(Lt)?64_.*\.dll$/i.test(file))
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
console.log(`\n🚀 Step 5: Running tauri ${bundleOnly ? "bundle" : "build"}...\n`);

// Get CLI args passed to this script (e.g. --bundles nsis)
const extraArgs = args.filter((arg, index) =>
  arg !== "--online-cuda" && arg !== "--bundle-only" &&
  index !== redistIdx && (redistIdx === -1 || index !== redistIdx + 1)
);
if (onlineCuda && !extraArgs.includes("--bundles")) extraArgs.push("--bundles", "nsis");
// shell:false — on Windows, shell:true mangles --config JSON (quotes stripped → invalid JSON).
// Finish embedding the current frontend before substituting the public entry
// point for packaging. A subsequent Cargo build must never overwrite it.
let tauriCommand = bundleOnly ? "bundle" : "build";
if (onlineCuda) {
  if (!bundleOnly) {
    const compile = spawnSync("bunx", ["tauri", "build", "--no-bundle", ...extraArgs], {
      stdio: "inherit", cwd: root, shell: false,
    });
    if (compile.status !== 0) failAndExit("Tauri app compilation failed");
  }

  if (process.platform !== "win32") failAndExit("The Windows repair launcher requires a native Windows build");
  launcherBuildDir = mkdtempSync(join(releaseDir, "repair-launcher-"));
  const coreBinary = join(srcTauri, "taurscribe-core.exe");
  if (existsSync(coreBinary)) failAndExit(`Refusing to overwrite ${coreBinary}`);
  copyFileSync(join(releaseDir, "taurscribe.exe"), coreBinary);
  copiedDlls.push("taurscribe-core.exe");
  // Bundler now stamps the launcher. Preserve the same NSIS format metadata
  // in the actual Tauri app that it previously stamped in the main executable.
  const coreBytes = readFileSync(coreBinary);
  const unmarkedBundle = Buffer.from("__TAURI_BUNDLE_TYPE_VAR_UNK");
  const bundleOffset = coreBytes.indexOf(unmarkedBundle);
  if (bundleOffset !== -1) {
    if (coreBytes.indexOf(unmarkedBundle, bundleOffset + 1) !== -1) {
      failAndExit("Unexpected duplicate Tauri bundle-format markers in the app");
    }
    Buffer.from("__TAURI_BUNDLE_TYPE_VAR_NSS").copy(coreBytes, bundleOffset);
    writeFileSync(coreBinary, coreBytes);
  }
  copyFileSync(join(srcTauri, "windows", "launcher.c"), join(launcherBuildDir, "launcher.c"));
  copyFileSync(join(srcTauri, "icons", "icon.ico"), join(launcherBuildDir, "icon.ico"));
  writeFileSync(join(launcherBuildDir, "launcher.rc"), '1 ICON "icon.ico"\n');
  const runtimeFiles = [...foundDlls, ...(cudaRedist?.files.map((file) => file.name) ?? [])];
  const requiredFiles = [{ name: "taurscribe-core.exe", bytes: statSync(coreBinary).size },
    ...runtimeFiles.map((name) => ({ name, bytes: statSync(join(releaseDir, name)).size }))];
  writeFileSync(join(launcherBuildDir, "launcher-files.h"),
    "static const struct { const wchar_t *name; ULONGLONG bytes; } required_files[] = {\n" +
    requiredFiles.map((file) => `    { L${JSON.stringify(file.name)}, ${file.bytes}ULL },`).join("\n") + "\n};\n");
  const launcherCompile = spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
    "-File", join(srcTauri, "windows", "build-launcher.ps1"), "-BuildDirectory", launcherBuildDir,
    "-Architecture", targetTriple?.startsWith("aarch64") ? "arm64" : "x64"], { stdio: "inherit", shell: false });
  if (launcherCompile.status !== 0) failAndExit("Repair launcher compilation failed");
  windowsConf.bundle.resources.push("taurscribe-core.exe");
  writeFileSync(windowsConfPath, JSON.stringify(windowsConf, null, 2) + "\n");
  // Tauri signs and bundles the launcher as the main executable. Keep the exact
  // original executable as a resource and restore the Cargo output after bundling.
  const backupPath = join(launcherBuildDir, "original-taurscribe.exe");
  renameSync(join(releaseDir, "taurscribe.exe"), backupPath);
  originalBinaryBackup = backupPath;
  copyFileSync(join(launcherBuildDir, "taurscribe.exe"), join(releaseDir, "taurscribe.exe"));
  writeFileSync(runtimeConfigPath, readFileSync(runtimeConfigPath, "utf8") + "\n!define TAURSCRIBE_REPAIR_LAUNCHER\n");
  tauriCommand = "bundle";
}
const tauriBuild = spawnSync("bunx", ["tauri", tauriCommand, ...extraArgs], {
  stdio: "inherit",
  cwd: root,
  shell: false,
});

// ── Step 6: Clean up ─────────────────────────────────────────
console.log("\n🧹 Step 6: Cleaning up...\n");
cleanup();
if (originalBinaryBackup) process.exit(1);
console.log("Removed copied DLLs and restored config");

if (tauriBuild.status !== 0) {
  console.error("❌ Tauri build failed");
  process.exit(1);
}

console.log("\n✅ Windows build complete!\n");
