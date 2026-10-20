# Runs at ggml's project() boundary, after transcribe.cpp sets its defaults.
# whisper-rs always links ggml-blas on macOS. Build it from the same ggml source
# as the other backends instead of borrowing an incompatible Whisper archive.
set(GGML_BLAS ON CACHE BOOL "" FORCE)
set(GGML_BLAS_VENDOR Apple CACHE STRING "" FORCE)
