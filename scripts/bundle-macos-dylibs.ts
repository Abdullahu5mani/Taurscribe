#!/usr/bin/env bun
/**
 * Cross-platform wrapper for bundle-macos-dylibs.sh.
 *
 * WHY A WRAPPER:
 * beforeBundleCommand runs on every platform (Windows, Linux, macOS). The shell
 * script uses dylibbundler (macOS-only) and bash. On Windows, bash may not be
 * in PATH; this Bun script exits immediately on non-darwin, so Windows/Linux
 * builds never invoke the shell script. Bun is already required for the build.
 */
import { platform } from "os";
import { spawnSync } from "child_process";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

if (platform() !== "darwin") {
  process.exit(0);
}

const scriptDir = dirname(fileURLToPath(import.meta.url));
const scriptPath = join(scriptDir, "bundle-macos-dylibs.sh");

const result = spawnSync("bash", [scriptPath], {
  stdio: "inherit",
  cwd: join(scriptDir, ".."),
});

process.exit(result.status ?? 0);
