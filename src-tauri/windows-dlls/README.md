# windows-dlls/

Staging directory for Windows DLLs that must ship alongside `taurscribe.exe`.

## Why this exists

`llama-cpp-2` uses `dynamic-link`, which builds `llama.dll` and `ggml.dll` as
shared libraries at compile time. Windows' DLL loader looks for DLLs in the
same directory as the executable, but Cargo does **not** copy these DLLs to
`target/release/` automatically.

`tauri.windows.conf.json` maps `windows-dlls/*.dll → "."` so Tauri's NSIS
bundler installs them at `$INSTDIR` next to `taurscribe.exe`.

## How it gets populated

The `beforeBundleCommand` in `tauri.conf.json` runs
`bun scripts/bundle-macos-dylibs.ts`, which on Windows:

1. Searches `$CARGO_TARGET_DIR/$TAURI_BUILD_TARGET/release/build/llama-cpp-sys-2-*/out/`
   for all `*.dll` files produced by CMake.
2. Copies them here (stale DLLs from previous builds are removed first).
3. Tauri then picks them up via the glob in `tauri.windows.conf.json`.

This directory is intentionally **committed empty** (`.gitkeep`). Its contents
are generated at build time and should not be committed.
