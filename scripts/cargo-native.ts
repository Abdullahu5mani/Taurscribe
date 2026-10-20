#!/usr/bin/env bun
// Keep development, native tests and checks on the shipped app's ggml stack.
import { spawnSync } from "child_process";
import { join } from "path";
import { ggmlStackEnv } from "./ggml-stack";

const root = join(import.meta.dir, "..");
const args = process.argv.slice(2);
const tauri = args[0] === "--tauri";
if (tauri) args.shift();
const option = (name: string) => {
  const separator = args.indexOf("--");
  const options = separator < 0 ? args : args.slice(0, separator);
  const index = options.indexOf(name);
  return index >= 0 ? options[index + 1] ?? null
    : options.find(arg => arg.startsWith(`${name}=`))?.slice(name.length + 1) ?? null;
};
const target = option("--target");
// Preserve the Windows ARM cross-toolchain path used by build-windows.ts.
const stack = process.platform === "win32" && target?.startsWith("aarch64")
  ? {} : ggmlStackEnv(option("--features"), target);
const env = { ...process.env, ...stack };
const result = spawnSync(tauri ? process.execPath : "cargo",
  tauri ? [join(root, "node_modules", "@tauri-apps", "cli", "tauri.js"), ...args] : args, {
  cwd: tauri ? root : join(root, "src-tauri"), env, stdio: "inherit",
  shell: !tauri && process.platform === "win32",
});
process.exit(result.status ?? 1);
