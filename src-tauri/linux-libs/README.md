# This directory is populated by scripts/bundle-linux-solibs.sh before `tauri build`.
# It contains libllama.so, libggml*.so, and libonnxruntime.so copied from the
# Cargo build output. These are declared as resources in tauri.linux.conf.json
# and bundled into usr/share/taurscribe/ in the final .deb / .rpm / AppImage.
#
# The binary's RPATH is patched by patchelf to $ORIGIN/../share/taurscribe
# so the dynamic linker finds these libs at runtime without LD_LIBRARY_PATH.
