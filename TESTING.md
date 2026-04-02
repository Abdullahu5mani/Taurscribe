# Testing & Accuracy Evaluation

This document explains the accuracy testing and evaluation suite in Taurscribe: what each component does, how audio pipelines relate to the live app, where test data comes from, and how to run everything.

---

## Overview

The testing suite has three purposes:

1. **Smoke test** — quickly verify that all three ASR engines load and produce non-empty output on a known audio clip (JFK speech).
2. **Memory regression test** — verify that engine load/transcribe/unload cycles and engine-switching sequences do not leak or unexpectedly retain RAM.
3. **Integration accuracy tests** — run the same library code paths as the live app against a real speech dataset and compute Word Error Rate (WER).
4. **Offline batch evaluation** — standalone CLI (`librispeech_eval`) for bulk WER benchmarking, outputting CSV for analysis.

All integration tests are marked `#[ignore]` so normal `cargo test` stays fast. Opt in with `cargo test -- --ignored`. Add `--nocapture` to print per-utterance WER lines and summaries in the terminal.

### If you do not have models or eval data yet

| What you have | What you can run |
| --- | --- |
| **Nothing extra** (fresh clone) | From `src-tauri`: `cargo test` **without** `--ignored`. That runs library unit tests only (e.g. WER math, preprocess sanity). No ASR weights or LibriSpeech required. |
| **No ASR models** | `librispeech_eval` and the ignored integration tests **need** at least one engine's weights under the [model locations](#model-locations) path, or they error / skip. To **mark ignored tests as passed without running inference** (e.g. CI): set `TAURSCRIBE_ASR_SMOKE_SKIP=1` when running `cargo test -- --ignored`. |
| **No LibriSpeech** | You cannot build a manifest from `test-clean` or run `mic_accuracy` / `file_drop_accuracy` / full `librispeech_eval` on real audio until you [download the dataset](#downloading-the-librispeech-test-clean-dataset). |
| **No `jfk.wav`** | The JFK smoke test and memory regression test fail unless you add `src-tauri/tests/fixtures/jfk.wav`, set `JFK_WAV`, or use `TAURSCRIBE_ASR_SMOKE_SKIP=1`. |

**Summary:** day-to-day development without GPUs or large downloads is still possible with plain `cargo test`. Full WER / smoke / memory workflows need models (via **Settings → Downloads** in the app) and, for LibriSpeech-based tests, the dataset plus a manifest.

---

## Key Files

| File | Type | Purpose |
| --- | --- | --- |
| `src-tauri/src/bin/librispeech_eval.rs` | CLI | Batch WER for Whisper / Parakeet / Cohere from a JSONL manifest |
| `src-tauri/src/bin/librispeech_manifest.rs` | CLI | Builds JSONL manifest (`utt_id`, `flac_path`, `ref_text`) from LibriSpeech `test-clean` |
| `src-tauri/src/librispeech_wer.rs` | Library | Text normalization, token-level Levenshtein WER, LibriSpeech FLAC path resolution helpers |
| `src-tauri/src/audio_decode.rs` | Library | Format-agnostic decode (FLAC, WAV, MP3, M4A, …) via Symphonia |
| `src-tauri/src/audio_preprocess.rs` | Library | Resample, denoise, DC remove, HP filter, level assist, clamp |
| `src-tauri/src/memory.rs` | Library | Process memory snapshots (`working_set`, `private_bytes`, `peak`) via Windows PSAPI / sysinfo; `trim_process_memory()` |
| `src-tauri/src/ort_session.rs` | Library | Low-RAM ORT session builder helpers: disabled CPU arena, `SameAsRequested` CUDA arena, heuristic cuDNN search |
| `src-tauri/tests/jfk_asr_smoke.rs` | Integration | JFK WAV → all three engines must return non-empty text |
| `src-tauri/tests/memory_engine_regression.rs` | Integration | Load/transcribe/unload cycles + cross-engine switch sequences; snapshots RAM at each step |
| `src-tauri/tests/parakeet_context_reset.rs` | Integration | Verifies `clear_context()` restores Parakeet to a fresh-session baseline (same audio → same transcript) |
| `src-tauri/tests/file_drop_accuracy.rs` | Integration | Same pipeline as file drag-and-drop (energy VAD assembly + chunking) |
| `src-tauri/tests/mic_accuracy.rs` | Integration | Same pipeline as live mic (chunking + energy VAD gate) |
| `scripts/download_librispeech_test_clean.sh` | Script | Download + verify + extract LibriSpeech test-clean (macOS / Linux) |
| `scripts/download_librispeech_test_clean.ps1` | Script | Same for Windows |
| `src-tauri/tests/fixtures/` | Directory | Place `jfk.wav` here (not committed); or set `JFK_WAV` |

---

## Integration Tests Reference

### `jfk_asr_smoke` — three-engine smoke test

Runs preprocessed JFK audio (`resample → trim → preprocess_assembled_speech_16k`) through Whisper, Parakeet, and Cohere in sequence. Asserts each returns a non-empty transcript. Any engine whose models are missing is reported as a failure rather than silently skipped.

```bash
cd src-tauri
cargo test jfk_audio_through_whisper_parakeet_and_granite -- --ignored --nocapture
```

**Requires:** `jfk.wav` + all three model bundles installed.
**Skip without failing:** `TAURSCRIBE_ASR_SMOKE_SKIP=1`

---

### `memory_engine_regression` — RAM regression across load/unload cycles

The most comprehensive memory test. Runs the following scenarios in order, taking a `ProcessMemoryStats` snapshot (working set, private bytes, peak) at each named step:

| Scenario | What it does |
| --- | --- |
| `whisper_cycle` | Initialize → transcribe ×2 → unload. Asserts unload clears the model. |
| `parakeet_cycle` | Same pattern with `FallbackGpu` load path (GPU with CPU fallback). |
| `parakeet_strict_gpu_cycle` | Same with `StrictGpu` (no CPU fallback). Skipped if CUDA/DirectML unavailable. |
| `parakeet_fallback_gpu_cycle` | Explicit `FallbackGpu` path (separate from default cycle). |
| `parakeet_cpu_cycle` | Force CPU for Parakeet regardless of GPU availability. |
| `cohere_cycle` | Initialize → transcribe ×2 → unload. Skipped if Cohere bundle missing. |
| `switch_whisper_parakeet_whisper` | W→P→W with explicit unloads between. Verifies VRAM/RAM releases at each switch. |
| `switch_whisper_cohere_whisper` | W→C→W. |
| `switch_parakeet_cohere_parakeet` | P→C→P. |

Each scenario prints peak working set and peak private bytes to stderr. The full structured report can be written to a JSON file for offline diffing.

```powershell
# PowerShell — basic run
$env:TAURSCRIBE_LOG_MEMORY = '1'
cargo test memory_engine_regression -- --ignored --nocapture
```

```powershell
# Write a JSON report for diffing across builds
$env:TAURSCRIBE_MEMORY_REPORT = 'memory_report.json'
$env:TAURSCRIBE_LOG_MEMORY    = '1'
cargo test memory_engine_regression -- --ignored --nocapture
```

```bash
# Force all engines to CPU (useful on machines without GPU)
TAURSCRIBE_MEMORY_FORCE_CPU=1 \
cargo test memory_engine_regression -- --ignored --nocapture
```

**Env vars:**

| Var | Effect |
| --- | --- |
| `TAURSCRIBE_LOG_MEMORY=1` | Enable per-step memory logging from `memory.rs` throughout the app |
| `TAURSCRIBE_MEMORY_REPORT=path.json` | Write a full JSON report (all scenarios + snapshots) to the given path |
| `TAURSCRIBE_MEMORY_FORCE_CPU=1` | Force CPU load path for all engines in the test |
| `TAURSCRIBE_ASR_SMOKE_SKIP=1` | Skip the test entirely (pass) |

**Requires:** `jfk.wav` + Whisper model + Parakeet model. Cohere scenarios are soft-skipped if the bundle is missing.

---

### `parakeet_clear_context_restores_session_baseline` — context reset regression

Verifies that calling `ParakeetManager::clear_context()` (which `stop_recording` calls at the end of every Parakeet recording) fully resets the streaming session state. The test transcribes JFK audio, calls `clear_context()`, transcribes the same audio again, and asserts both transcripts are identical. A mismatch means accumulated decoder state is bleeding between recordings.

```bash
cd src-tauri
cargo test parakeet_clear_context_restores_session_baseline -- --ignored --nocapture
```

**Requires:** `jfk.wav` + at least one Parakeet/Nemotron ONNX bundle.

---

### `file_drop_accuracy` — file drag-and-drop pipeline WER

Mirrors `commands/file_transcription.rs: transcribe_file_blocking` exactly:

```
decode → mono mix → resample 16 kHz → trim edges
  → assemble_speech_audio (adaptive energy/RMS VAD)
  → preprocess_assembled_speech_16k
  → chunked engine call → clean_transcript → WER
```

```bash
cd src-tauri
TAURSCRIBE_EVAL_MANIFEST=../taurscribe-runtime/librispeech/eval_manifest.jsonl \
TAURSCRIBE_LIBRISPEECH_AUDIO_ROOT=../taurscribe-runtime/librispeech/LibriSpeech/test-clean \
  cargo test file_drop_accuracy -- --ignored --nocapture
```

---

### `mic_accuracy` — live recording pipeline WER

Mirrors `commands/recording.rs` without cpal or Tauri. Audio files are decoded at native sample rate and fed through the same chunk-accumulation loop as a live recording:

- **Whisper / Cohere:** 6s chunks → `preprocess_live_transcribe_chunk` → energy VAD gate (0.35) → transcribe
- **Parakeet:** 4s chunks → `preprocess_live_transcribe_chunk` → pad to ≥64 000 samples → transcribe (no VAD gate)

```bash
cd src-tauri
TAURSCRIBE_EVAL_MANIFEST=../taurscribe-runtime/librispeech/eval_manifest.jsonl \
TAURSCRIBE_LIBRISPEECH_AUDIO_ROOT=../taurscribe-runtime/librispeech/LibriSpeech/test-clean \
  cargo test mic_accuracy -- --ignored --nocapture
```

---

## Online Resources & Dataset

### LibriSpeech test-clean

- **Source:** [OpenSLR SLR12](https://www.openslr.org/12/) — `https://www.openslr.org/resources/12/test-clean.tar.gz`
- **License:** CC BY 4.0 (derived from LibriVox public-domain audiobooks)
- **Size:** ~346 MB compressed
- **Content:** ~2,620 utterances of clean read English from 40 speakers; typical utterance ~5–15 seconds
- **MD5:** `32fa31d27d2e1cad72775fee3f4849a9`
- **Layout:** FLAC files + `.trans.txt` transcripts in `reader/chapter/utt_id.flac` form

### JFK smoke test audio

The JFK sample is **not** in the repository. Use either:

1. `src-tauri/tests/fixtures/jfk.wav`, or
2. **`JFK_WAV`** pointing at any path on disk.

---

## Downloading the LibriSpeech test-clean dataset

Eval and integration tests need the **test-clean** split from LibriSpeech: read English speech as FLAC files plus reference text. The repo ships scripts that download the official tarball from OpenSLR, verify integrity, and extract it. You can also download manually if you prefer.

### Prerequisites

| Platform | Requirements |
| --- | --- |
| macOS / Linux | `bash`, `curl`, `tar`, and `md5` or `md5sum` (checksum; script warns if missing) |
| Windows | PowerShell 5+, **`tar`** (included in Windows 10+), network access for `Invoke-WebRequest` |

### Recommended: use the repo scripts

Run from the **repository root** (the folder that contains `scripts/` and `src-tauri/`).

**macOS / Linux**

```bash
bash scripts/download_librispeech_test_clean.sh
```

**Windows (PowerShell)**

```powershell
.\scripts\download_librispeech_test_clean.ps1
```

### What the scripts do

1. **Download** `https://www.openslr.org/resources/12/test-clean.tar.gz` (~346 MB) into a destination folder as `test-clean.tar.gz`.
2. **Verify** the archive MD5 matches `32fa31d27d2e1cad72775fee3f4849a9`. On mismatch, delete the bad file and retry.
3. **Extract** with `tar -xzf` so you get a `LibriSpeech/test-clean/` tree with readers, chapters, `.flac`, and `.trans.txt` files.

The process is **idempotent**: if the tarball already exists, download is skipped; if `LibriSpeech/test-clean` already exists, extraction is skipped.

### Where files land (default vs custom)

By default, data goes under **`taurscribe-runtime/librispeech/`** at the repo root (that folder is gitignored). After a successful run you should have:

- `taurscribe-runtime/librispeech/test-clean.tar.gz` — cached archive
- `taurscribe-runtime/librispeech/LibriSpeech/test-clean/` — extracted corpus (this is the path you pass to `librispeech_manifest --root` and to `TAURSCRIBE_LIBRISPEECH_AUDIO_ROOT` / `--audio-root`)

To install elsewhere, set **`LIBRISPEECH_ROOT`** to the **parent directory** that should contain the `LibriSpeech` folder (not the `test-clean` path itself).

**macOS / Linux**

```bash
export LIBRISPEECH_ROOT="/Volumes/ExternalData/speech-data"
bash scripts/download_librispeech_test_clean.sh
# → /Volumes/ExternalData/speech-data/LibriSpeech/test-clean/
```

**Windows (PowerShell)**

```powershell
$env:LIBRISPEECH_ROOT = "D:\speech-data"
.\scripts\download_librispeech_test_clean.ps1
# → D:\speech-data\LibriSpeech\test-clean\
```

When the script finishes, it prints a sample **`librispeech_manifest`** command you can run to build `eval_manifest.jsonl` next to the archive.

### Manual download (optional)

If you do not use the scripts:

1. Download [test-clean.tar.gz](https://www.openslr.org/resources/12/test-clean.tar.gz) from [OpenSLR 12](https://www.openslr.org/12/).
2. Confirm MD5 `32fa31d27d2e1cad72775fee3f4849a9` (see [md5sum.txt](https://www.openslr.org/resources/12/md5sum.txt)).
3. Extract: `tar -xzf test-clean.tar.gz` in a directory of your choice.
4. Use the resulting **`.../LibriSpeech/test-clean`** path as `--root` for `librispeech_manifest` and as the audio root for eval/tests when needed.

---

## Audio Pipelines — Do They Mirror the Real App?

**Yes.** Tests and eval binaries call the same library functions as production code. Shared entry points:

- `audio_decode::decode_audio_interleaved_f32` — file loading for `file_transcription`, tests, and `librispeech_eval`
- `audio_preprocess::preprocess_assembled_speech_16k` — post–speech-segment preprocessing for file-drop path and `file_drop_accuracy`
- `audio_preprocess::preprocess_live_transcribe_chunk` — live streaming preprocessing for `recording` and `mic_accuracy`

### Pipeline comparison

| Test / tool | App path it mirrors | VAD | Chunking |
| --- | --- | --- | --- |
| `librispeech_eval` | *(standalone; no UI)* | No — utterances are short clips | Whisper: 3 min; Parakeet / Cohere: 15 s |
| `jfk_asr_smoke` | Sanity check only | No | Full clip |
| `memory_engine_regression` | Engine manager lifecycle | No | Full JFK clip per scenario |
| `parakeet_context_reset` | `stop_recording` → `clear_context()` | No | Full JFK clip |
| `file_drop_accuracy` | `commands/file_transcription.rs` | Yes — **adaptive energy (RMS)** segment assembly | Same as eval binary for engines |
| `mic_accuracy` | `commands/recording.rs` | Yes — energy gate on 6 s windows | Parakeet: 4 s chunks, no gate, padded to ≥64k samples |

### File drop path (step by step)

`file_drop_accuracy` and `transcribe_file_blocking` share:

```
decode → mono → resample 16 kHz → trim edge silence
  → assemble_speech_audio (adaptive RMS / energy VAD: keep speech segments only)
  → preprocess_assembled_speech_16k
  → engine-specific chunking → ASR → clean_transcript
```

### Live mic path (step by step)

```
cpal capture → preprocess_live_transcribe_chunk → 6 s rolling chunks
  → energy VAD gate (~0.25) → Whisper / Cohere

Parakeet: 4 s chunks, no VAD gate, padded to ≥64k samples
```

### Eval binary path (`librispeech_eval`)

No VAD assembly — LibriSpeech utterances are treated as single clips:

```
decode → mono 16 kHz → trim_file_buffer_edges_16k → preprocess_assembled_speech_16k
  → chunk → ASR → clean_transcript → WER
```

---

## Memory Infrastructure

### `memory.rs`

Provides `ProcessMemoryStats` (working set, private bytes, virtual bytes, peak working set) and two collection backends:

- **Windows:** `GetProcessMemoryInfo` via PSAPI — accurate private bytes and peak working set
- **Fallback:** `sysinfo` crate — working set + virtual memory only

Key functions:

| Function | Purpose |
| --- | --- |
| `process_memory_stats()` | Snapshot current process memory |
| `log_process_memory(label)` | Print a formatted memory line to stdout |
| `maybe_log_process_memory(label)` | Same but only when `TAURSCRIBE_LOG_MEMORY=1` |
| `trim_process_memory()` | Ask the OS to reclaim idle pages: `EmptyWorkingSet` (Windows), `malloc_trim(0)` (Linux) |

### `ort_session.rs`

Centralises low-RAM ORT session configuration so all three engines use the same settings:

| Helper | What it configures |
| --- | --- |
| `initialize_low_ram_ort_environment()` | Shared global ORT thread pool (avoids per-session pool overhead); thread counts via `TAURSCRIBE_ORT_INTRA_THREADS` / `TAURSCRIBE_ORT_INTER_THREADS` |
| `build_low_ram_cuda_execution_provider()` | `SameAsRequested` arena growth, heuristic cuDNN search, 32 MB conv workspace cap, optional `TAURSCRIBE_ORT_CUDA_MEM_LIMIT_MB` |
| `configure_low_ram_session_builder(builder, label)` | Disables CPU mem arena, memory pattern, prepacking, inter/intra-op thread spinning |

These are applied to every ORT session created by Cohere (`cohere.rs`) and Parakeet (`vendor/parakeet-rs/src/execution.rs`).

---

## Manifest paths and moving the corpus

`librispeech_manifest` writes **absolute** `flac_path` strings. If you move LibriSpeech, copy the manifest to another machine, or run from a different checkout, those paths may break.

**Resolution:** If the stored path is missing, tools rebuild
`test-clean/<reader>/<chapter>/<utt_id>.flac` from `utt_id` when you set the **`test-clean`** root:

| Mechanism | Where |
| --- | --- |
| Env `TAURSCRIBE_LIBRISPEECH_AUDIO_ROOT` | `librispeech_eval`, `mic_accuracy`, `file_drop_accuracy` |
| Flag `--audio-root <path>` | `librispeech_eval` only (overrides env if both set) |

Point at the directory that **contains** speaker folders (e.g. `908/`), not the parent `LibriSpeech/` folder.

---

## WER (`librispeech_wer.rs`)

WER counts word-level insertions, substitutions, and deletions vs. the reference.

**Normalization** (reference and hypothesis):

1. Lowercase
2. Non-alphanumeric → space, except apostrophes kept
3. Collapse whitespace → word tokens

**Formula:** `Levenshtein(ref_tokens, hyp_tokens) / max(len(ref_tokens), 1)`

The eval binary applies `clean_transcript()` to raw ASR output before normalization.

---

## Running everything

Assume repository root unless noted. Use `--manifest-path src-tauri/Cargo.toml` when invoking Cargo from the repo root.

### 1. Download the dataset

Follow [Downloading the LibriSpeech test-clean dataset](#downloading-the-librispeech-test-clean-dataset) above.

### 2. Build the JSONL manifest

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin librispeech_manifest -- \
  --root taurscribe-runtime/librispeech/LibriSpeech/test-clean \
  --out taurscribe-runtime/librispeech/eval_manifest.jsonl
```

Useful: `--limit N` and `--shuffle-seed U64` for a smaller, reproducible subset.

### 3. Run `librispeech_eval`

From repo root:

```bash
cargo run --release --manifest-path src-tauri/Cargo.toml --bin librispeech_eval -- \
  --manifest taurscribe-runtime/librispeech/eval_manifest.jsonl \
  --audio-root taurscribe-runtime/librispeech/LibriSpeech/test-clean \
  --out librispeech_results.csv
```

`--audio-root` is optional if every `flac_path` in the manifest still exists on disk.

Other flags: `--engines whisper,parakeet,cohere`, `--limit 50`, `--force-cpu`.

Model IDs (optional env): `TAURSCRIBE_WHISPER_MODEL_ID`, `TAURSCRIBE_PARAKEET_MODEL_ID`, `TAURSCRIBE_GRANITE_MODEL_ID`.

CSV columns: `utt_id, engine, wer, ref_word_count, hyp_snippet`. Mean / median WER print to stderr at the end.

**Note:** The CSV **`engine`** column is only `whisper`, `parakeet`, or `cohere` — it does **not** record which Whisper size, Parakeet bundle, or Cohere folder you used. For a multi-model sweep, use a **different `--out` path per model** (or add a column yourself when merging).

### 3b. WER on every installed model

`librispeech_eval` loads **one** checkpoint per engine **per process**: either the first one the app discovers, or the one you select with env vars (`TAURSCRIBE_WHISPER_MODEL_ID`, `TAURSCRIBE_PARAKEET_MODEL_ID`, `TAURSCRIBE_GRANITE_MODEL_ID`). There is no single flag that loops over all local models automatically.

**Approach:** run the binary multiple times — change the env var(s), keep the same manifest, and write to a new CSV each time (or use `--engines whisper` only while sweeping Whisper so Parakeet/Cohere are not repeated unnecessarily).

**Whisper IDs** match the `ggml-*.bin` stem after `ggml-` and before `.bin` (e.g. `tiny.en`, `base`, `small`). Example sweep on macOS (repo root):

```bash
MODELS="$HOME/Library/Application Support/Taurscribe/models"
MANIFEST=taurscribe-runtime/librispeech/eval_manifest.jsonl
ROOT=taurscribe-runtime/librispeech/LibriSpeech/test-clean
LIMIT=100   # optional: drop for full test-clean

for bin in "$MODELS"/ggml-*.bin; do
  [[ -f "$bin" ]] || continue
  case "$(basename "$bin")" in *silero*) continue ;; esac
  id=$(basename "$bin" .bin)
  id=${id#ggml-}
  echo "=== Whisper: $id ==="
  TAURSCRIBE_WHISPER_MODEL_ID="$id" \
  TAURSCRIBE_LIBRISPEECH_AUDIO_ROOT="$ROOT" \
  cargo run --release --manifest-path src-tauri/Cargo.toml --bin librispeech_eval -- \
    --manifest "$MANIFEST" --audio-root "$ROOT" --engines whisper --limit "$LIMIT" \
    --out "wer_whisper_${id//./_}.csv"
done
```

**Parakeet IDs** look like `nemotron:folder_name` (directory under the models folder that contains Nemotron ONNX files). Get exact strings from the app's model list or from folder names under `models/`. Loop the same way with `TAURSCRIBE_PARAKEET_MODEL_ID` and `--engines parakeet`.

**Cohere engine:** this uses a single q4f16 universal bundle under `granite-speech-1b`. Set `TAURSCRIBE_GRANITE_MODEL_ID=granite-speech-1b` (or `granite-speech-1b-cpu`) and run with `--engines cohere`.

**All engines × all Whisper variants:** run one full `--engines whisper,parakeet,cohere` job per Whisper ID (Parakeet/Cohere stay the same unless you also change those env vars). That quickly multiplies runtime and VRAM use — use `--limit` while iterating.

### 4. Integration tests (`cd src-tauri`)

```bash
# JFK smoke — needs jfk.wav + all three models installed
cargo test jfk_audio_through_whisper_parakeet_and_granite -- --ignored --nocapture

# Memory regression — needs jfk.wav + Whisper + Parakeet (Cohere optional)
TAURSCRIBE_LOG_MEMORY=1 \
  cargo test memory_engine_regression -- --ignored --nocapture

# Memory regression — write JSON report for diffing
TAURSCRIBE_LOG_MEMORY=1 TAURSCRIBE_MEMORY_REPORT=memory_report.json \
  cargo test memory_engine_regression -- --ignored --nocapture

# Memory regression — CPU only (no GPU required)
TAURSCRIBE_MEMORY_FORCE_CPU=1 \
  cargo test memory_engine_regression -- --ignored --nocapture

# Parakeet context reset — needs jfk.wav + Parakeet bundle
cargo test parakeet_clear_context_restores_session_baseline -- --ignored --nocapture

# Accuracy — needs manifest + corpus (use audio root if paths are stale)
TAURSCRIBE_EVAL_MANIFEST=../taurscribe-runtime/librispeech/eval_manifest.jsonl \
TAURSCRIBE_LIBRISPEECH_AUDIO_ROOT=../taurscribe-runtime/librispeech/LibriSpeech/test-clean \
  cargo test --test file_drop_accuracy --test mic_accuracy -- --ignored --nocapture
```

Skip without failing when models are missing: `TAURSCRIBE_ASR_SMOKE_SKIP=1`.

### 5. Run all ignored tests at once

```bash
cd src-tauri
TAURSCRIBE_EVAL_MANIFEST=../taurscribe-runtime/librispeech/eval_manifest.jsonl \
TAURSCRIBE_LIBRISPEECH_AUDIO_ROOT=../taurscribe-runtime/librispeech/LibriSpeech/test-clean \
TAURSCRIBE_LOG_MEMORY=1 \
  cargo test -- --ignored --nocapture
```

---

## Environment Variable Reference

| Variable | Used by | Effect |
| --- | --- | --- |
| `TAURSCRIBE_ASR_SMOKE_SKIP=1` | All ignored tests | Skip the test (pass silently) |
| `TAURSCRIBE_LOG_MEMORY=1` | App + memory regression | Print per-step memory snapshots to stdout |
| `TAURSCRIBE_MEMORY_REPORT=path.json` | `memory_engine_regression` | Write full JSON scenario report to path |
| `TAURSCRIBE_MEMORY_FORCE_CPU=1` | `memory_engine_regression` | Force CPU load path for all engines |
| `TAURSCRIBE_ORT_INTRA_THREADS=N` | App startup (`ort_session.rs`) | Override ORT global intra-op thread count (default 1) |
| `TAURSCRIBE_ORT_INTER_THREADS=N` | App startup (`ort_session.rs`) | Override ORT global inter-op thread count (default 1) |
| `TAURSCRIBE_ORT_CUDA_MEM_LIMIT_MB=N` | App startup (`ort_session.rs`) | Cap CUDA device arena in MB |
| `TAURSCRIBE_EVAL_MANIFEST=path` | `file_drop_accuracy`, `mic_accuracy` | Path to JSONL manifest |
| `TAURSCRIBE_LIBRISPEECH_AUDIO_ROOT=path` | `librispeech_eval`, accuracy tests | Override stale FLAC paths in manifest |
| `TAURSCRIBE_WHISPER_MODEL_ID=id` | `librispeech_eval`, smoke | Pin specific Whisper model (e.g. `base.en`) |
| `TAURSCRIBE_PARAKEET_MODEL_ID=id` | `librispeech_eval`, smoke | Pin specific Parakeet model (e.g. `nemotron:folder`) |
| `TAURSCRIBE_GRANITE_MODEL_ID=id` | `librispeech_eval`, smoke | Pin Cohere model dir name |
| `LIBRISPEECH_ROOT=path` | Download scripts | Override where test-clean is downloaded |
| `JFK_WAV=path` | Smoke + memory tests | Path to `jfk.wav` if not in `tests/fixtures/` |

---

## Model locations

Eval and tests load models from the same directory as the app:

| Platform | Path |
| --- | --- |
| Windows | `%LOCALAPPDATA%\Taurscribe\models\` |
| macOS | `~/Library/Application Support/Taurscribe/models/` |
| Linux | `~/.local/share/taurscribe/models/` |

Download models through the app (or place compatible files manually) before running engine-dependent tests.
