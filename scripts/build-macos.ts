#!/usr/bin/env bun
/**
 * macOS build script that creates tauri.macos.conf.json BEFORE tauri build.
 *
 * WHY THIS EXISTS:
 * Tauri loads config (including tauri.macos.conf.json) at the start of
 * "tauri build". The beforeBundleCommand runs too late—tauri.macos.conf.json
 * is created after config load. So when you run "tauri build" directly,
 * the dylib frameworks never get merged and the app crashes with
 * "Library not loaded: @rpath/libggml-base.0.dylib".
 *
 * This script mirrors the CI flow: build binary → run dylibbundler →
 * tauri build. The config exists when tauri build starts.
 *
 * USAGE: bun scripts/build-macos.ts
 * Or: bun run build:macos
 */
import { platform } from "os";
import { spawnSync } from "child_process";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

if (platform() !== "darwin") {
  console.error("build-macos: This script is for macOS only.");
  process.exit(1);
}

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

const rawArgs = process.argv.slice(2);
let targetTriple = "";
const targetIdx = rawArgs.indexOf("--target");
if (targetIdx !== -1 && rawArgs[targetIdx + 1]) {
  targetTriple = rawArgs[targetIdx + 1];
}

const cargoArgs = ["build", "--release"];
const tauriArgs = ["run", "tauri", "build"];
if (targetTriple) {
  cargoArgs.push("--target", targetTriple);
  tauriArgs.push("--target", targetTriple);
}

const buildEnv = {
  ...process.env,
  CI: "true",
  ...(targetTriple ? { TAURI_BUILD_TARGET: targetTriple } : {}),
};

function run(cmd: string, args: string[], opts?: { cwd?: string; env?: NodeJS.ProcessEnv }) {
  const cwd = opts?.cwd ?? root;
  const r = spawnSync(cmd, args, { stdio: "inherit", cwd, env: opts?.env });
  if (r.status !== 0) process.exit(r.status ?? 1);
}

console.log("build-macos: Building frontend...");
run("bun", ["run", "build"]);

console.log(`build-macos: Building Rust binary${targetTriple ? ` for ${targetTriple}` : ""}...`);
run("cargo", cargoArgs, { cwd: join(root, "src-tauri") });

console.log("build-macos: Bundling dylibs (dylibbundler)...");
run("bun", ["scripts/bundle-macos-dylibs.ts"], { cwd: root, env: buildEnv });

console.log(`build-macos: Creating app bundle${targetTriple ? ` for ${targetTriple}` : ""}...`);
// CI=true makes create-dmg skip the Finder AppleScript window-styling step
// (--skip-jenkins), which fails in non-interactive or restricted macOS sessions.
run("bun", tauriArgs, { cwd: root, env: buildEnv });

