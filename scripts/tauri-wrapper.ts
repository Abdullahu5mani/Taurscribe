#!/usr/bin/env bun
/**
 * Cross-platform wrapper for the Tauri CLI.
 * On macOS, sets CARGO_TARGET_DIR to a cache dir on the internal drive to avoid
 * ._* resource fork files when the project lives on an external volume.
 * On Windows/Linux, passes through to tauri with no env change.
 */
import { platform } from "os";
import { spawnSync } from "child_process";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const tauriBin = join(root, "node_modules", ".bin", "tauri");

const env = { ...process.env };
if (platform() === "darwin" && !env.CARGO_TARGET_DIR) {
  const home = env.HOME || env.USERPROFILE || "";
  if (home) env.CARGO_TARGET_DIR = join(home, ".cache", "taurscribe-target");
}

const result = spawnSync(tauriBin, process.argv.slice(2), {
  stdio: "inherit",
  cwd: root,
  env,
});
process.exit(result.status ?? 0);
