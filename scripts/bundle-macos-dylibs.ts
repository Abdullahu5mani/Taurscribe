#!/usr/bin/env bun
/**
 * Cross-platform wrapper for platform-specific shared library bundling.
 *
 * WHY A WRAPPER:
 * beforeBundleCommand runs on every platform (Windows, Linux, macOS). Shell
 * scripts are platform-specific; this Bun script dispatches to the right one.
 * Windows builds are a no-op here (MSVC links statically or ships DLLs via
 * the installer; there is no equivalent patchelf/dylibbundler on Windows).
 *
 * macOS: dylibbundler finds libllama/libggml dylibs, copies to macos-dylibs/,
 *        rewrites @rpath → @executable_path/../Frameworks in the binary.
 * Linux: patchelf finds libllama/libggml/libonnxruntime .so files, copies to
 *        linux-libs/, rewrites RPATH → $ORIGIN/../share/taurscribe in the binary.
 */
import { platform } from "os";
import { spawnSync } from "child_process";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

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
} else {
  // Windows: DLLs are handled by the NSIS/WiX installer or linked statically.
  process.exit(0);
}
