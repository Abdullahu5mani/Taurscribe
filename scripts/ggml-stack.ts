#!/usr/bin/env bun
/**
 * One ggml for whisper.cpp and transcribe.cpp (Windows and Linux builds).
 *
 * whisper-rs and transcribe-cpp each compile their own static ggml, of
 * different versions (whisper.cpp 1.8.2 ships ggml 0.9.4, transcribe.cpp
 * 0.20.2). Both define the same symbols; the Windows and Linux links accept
 * duplicates (/FORCE:MULTIPLE, --allow-multiple-definition, needed for
 * llama.cpp's DLLs), and then keep one definition per symbol — so the app can
 * run a mix of two ggml versions. One such mix crashed in a CUDA kernel
 * ("CUDA error: invalid argument" in softmax.cu on the first Granite run).
 *
 * This script builds transcribe.cpp — the revision pinned in Cargo.lock, with
 * the GPU backends of the chosen features — into an install prefix, and sets:
 *   TRANSCRIBE_DIR           transcribe-cpp-sys links that prefix instead of building
 *   WHISPER_USE_SYSTEM_GGML  whisper-rs builds whisper.cpp against the prefix's ggml
 *   CMAKE_PREFIX_PATH        where whisper.cpp's find_package(ggml) finds it
 *   CARGO_TARGET_<TRIPLE>_RUSTFLAGS  "-L native=<prefix>/lib": whisper-rs-sys bundles
 *                            the static ggml into its rlib when it compiles, so it
 *                            must find the libraries itself (Cargo merges this with
 *                            .cargo/config.toml's rustflags)
 * so one ggml serves both. (llama.cpp keeps its own ggml inside its DLLs.)
 *
 *   bun scripts/ggml-stack.ts --features gpu-nvidia,gpu-vulkan [--target <triple>]
 *       prints KEY=VALUE lines;  --github-env appends them to $GITHUB_ENV.
 * scripts/build-windows.ts calls ggmlStackEnv() itself. The prefix is cached
 * under target/ggml-stack/<key> and rebuilt only when its inputs change.
 *
 * macOS builds are left as they are: their link has not shown the problem, and
 * whisper-rs there also links ggml's BLAS backend, which transcribe.cpp turns off.
 */

import { appendFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "fs";
import { dirname, join } from "path";
import { spawnSync } from "child_process";
import { createHash } from "crypto";

const root = join(import.meta.dir, "..");
const srcTauri = join(root, "src-tauri");

/** Stand-in for a ggml constant whisper.cpp 1.8.2 still uses. ggml no longer
 * needs the attention mask padded; upstream whisper.cpp dropped the padding,
 * and a pad of 1 is the same as none. */
const GGML_COMPAT_SHIM = `
// Taurscribe (scripts/ggml-stack.ts): whisper.cpp 1.8.2 pads its attention mask
// to GGML_KQ_MASK_PAD, which this ggml removed (it no longer needs padding, and
// upstream whisper.cpp dropped it). A pad of 1 is no padding.
#ifndef GGML_KQ_MASK_PAD
#define GGML_KQ_MASK_PAD 1
#endif
`;

function run(cmd: string, args: string[], cwd?: string) {
  console.error(`[ggml-stack] ${cmd} ${args.join(" ")}`);
  const r = spawnSync(cmd, args, { cwd, stdio: ["ignore", "inherit", "inherit"], shell: process.platform === "win32" });
  if (r.status !== 0) throw new Error(`${cmd} failed (exit ${r.status})`);
}

/** transcribe.cpp's source tree: the git checkout Cargo uses for transcribe-cpp-sys. */
function transcribeSource(target: string | null): { dir: string; rev: string } {
  const args = ["metadata", "--format-version", "1", "--manifest-path", join(srcTauri, "Cargo.toml")];
  if (target) args.push("--filter-platform", target);
  const r = spawnSync("cargo", args, { encoding: "utf8", maxBuffer: 256 * 1024 * 1024, shell: process.platform === "win32" });
  if (r.status !== 0) throw new Error(`cargo metadata failed: ${r.stderr}`);
  const pkg = JSON.parse(r.stdout).packages.find((p: { name: string }) => p.name === "transcribe-cpp-sys");
  if (!pkg) throw new Error("transcribe-cpp-sys is not a dependency");
  // The checkout root (CMakeLists.txt and ggml/) above the crate's Cargo.toml.
  let dir = dirname(pkg.manifest_path);
  while (!(existsSync(join(dir, "CMakeLists.txt")) && existsSync(join(dir, "ggml")))) {
    const up = dirname(dir);
    if (up === dir) throw new Error(`transcribe.cpp sources not found above ${pkg.manifest_path}`);
    dir = up;
  }
  const rev = /rev=([0-9a-f]+)/.exec(pkg.source ?? "")?.[1] ?? /#([0-9a-f]+)$/.exec(pkg.source ?? "")?.[1] ?? "unknown";
  return { dir, rev };
}

/** CMake options for the features, as transcribe-cpp-sys's build.rs sets them. */
function cmakeDefines(features: string[]): string[] {
  const d = [
    "-DCMAKE_BUILD_TYPE=Release",
    "-DTRANSCRIBE_INSTALL=ON",
    "-DTRANSCRIBE_BUILD_TESTS=OFF",
    "-DTRANSCRIBE_BUILD_EXAMPLES=OFF",
    "-DTRANSCRIBE_BUILD_TOOLS=OFF",
    "-DTRANSCRIBE_BUILD_SHARED=OFF",
    "-DTRANSCRIBE_USE_OPENMP=OFF",
  ];
  if (features.includes("gpu-nvidia") || features.includes("windows-nvidia")) d.push("-DTRANSCRIBE_CUDA=ON");
  if (features.includes("gpu-vulkan")) d.push("-DTRANSCRIBE_VULKAN=ON");
  if (features.includes("gpu-amd")) d.push("-DTRANSCRIBE_HIP=ON");
  // Same escape hatch transcribe-cpp-sys honours (CI passes -DGGML_NATIVE=OFF).
  for (const extra of [process.env.TRANSCRIBE_CMAKE_ARGS, process.env.CMAKE_ARGS]) {
    if (extra) d.push(...extra.split(/\s+/).filter(Boolean));
  }
  return d;
}

export type GgmlStackEnv = Record<string, string>;

/** The triple cargo builds for: --target, else the host's. */
function buildTriple(target: string | null): string {
  if (target) return target;
  const r = spawnSync("rustc", ["-vV"], { encoding: "utf8", shell: process.platform === "win32" });
  const host = /^host: (\S+)/m.exec(r.stdout ?? "")?.[1];
  if (!host) throw new Error("could not read the host triple from rustc -vV");
  return host;
}

/** Builds (or reuses) the shared prefix and returns the environment to build with. */
export function ggmlStackEnv(featureList: string | null, target: string | null): GgmlStackEnv {
  if (process.platform === "darwin") throw new Error("ggml-stack is for Windows and Linux builds");
  const features = (featureList ?? "").split(",").map((f) => f.trim()).filter(Boolean).sort();
  const { dir: source, rev } = transcribeSource(target);
  const defines = cmakeDefines(features);
  const key = createHash("sha256")
    .update(JSON.stringify({ rev, defines, target, cuda: process.env.CMAKE_CUDA_ARCHITECTURES ?? "", gen: process.env.CMAKE_GENERATOR ?? "" }))
    .digest("hex")
    .slice(0, 16);
  const targetDir = process.env.CARGO_TARGET_DIR || join(srcTauri, "target");
  const prefix = join(targetDir, "ggml-stack", key);
  const stamp = join(prefix, ".built");
  // Windows: build in a short directory; the Vulkan shader generator's nested
  // CMake project overflows MAX_PATH under a deep target dir.
  const build = process.platform === "win32" && process.env.LOCALAPPDATA
    ? join(process.env.LOCALAPPDATA, "tgs", key)
    : join(prefix, "build");

  if (!existsSync(stamp)) {
    console.error(`[ggml-stack] building transcribe.cpp ${rev.slice(0, 8)} (${features.join(",") || "cpu"}) into ${prefix}`);
    mkdirSync(build, { recursive: true });
    run("cmake", ["-S", source, "-B", build, ...defines]);
    run("cmake", ["--build", build, "--config", "Release", "--parallel"]);
    run("cmake", ["--install", build, "--config", "Release", "--prefix", prefix]);
    const header = join(prefix, "include", "ggml.h");
    if (!readFileSync(header, "utf8").includes("GGML_KQ_MASK_PAD")) appendFileSync(header, GGML_COMPAT_SHIM);
    writeFileSync(stamp, JSON.stringify({ rev, features, defines, target }, null, 2));
  } else {
    console.error(`[ggml-stack] reusing ${prefix}`);
  }

  // CMake takes ';'-separated lists on every platform; keep what is already
  // there (CI puts SPIRV-Headers on it for transcribe.cpp's Vulkan build).
  const existing = process.env.CMAKE_PREFIX_PATH;
  const flagsVar = `CARGO_TARGET_${buildTriple(target).toUpperCase().replace(/-/g, "_")}_RUSTFLAGS`;
  const libFlag = `-L native=${join(prefix, "lib")}`;
  const flags = process.env[flagsVar];
  return {
    TRANSCRIBE_DIR: prefix,
    WHISPER_USE_SYSTEM_GGML: "ON",
    // Keep what is already there; running twice must not drop other entries.
    CMAKE_PREFIX_PATH: !existing ? prefix : existing.split(";").includes(prefix) ? existing : `${prefix};${existing}`,
    [flagsVar]: flags && !flags.includes(libFlag) ? `${flags} ${libFlag}` : flags ?? libFlag,
  };
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const opt = (name: string) => {
    const i = args.indexOf(name);
    return i !== -1 && args[i + 1] ? args[i + 1] : null;
  };
  const env = ggmlStackEnv(opt("--features"), opt("--target"));
  const lines = Object.entries(env).map(([k, v]) => `${k}=${v}`);
  if (args.includes("--github-env")) {
    const file = process.env.GITHUB_ENV;
    if (!file) throw new Error("--github-env needs GITHUB_ENV");
    appendFileSync(file, lines.join("\n") + "\n");
  }
  console.log(lines.join("\n"));
}
