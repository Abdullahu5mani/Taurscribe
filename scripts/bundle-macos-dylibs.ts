#!/usr/bin/env bun
/**
 * Cross-platform wrapper for platform-specific shared library bundling.
 *
 * WHY A WRAPPER:
 * beforeBundleCommand runs on every platform (Windows, Linux, macOS). Shell
 * scripts are platform-specific; this Bun script dispatches to the right one.
 *
 * macOS: dylibbundler finds libllama/libggml dylibs, copies to macos-dylibs/,
 *        rewrites @rpath → @executable_path/../Frameworks in the binary.
 * Linux: patchelf finds libllama/libggml/libonnxruntime .so files, copies to
 *        linux-libs/, rewrites RPATH → $ORIGIN/../share/taurscribe in the binary.
 * Windows: llama-cpp-2 (dynamic-link) produces llama.dll / ggml.dll that the
 *          OS loader must find next to taurscribe.exe. This script locates the
 *          DLLs in Cargo's build output, copies them to src-tauri/windows-dlls/,
 *          and tauri.windows.conf.json bundles them into the NSIS installer at
 *          the install root ($INSTDIR) so Windows can find them.
 */
import { platform } from "os";
import { spawnSync } from "child_process";
import { join, dirname, basename } from "path";
import { fileURLToPath } from "url";
import {
  readdirSync,
  copyFileSync,
  mkdirSync,
  rmSync,
  existsSync,
  statSync,
} from "fs";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const cwd = join(scriptDir, "..");

if (platform() === "darwin") {
  const result = spawnSync("bash", [join(scriptDir, "bundle-macos-dylibs.sh")], {
    stdio: "inherit",
    cwd,
  });
  process.exit(result.status ?? 0);
} else if (platform() === "linux") {
  const result = spawnSync("bash", [join(scriptDir, "bundle-linux-solibs.sh")], {
    stdio: "inherit",
    cwd,
  });
  process.exit(result.status ?? 0);
} else if (platform() === "win32") {
  // Windows DLL bundling is handled by scripts/copy-dlls.ts
  // which runs first in the beforeBundleCommand chain and updates
  // tauri.conf.json + tauri.windows.conf.json with the correct
  // resource map (DLLs → root install directory).
  process.exit(0);
} else {
  process.exit(0);
}

/** Recursively find all .dll files under a directory. */
function findDlls(dir: string, results: string[] = []): string[] {
  if (!existsSync(dir)) return results;
  try {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const fullPath = join(dir, entry.name);
      if (entry.isDirectory()) {
        findDlls(fullPath, results);
      } else if (entry.isFile() && entry.name.toLowerCase().endsWith(".dll")) {
        results.push(fullPath);
      }
    }
  } catch {
    // Ignore permission errors on deep build-cache paths
  }
  return results;
}

/**
 * Locate DLLs produced by llama-cpp-sys-2 (dynamic-link) in Cargo's build
 * output directory and copy them to src-tauri/windows-dlls/.
 *
 * Build output location:
 *   $CARGO_TARGET_DIR/$TAURI_BUILD_TARGET/release/build/llama-cpp-sys-2-<hash>/out/
 *
 * tauri.windows.conf.json maps windows-dlls/*.dll → "." so Tauri's NSIS
 * bundler places them at $INSTDIR (same directory as taurscribe.exe), which
 * is exactly where Windows' DLL loader looks first.
 */
function bundleWindowsDlls(): void {
  const srcTauri = join(cwd, "src-tauri");
  const targetDir = process.env.CARGO_TARGET_DIR ?? join(srcTauri, "target");
  const targetTriple = process.env.TAURI_BUILD_TARGET ?? "";

  // Cross-compiled builds land in target/<triple>/release/build/
  const primaryBuildBase = targetTriple
    ? join(targetDir, targetTriple, "release", "build")
    : join(targetDir, "release", "build");

  const dllsDir = join(srcTauri, "windows-dlls");

  // Clean stale DLLs from a previous build
  if (existsSync(dllsDir)) {
    for (const f of readdirSync(dllsDir)) {
      if (f.toLowerCase().endsWith(".dll")) rmSync(join(dllsDir, f));
    }
  } else {
    mkdirSync(dllsDir, { recursive: true });
  }

  const copiedNames = new Set<string>();
  let copied = 0;

  function copyFromBuildBase(buildBase: string): void {
    if (!existsSync(buildBase)) return;
    for (const entry of readdirSync(buildBase, { withFileTypes: true })) {
      if (!entry.isDirectory() || !entry.name.startsWith("llama-cpp-sys-2-")) continue;
      const outDir = join(buildBase, entry.name, "out");
      for (const dll of findDlls(outDir)) {
        const name = basename(dll);
        if (copiedNames.has(name)) continue; // skip duplicates across search paths
        copyFileSync(dll, join(dllsDir, name));
        console.log(`bundle-windows-dlls: Copied ${name} → windows-dlls/`);
        copiedNames.add(name);
        copied++;
      }
    }
  }

  copyFromBuildBase(primaryBuildBase);

  // Fallback: also try the non-triple path in case target wasn't specified
  if (targetTriple) {
    const hostBuildBase = join(targetDir, "release", "build");
    if (hostBuildBase !== primaryBuildBase) {
      copyFromBuildBase(hostBuildBase);
    }
  }

  if (copied === 0) {
    console.warn("bundle-windows-dlls: WARNING - No DLLs found in llama-cpp-sys-2 build output.");
    console.warn(`  Searched: ${primaryBuildBase}`);
    console.warn("  The grammar LLM feature will fail to load on installed systems.");
  } else {
    console.log(`bundle-windows-dlls: Bundled ${copied} DLL(s) into windows-dlls/`);
  }

  process.exit(0);
}
