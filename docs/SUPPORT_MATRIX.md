# Taurscribe Support Matrix

Tracks which speech engine runs on which OS and hardware:
- **Now**: what the app does today, and how far each cell has been proven.
- **Target**: the most optimised runtime and model researched for that cell.

Feature coverage and its verification status appear after the hardware matrix.

Last updated: 2026-09-23

Minimum macOS: **14.0 (Sonoma)**, required by MLX 0.32. Raised from 13.4 on 2026-09-23.

## Legend

| Mark | Meaning |
|---|---|
| ✅ | Verified on real hardware (UI suite or harness run) |
| 🧪 | Verified in a VM or emulator only |
| 🟡 | Wired into source for that platform, but target build and runtime are not yet verified |
| 🔵 | Target: researched solution, not implemented |
| ❌ | No support today (falls back as noted) |
| — | Combination doesn't exist, so there is nothing to support |
|  | Runs on **MLX** in the app on Apple Silicon (the Apple logo is a private-use character, so it shows as a box on Windows and Linux) |

## Hardware columns

| Column | What it covers |
|---|---|
| **ARM** | The whole ARM chip: CPU plus its own GPU/NPU. Apple Silicon (Metal GPU + Apple Neural Engine), Snapdragon X (Oryon CPU, Adreno GPU, Hexagon NPU), ARM Linux boards |
| **x86 AMD** | AMD Ryzen/EPYC CPU cores |
| **x86 Intel** | Intel Core/Xeon CPU cores |
| **+ NVIDIA** | Discrete NVIDIA GPU (CUDA) |
| **+ AMD GPU** | Discrete AMD Radeon GPU |
| **Intel/AMD iGPU · NPU** | Integrated graphics and NPUs on x86 chips: Intel Iris Xe / Arc iGPU, Intel NPU (Core Ultra), AMD Radeon 7x0M/8x0M iGPU, AMD XDNA NPU (Ryzen AI). The Apple Neural Engine is in the ARM column. |

Where the dashes go:
- **macOS · x86 AMD**: no Mac ever shipped with an AMD CPU.
- **macOS · + NVIDIA**: Apple has had no NVIDIA drivers since macOS 10.14, and CUDA is gone.
- **macOS · iGPU/NPU**: only Intel Macs count (Intel UHD/Iris iGPU). No Mac has an x86 NPU.
- **macOS · + AMD GPU**: valid. Intel MacBook Pro 16", iMac and Mac Pro shipped with Radeon GPUs.

## Engine lineup

| Engine | Model | Apple Silicon runtime | Status |
|---|---|---|---|
| Whisper | whisper.cpp GGML (tiny → large-v3-turbo) + CoreML encoders | Metal + CoreML (ANE) | Shipping |
| Qwen3-ASR | Unquantized F16 GGUF, 1.7B and 0.6B | GGUF on Metal (an MLX build exists but was about 5× slower) | In app via transcribe.cpp; Files and a 1.7B Google Meet call verified on Apple M4 |
| Granite Speech 5 | Unquantized F16 GGUF, 470M CTC, English, non-commercial weights | GGUF on Metal (MLX was on par, so dropped for one runtime) | In app via transcribe.cpp; Files verified on Apple M4 |
|  Nemotron 3 Diarization | Speaker separation, 100M, up to 8 speakers. BF16 MLX on Apple Silicon, F16 GGUF elsewhere | **MLX** (mlx-rs 0.32) | In app for meetings (call channel). Model verified on Apple M4; full in-app meeting run pending |
| ~~Parakeet Nemotron~~ | NVIDIA Nemotron streaming (ONNX INT4 + MLX) | — | Removed from app model choices; old benchmark/tests still being retired |
| ~~Granite Speech 4.1~~, ~~Parakeet TDT~~ | — | — | Removed 2026-09-22 |

The app registry pins these exact downloads, all verified against their SHA-256 values in the installed model directory on 2026-09-22:

| Model ID | Hugging Face repository | File | SHA-256 |
|---|---|---|---|
| `granite-speech-5-nc` | `handy-computer/granite-speech-5.0-470m-turboctc-nc-gguf` | `granite-speech-5.0-470m-turboctc-nc-F16.gguf` | `baceebaaf85210f50463dfec059eb46ae6dbea8ea9a090500fbdee4f92b0c302` |
| `qwen3-asr-1.7b` | `handy-computer/Qwen3-ASR-1.7B-gguf` | `Qwen3-ASR-1.7B-F16.gguf` | `edb09c29b8f73822c639168d5ef72aa2dccdf8b4e48fc4b8518885352ff62c71` |
| `qwen3-asr-0.6b` | `handy-computer/Qwen3-ASR-0.6B-gguf` | `Qwen3-ASR-0.6B-F16.gguf` | `5c90e4b1a72a4c59cd12afa5ebb0cc8628848148f2b337d025cc5121ae4d2eea` |
| `diarization-nemotron3` ( Apple Silicon) | `mlx-community/Nemotron-3-Diarization` @ `59ed2db` | `model.safetensors` (BF16) | `21e8427d1795c9c46c5800f56b16061734ffd0dcadd71d9bcf0b4d6ef7261da5` |
| `diarization-nemotron3` (everything else) | `Glimpse-Dictation/Nemotron-3-Diarization-gguf` @ `3bf8566` | `nemotron-3-diarization-F16.gguf` | `5513da21cc39fc3ab5a36bd945324aeb013369b63172849b4ef174686e15f27c` |

One model ID per model on every platform; only the file behind `diarization-nemotron3` differs by platform.

---

## Runtime catalogue (the solutions)

| Runtime | Hardware it reaches | Models we care about | License | Fit |
|---|---|---|---|---|
| **MLX** (mlx-rs 0.32) | Apple Silicon GPU | Nemotron 3 Diarization | MIT | In app for diarization only, where it beats GGUF on Metal (see §6). Qwen3 and Granite stay on GGUF, which matched or beat their MLX ports |
| **whisper.cpp / ggml** (whisper-rs) | CPU, Metal, CUDA, Vulkan, HIP/ROCm, SYCL; encoder offload to CoreML, OpenVINO | Whisper | MIT | In app. whisper-rs exposes `cuda`, `vulkan`, `hipblas`, `intel-sycl`, `coreml`, and passes any `WHISPER_*` CMake flag through, so `WHISPER_OPENVINO=ON` works without a fork |
| **transcribe.cpp** ([handy-computer](https://github.com/handy-computer/transcribe.cpp)) | CPU, Metal, Vulkan, CUDA (depending on build) | Granite 5 (incl. **-nc**), Qwen3-ASR 0.6B/1.7B | MIT | Integrated through the `transcribe-cpp` Rust crate, pinned to the [Glimpse fork](https://github.com/LegendarySpy/transcribe.cpp/tree/glimpse-diarization) at `b893ed2`: upstream `ed3468f` plus Nemotron 3 Diarization, not yet upstreamed. Granite and Qwen3 output is identical on both. This checkout enables Metal on Apple ARM and CUDA + Vulkan on Windows/Linux x86; only the Mac Metal path has been run here. Unquantized F16 downloads are SHA-256 pinned |
| **ONNX Runtime** (ort) | CPU, CUDA, TensorRT-RTX, DirectML, CoreML, OpenVINO, VitisAI, QNN, MIGraphX | CAM++, Nemotron; possible NPU path for Granite 5 | MIT | In app. DirectML is in maintenance: Microsoft moved to **Windows ML**, which provisions the OpenVINO/QNN/VitisAI/TensorRT EPs. The ROCm EP was **removed in ORT 1.23**; AMD GPUs now go through MIGraphX (Linux) |
| **OpenVINO GenAI** | Intel CPU, iGPU, NPU (Windows + Linux) | Whisper (NPU supported); **Qwen3-ASR** on CPU/GPU (NPU support in progress, openvino.genai PR #4389) | Apache-2.0 | Best Intel iGPU/NPU route. C++ API; needs a small FFI shim |
| **AMD whisper.cpp fork** + Ryzen AI | AMD XDNA/XDNA2 NPU (encoder) | Whisper | MIT fork + AMD runtime | Windows only today; Linux "planned". The encoder runs on the NPU from a `.rai` cache; the decoder stays on ggml. Ryzen AI 1.8: 3 s of audio at RTF 0.139 (small), 0.337 (large-v3-turbo) |
| **FastFlowLM** | AMD XDNA2 NPU (Windows + Linux) | Whisper large-v3-turbo fully on the NPU | MIT code, free NPU kernels | The only AMD NPU route on Linux. Runs as a separate server process |
| **Qualcomm QNN** (ORT QNN EP / AI Hub) | Snapdragon Hexagon NPU | Whisper (AI Hub exports) | Qualcomm license | Needs static shapes and quantization. Community reports say the Adreno GPU often beats the NPU |

---

## 1. Whisper (whisper.cpp)

**Now**

| Whisper | ARM | x86 AMD | x86 Intel | + NVIDIA | + AMD GPU | Intel/AMD iGPU · NPU |
|---|---|---|---|---|---|---|
| **macOS** | ✅ Metal + CoreML encoder on ANE | — | 🧪 CPU (Rosetta) + CoreML encoder | — | 🟡 CoreML encoder may use the Radeon; decoder on CPU | 🟡 same (Intel iGPU) · NPU — |
| **Windows** | 🟡 CPU | 🟡 CPU | 🟡 CPU | ✅ CUDA (RTX 4070 Laptop) | ❌ CPU (Vulkan turned off in the Windows build) | ❌ CPU · ❌ NPU |
| **Linux** | 🧪 CPU (Docker) | 🟡 CPU | 🟡 CPU | 🟡 CUDA | 🟡 Vulkan | 🟡 Vulkan (iGPU) · ❌ NPU |

**Target**

| Whisper | ARM | x86 AMD | x86 Intel | + NVIDIA | + AMD GPU | Intel/AMD iGPU · NPU |
|---|---|---|---|---|---|---|
| **macOS** | ✅ as now | — | 🔵 CPU (AVX2) + CoreML encoder | — | 🔵 CoreML encoder on the Radeon (verify) | 🔵 CoreML encoder on the Intel iGPU (verify) · — |
| **Windows** | 🔵 CPU (NEON); try Vulkan on Adreno; Hexagon via QNN only if worth it | 🔵 CPU | 🔵 CPU | 🔵 CUDA | 🔵 **Re-enable `vulkan`**; HIP (ROCm 7 on Windows) optional | 🔵 Vulkan (iGPU) · Intel NPU: OpenVINO encoder · AMD NPU: AMD fork, VitisAI encoder |
| **Linux** | 🔵 CPU | 🔵 CPU | 🔵 CPU | 🔵 CUDA | 🔵 Vulkan, or `hipblas` (ROCm) for RDNA2+ | 🔵 Vulkan · Intel NPU: OpenVINO encoder · AMD NPU: FastFlowLM |

Notes:
- ggml's Metal backend targets Apple Silicon. On Intel Macs whisper.cpp falls back to CPU, so the only GPU path there is the CoreML encoder.
- The Windows x86 build compiles whisper.cpp with `cuda` only ("vulkan temporarily removed"). Turning `vulkan` back on is the cheapest fix in this whole doc: one build covers AMD Radeon, Intel Arc/Xe and AMD iGPUs.
- The Linux x86 build enables `cuda` + `vulkan`, so building it needs the CUDA toolkit.

## 2. Qwen3-ASR

**Now**

| Qwen3 | ARM | x86 AMD | x86 Intel | + NVIDIA | + AMD GPU | Intel/AMD iGPU · NPU |
|---|---|---|---|---|---|---|
| **macOS** | ✅ F16 GGUF / Metal (1.7B and 0.6B Files; 1.7B Meet) | — | 🧪 CPU (Rosetta) | — | 🟡 CPU | 🟡 CPU · — |
| **Windows** | 🔵 CPU (ARM build pending) | ✅ CPU (Ryzen 7 8845HS) | 🟡 CPU / Vulkan | ✅ CUDA and Vulkan (RTX 4070 Laptop) | 🟡 Vulkan (verified on the 780M iGPU, not a discrete Radeon) | ✅ Vulkan on AMD Radeon 780M · NPU ❌ |
| **Linux** | 🧪 CPU (Docker, ARM64) | 🟡 CPU / Vulkan | 🟡 CPU / Vulkan | 🟡 CUDA | 🟡 Vulkan | 🟡 Vulkan · NPU ❌ |

**Target** (improve or verify the integrated GGUF path)

| Qwen3 | ARM | x86 AMD | x86 Intel | + NVIDIA | + AMD GPU | Intel/AMD iGPU · NPU |
|---|---|---|---|---|---|---|
| **macOS** | ✅ GGUF Metal; profile memory and latency | — | 🔵 verify CPU, prefer **0.6B** | — | 🔵 verify CPU | 🔵 verify CPU · — |
| **Windows** | 🔵 native ARM build and Adreno Vulkan | 🔵 verify CPU / Vulkan | 🔵 verify CPU / Vulkan | 🔵 verify CUDA | 🔵 verify Vulkan | 🔵 verify Vulkan · Intel NPU: OpenVINO GenAI when supported |
| **Linux** | 🔵 native ARM build | 🔵 verify CPU / Vulkan | 🔵 verify CPU / Vulkan | 🔵 verify CUDA | 🔵 verify Vulkan / HIP | 🔵 verify Vulkan · Intel NPU: OpenVINO GenAI when supported |

Notes:
- **1.7B is too heavy for CPU-only machines.** transcribe.cpp measures it at 2.7× real time on a Ryzen 7 4750U CPU, 4.5× on its Vega iGPU (Vulkan) and 10× on an M4 Max CPU. On CPU-only and iGPU machines, offer **Qwen3-ASR-0.6B** (0.5 pp worse WER, about 2.5× cheaper).
- The former ONNX and MLX app paths have been removed. Both sizes now use one model registry and one GGUF runtime across platforms.
- OpenVINO INT8 exports exist (`dseditor/Qwen3-ASR-1.7B-INT8_OpenVINO`). They're quantized; an unquantized one can be exported with `optimum-intel`.
- The old MLX path reached about 10–12 GB of process memory on this 16 GB Mac. The new 1.7B GGUF path was about 5.4 GB after file inference in the 2026-09-22 development run. This is one observed working set, not a cross-platform peak-RAM guarantee.

## 3. Granite Speech 5.0 470M TurboCTC

**Now**

| Granite 5 | ARM | x86 AMD | x86 Intel | + NVIDIA | + AMD GPU | Intel/AMD iGPU · NPU |
|---|---|---|---|---|---|---|
| **macOS** | ✅ F16 GGUF / Metal (Files) | — | 🧪 CPU (Rosetta) | — | 🟡 CPU | 🟡 CPU · — |
| **Windows** | 🔵 CPU (ARM build pending) | ✅ CPU (Ryzen 7 8845HS) | 🟡 CPU / Vulkan | ✅ CUDA and Vulkan (RTX 4070 Laptop) | 🟡 Vulkan (verified on the 780M iGPU, not a discrete Radeon) | ✅ Vulkan on AMD Radeon 780M · NPU ❌ |
| **Linux** | 🧪 CPU (Docker, ARM64) | 🟡 CPU / Vulkan | 🟡 CPU / Vulkan | 🟡 CUDA | 🟡 Vulkan | 🟡 Vulkan · NPU ❌ |

**Target** (verify the integrated GGUF path; optional later accelerators)

| Granite 5 | ARM | x86 AMD | x86 Intel | + NVIDIA | + AMD GPU | Intel/AMD iGPU · NPU |
|---|---|---|---|---|---|---|
| **macOS** | ✅ GGUF Metal; benchmark against historic MLX | — | 🔵 verify CPU | — | 🔵 verify CPU | 🔵 verify CPU · — |
| **Windows** | 🔵 native ARM build and Adreno Vulkan | 🔵 verify CPU / Vulkan | 🔵 verify CPU / Vulkan | 🔵 verify CUDA | 🔵 verify Vulkan | 🔵 verify Vulkan · NPU low priority |
| **Linux** | 🔵 native ARM build | 🔵 verify CPU / Vulkan | 🔵 verify CPU / Vulkan | 🔵 verify CUDA | 🔵 verify Vulkan / HIP | 🔵 verify Vulkan · NPU low priority |

Why a plain CPU is enough here:
- transcribe.cpp measures Granite 5 at **18× real time on a Ryzen 7 4750U CPU** (27× on its iGPU through Vulkan), and 45× on an M4 Max CPU.
- A 10 s dictation costs about 0.5 s even on a 2020 laptop with no GPU.
- NPUs would mostly save battery, not time. They need an ONNX export of the NC weights plus a vendor EP, so they come last.

Weights:

| Source | Repo | Precision | Size | License |
|---|---|---|---|---|
| IBM original (NC) | `ibm-granite/granite-speech-5.0-470m-turboctc-nc` | bf16 | 902 MB | CC-BY-NC-SA-4.0 |
| MLX, no quantization (NC), historic benchmark only | `iky1e/granite-speech-5.0-470m-turboctc-nc-mlx-fp16` | fp16 | 902 MB | CC-BY-NC-SA-4.0 |
| GGUF (NC), **app default on all platforms** | `handy-computer/granite-speech-5.0-470m-turboctc-nc-gguf` | F16 | 949 MB | CC-BY-NC-SA-4.0 |
| IBM original (commercial) | `ibm-granite/granite-speech-5.0-470m-turboctc` | bf16 | 902 MB | Apache-2.0 |
| ONNX (commercial weights only) | `qwertz92/granite-speech-5.0-470m-turboctc-onnx` | fp32 / fp16 / int8 | 1.9 GB / 947 MB / 551 MB | Apache-2.0 |

- The app uses unquantized F16 per the project's no-quantization preference. F16 rather than BF16 because ggml's CPU path is slow on BF16: on an Apple M4 CPU, JFK (11 s) took 5.9 s in BF16 and 0.53 s in F16. On Metal the two are within noise. Quantized Granite builds are not in the model picker.
- The NC model was trained on about 15k more hours (GigaSpeech + SPGISpeech). transcribe.cpp measures 1.29% WER vs 1.33% for Apache on LibriSpeech test-clean.

Decisions:
- [x] **License**: NC. Taurscribe is open source and non-commercial. Both variants output the same lowercase, normalised text.
- [x] **Non-Mac runtime**: transcribe.cpp (GGUF) rather than ONNX Runtime. One integration covers CPU, CUDA, Vulkan (AMD/Intel/iGPU/Adreno) and HIP. It also runs the NC weights, which have no ONNX export.
- [x] **Live dictation latency**: measured below. Transcribing the whole utterance on release beats Nemotron's streaming flush for every utterance tested.
- [ ] **Casing and punctuation**: Granite needs a post-pass for dictation (FlowScribe grammar LLM, or a small punctuation/truecase model).

**Historic MLX port** (removed from current app build):
- Pure Rust front-end plus an MLX encoder.
- Parity against transformers `GraniteSpeech5ForCTC` (fp32, CPU):
  - Front-end max |diff| is 2.5e-5.
  - Transcripts are identical on 14/14 clips (2–57 s) with both IBM's bf16 weights and the `iky1e` MLX fp16 weights.
- Parity check: `cargo run --release --example granite5_parity`.

**Granite 5 vs Nemotron, both MLX** (2026-09-22, Apple M4 with a 10-core GPU and 16 GB, 200 LibriSpeech test-clean utterances, 1,503 s of audio; `examples/asr_latency_bench.rs`):

| | Granite 5 (fp16) | Nemotron (MLX) |
|---|---|---|
| Throughput (RTFx) | **113×** | 5.2× |
| Release → text, median | **59 ms** | 190 ms |
| Release → text, p90 | **120 ms** | 195 ms |
| Release → text, max (35 s utterance) | 194 ms | 203 ms |
| WER, all utterances | 4.28% | **3.16%** |
| WER, utterances without contractions/numbers (146) | **1.55%** | 2.80% |
| Memory after load | **979 MB** | 1,269 MB |
| Warm-up | **0.07 s** | 1.37 s |

How release latency is measured:
- **Nemotron** streams 560 ms chunks while you speak, so release latency is the flush of the tail plus 400 ms of silence.
- **Granite** transcribes the whole utterance on release.

For reference, transcribe.cpp publishes 35 ms for JFK (11 s) on an M4 Max (Metal, Q8_0). We measure 89 ms on a base M4. A same-machine comparison is still to do.

Output style is the trade-off:
- Granite writes lowercase text with no punctuation.
- It expands contractions ("i am") and writes numbers as digits ("27th 1837").
- Possessives sometimes become "is" ("man's" → "man is").
- These formatting differences explain the whole WER gap. On words both engines format the same way, Granite makes about half as many errors.

## 4. Parakeet Nemotron (historic; removed from current app)

| Nemotron | ARM | x86 AMD | x86 Intel | + NVIDIA | + AMD GPU | Intel/AMD iGPU · NPU |
|---|---|---|---|---|---|---|
| **macOS** | ❌ | — | ❌ | — | ❌ | ❌ · — |
| **Windows** | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ · ❌ |
| **Linux** | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ · ❌ |

If a streaming engine is ever wanted again, transcribe.cpp also runs Nemotron streaming, with the same backends as Granite 5.

## 5. Nemotron 3 Diarization (speaker separation)

Splits the call channel of a meeting into the people on it (up to 8). You are always the mic channel, so that needs no model. It runs once, when a meeting ends. People are recognised across meetings by CAM++ (§6), not by this model. Without it, the app falls back to its old energy-and-pitch clustering.

**Now**

| Diarization | ARM | x86 AMD | x86 Intel | + NVIDIA | + AMD GPU | Intel/AMD iGPU · NPU |
|---|---|---|---|---|---|---|
| **macOS** | ✅  MLX (BF16) | — | 🟡 GGUF CPU | — | 🟡 GGUF CPU | 🟡 GGUF CPU · — |
| **Windows** | 🟡 GGUF CPU | 🟡 GGUF CPU | 🟡 GGUF CPU / Vulkan | 🟡 GGUF CUDA | 🟡 GGUF Vulkan | 🟡 GGUF Vulkan · NPU ❌ |
| **Linux** | 🟡 GGUF CPU | 🟡 GGUF CPU / Vulkan | 🟡 GGUF CPU / Vulkan | 🟡 GGUF CUDA | 🟡 GGUF Vulkan | 🟡 GGUF Vulkan · NPU ❌ |

The ✅ is for the model, not the whole meeting flow: the Rust MLX port was checked against mlx-audio and on synthetic meetings (below). A real call through the app is still to do. The GGUF cells share transcribe.cpp's backends with Granite and Qwen3, which are verified on Windows; the diarizer itself has only been run on the Mac (Metal and CPU).

**Accuracy** on synthetic meetings built from LibriSpeech test-clean (speakers take turns, no overlap, clean audio). "Mixed up" is the share of speech given to the wrong person:

| Meeting | Old clustering: mixed up / speakers found | Nemotron 3: mixed up / speakers found |
|---|---|---|
| 2 people, 3 min | 34% / 1 of 2 | **0%** / 2 of 2 |
| 4 people, 4 min | 31% / 3 of 4 | **0%** / 4 of 4 |
| 6 people, 5 min | 57% / 2 of 6 | **0%** / 6 of 6 |
| 4 people, 10 min | 40% / 3 of 4 | **0%** / 4 of 4 |

GGUF (F16), mlx-audio (BF16) and the Rust MLX port give the same scores. The Rust port agrees with mlx-audio on 99.995% of 10 ms speaker decisions.

**Speed and memory on Apple M4** (warm, whole process peak):

| Runtime | 4 min | 10 min | 60 min | Peak memory, 4 min → 60 min |
|---|---|---|---|---|
|  **MLX, Rust port (app)** | 1.2 s | 2.8 s | **15.1 s (242×)** | 1.05 GB → **1.28 GB** |
| GGUF on Metal (transcribe.cpp) | 1.9 s | 3.2 s | 19.3 s (189×) | **0.43 GB** → 1.8 GB |
| GGUF on CPU (transcribe.cpp) | — | 20.7 s (29×) | — | 0.56 GB at 10 min |
| mlx-audio (Python reference, not shipped) | 0.9 s | 2.3 s | 12.4 s (295×) | — |

- MLX is faster at every length. On memory the two cross over: GGUF holds the whole recording, so it grows about 0.35 MB per second of audio. MLX works on 27 s windows and has a fixed cost of about 0.45 GB for one window's GPU buffers, released when it finishes.
- The Rust port uses mlx-rs 0.32 (MLX 0.32). On mlx-rs 0.25 (MLX 0.25) the same code ran at 131×.
- Weights are 199 MB in both formats.

## 6. Supporting models

| Model | Runtime | macOS ARM | macOS Intel | Windows | Linux | Target |
|---|---|---|---|---|---|---|
| Speaker Recognition (CAM++, 28 MB) | ONNX CPU | ✅ | 🟡 | 🟡 | 🟡 | Keep on CPU everywhere. The model is tiny, so GPU transfer would cost more than it saves |
| Speaker Separation (Nemotron 3, 199 MB) | MLX / transcribe.cpp | ✅  MLX | 🟡 GGUF CPU | 🟡 GGUF (CUDA / Vulkan / CPU) | 🟡 GGUF (CUDA / Vulkan / CPU) | See §5 |
| FlowScribe grammar LLM (Qwen2.5 0.5B GGUF) | llama.cpp | ✅ Metal | 🟡 CPU | 🟡 **CPU only** | 🟡 CUDA (x86) / CPU (ARM) | Enable llama-cpp-2 `vulkan` on Windows + Linux (AMD/Intel/NVIDIA GPUs, iGPUs). `rocm` is optional on Linux. A 0.5B model is fine on CPU if a build is problematic |
| RNNoise | pure Rust | ✅ | 🟡 | 🟡 | 🟡 | Keep on CPU. It needs microseconds per frame |

---

## Recommended order of work

1. **Finish proving the integrated GGUF path.** Compile and run Windows/Linux x86 binaries in the available VMs/emulators, then test CUDA/Vulkan on real hardware. Check macOS Intel CPU under Rosetta. Confirm an installer can select a working backend without a GPU toolchain installed.
2. **Complete app acceptance checks.** Prove literal drag/drop (native Browse already passes), all three ASR families in mic dictation and real meetings, and recording duration on the process-specific callers tap.
3. **Turn Vulkan back on** for whisper.cpp and llama.cpp in the Windows build. This covers AMD GPUs and all iGPUs for Whisper and FlowScribe. It needs the Vulkan SDK in CI.
4. **Intel Mac check.** Confirm whether the CoreML encoder runs on the Radeon or Intel iGPU; otherwise document CPU-only.
5. **NPUs, one vendor at a time, after everything above.**
   - Intel: the OpenVINO encoder for whisper.cpp (`WHISPER_OPENVINO=ON`), then OpenVINO GenAI for Qwen3-ASR once its NPU support lands.
   - AMD: the AMD whisper.cpp fork's VitisAI encoder on Windows; FastFlowLM on Linux.
   - Qualcomm: QNN EP, but only if the Adreno Vulkan path turns out slower.

---

## Cross-platform verification (2026-09-23)

`cargo test --all-targets` plus JFK (11 s) through every model with `examples/gguf_probe.rs`. Transcripts matched the Apple M4 output on every platform.

| Platform | How | Tests | Result |
|---|---|---|---|
| macOS ARM (Apple M4) | Native | 278 passed, 7 ignored | ✅ |
| macOS x86_64 | Rosetta 2, `x86_64-apple-darwin` build | 277 passed, all targets | 🧪 All models transcribe on the CPU. Timings not meaningful: Rosetta has no AVX |
| Linux ARM64 | Docker (Ubuntu 24.04) on the M4 | All pass | 🧪 CPU: Granite 0.81 s, Qwen3 0.6B 5.3 s |
| Windows x86_64 | Real PC: Ryzen 7 8845HS, RTX 4070 Laptop (8 GB), Radeon 780M, 15 GB | 82 + 16 + 179 passed, all targets | ✅ Release config (CUDA + Vulkan) |
| Linux x86_64 | Docker under Rosetta, CUDA 12.6 + Vulkan config | Not completed | Partial: configure and whisper.cpp's CUDA build succeed, then the emulated transcribe.cpp CUDA build stalls under Rosetta (twice). Surfaced the SPIRV-Headers requirement below. Needs a real Linux x86_64 machine or CI |
| Windows ARM64 | UTM VM | — | Dropped in favour of the real PC; not verified |

Windows x86_64 real-hardware timings for JFK (11 s), warm:

| Model | CUDA (RTX 4070) | Vulkan (RTX 4070) | Vulkan (Radeon 780M iGPU) | CPU (Ryzen 7 8845HS) |
|---|---|---|---|---|
| Granite 5 F16 | 77 ms | 105 ms | 480 ms | 0.80 s |
| Qwen3-ASR 0.6B F16 | 365 ms | 336 ms | 1.0 s | 2.3 s |
| Qwen3-ASR 1.7B F16 | 715 ms | 718 ms | 2.5 s | 5.1 s |
| Whisper base.en (whisper.cpp CUDA) | ~210 ms | — | — | — |

- `Auto` picks CUDA when an NVIDIA GPU is present.
- On Vulkan, the very first run after install took about 10 s: the driver compiles shaders once and caches them.

Build requirements these runs exposed:
- **Vulkan backend (Windows and Linux x86_64):** transcribe.cpp's ggml Vulkan backend needs the SPIRV-Headers CMake package. The full Vulkan SDK ships it; both `build.yml` and `release.yml` install that SDK (1.4.309) on Windows and Linux x64. Local Linux builds without the SDK need Ubuntu's `spirv-headers` package.
- **`vulkan-1.lib` on Windows:** nothing added the SDK's `Lib` folder to the linker search path. `build.rs` now adds `%VULKAN_SDK%\Lib`.
- **Linux dev builds:** llama-cpp-sys hard-links its `.so` files into the target folder and fails with "File exists" on a rebuild. CI already deletes them before building; local rebuilds need the same step.

Bugs found and fixed through these runs:
- Linux compile errors in the meeting detector and the dual-channel stub.
- `mrec_probe` example on Linux.
- Blank CPU name on ARM Linux.
- Three tests that pasted into the frontmost app (now opt-in).

## Test rigs: what each cell can be verified on

| Rig | Covers | Limits |
|---|---|---|
| This Mac (Apple M4) | macOS ARM: Metal, MLX, ANE | Real hardware |
| Tart macOS 13/14/15 VMs | macOS ARM OS versions | Paravirtual GPU; MLX/Metal behaviour in a VM is not representative |
| Rosetta 2 x86_64 build | macOS Intel (CPU paths) | No Intel iGPU or Radeon |
| UTM Windows 11 ARM | Windows ARM CPU; DirectML via WARP (software) | No real GPU or NPU |
| Linux arm64 VM | Linux ARM CPU; Vulkan via lavapipe (software) | No real GPU |
| Linux/Windows x86 under emulation | x86 CPU correctness only | Very slow; timings meaningless |
| **Needs real hardware or a cloud GPU** | CUDA, ROCm/HIP, Vulkan on real GPUs, Intel/AMD iGPUs, every NPU | Can't be simulated: software backends prove the code path, not the driver |

## Feature matrix (Apple M4 development build, 2026-09-22)

This records observed behavior, not a promise for other OSes. “Wired” means the code path exists but this exact model/feature combination was not exercised in this run.

| Feature | Whisper | Granite Speech 5 F16 | Qwen3 1.7B F16 | Qwen3 0.6B F16 |
|---|---|---|---|---|
| Files via native Browse | ✅ 4 installed variants, 36/36 checks | ✅ 9/9 checks | ✅ 9/9 checks | ✅ 9/9 checks |
| Literal OS drag-and-drop | Wired; not separately exercised | Wired; not separately exercised | Wired; not separately exercised | Wired; not separately exercised |
| Mic dictation / transcript feed / clipboard paste | ✅ 4 installed variants | ✅ | ✅ | ✅ |
| Real Google Meet dual-channel recording/transcript | ✅ tiny Q5_1; 38.0s playback | ✅ 37.8s playback | ✅ 37.6s playback | ✅ 38.1s playback |
| Speaker vault / voiceprints | Shared post-processing path; not retested per model | Same | Same | Same |
| FlowScribe grammar pass | Shared optional pass; not retested per model | Same | Same | Same |

The Files tests selected each model, imported a spoken WAV through the native Open dialog, checked the transcript and saved engine/model history, then restored the prior selection. The four Meet tests used real calls with overlapping host/guest speech, not mock API feeds. Each verified two-channel recording, separate speaker turns, no cross-channel word leakage, saved playback, and meeting detection clearing after hangup. The duration bug (a roughly 38-second call saved as roughly 57 seconds) was traced to repeated padding for asynchronous packet arrivals in `audio_dual_channel.rs`; after removing that padding, each re-run saved roughly 38 seconds.

Evidence: `scripts/harness/reports/ui-20260922-223734`, `ui-20260922-223837`, `ui-20260922-224620`, `ui-20260922-230615` (Files), `ui-20260922-230839` (dictation, 42/42 checks), and post-fix Meet reports `meet-20260922-225750`, `meet-20260922-230003`, `meet-20260922-230159`, `meet-20260922-230358`.

Remaining gaps: literal OS drag gesture (native Browse and the shared `addPaths` path are proven), overlay/hotkey automation beyond the tested dictation button, Windows/Linux builds and runtime, and the old all-target benchmark/test files that still reference deleted MLX/Parakeet modules. `cargo test --lib` passes 82 tests, but `cargo check --all-targets` is not clean yet.

## Sources

- transcribe.cpp: [repo and model table](https://github.com/handy-computer/transcribe.cpp); per-model speed and WER in `docs/models/granite-speech-5.0-470m-turboctc-nc.md`, `qwen3-asr-1.7b.md` and `whisper-large-v3-turbo.md` (M4 Max and Ryzen 7 4750U rigs, 2026-09-14).
- ONNX Runtime: [DirectML EP](https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html) (sustained engineering), [Windows ML execution providers](https://learn.microsoft.com/en-us/windows/ai/new-windows-ml/supported-execution-providers), [ROCm EP removed in 1.23](https://onnxruntime.ai/docs/execution-providers/ROCm-ExecutionProvider.html), [MIGraphX EP](https://onnxruntime.ai/docs/execution-providers/MIGraphX-ExecutionProvider.html), [QNN EP](https://onnxruntime.ai/docs/execution-providers/QNN-ExecutionProvider.html), [Vitis AI EP](https://onnxruntime.ai/docs/execution-providers/Vitis-AI-ExecutionProvider.html).
- Intel: [OpenVINO release notes](https://docs.openvino.ai/releasenotes) (ASRPipeline with Whisper and Qwen3-ASR), [GenAI on NPU](https://docs.openvino.ai/2026/openvino-workflow-generative/inference-with-genai/inference-with-genai-on-npu.html), [Qwen3-ASR NPU PR #4389](https://github.com/openvinotoolkit/openvino.genai/pull/4389), [llama.cpp OpenVINO backend](https://github.com/ggml-org/llama.cpp/blob/master/docs/backend/OPENVINO.md) (text-only, audio work in progress).
- AMD: [Ryzen AI whisper.cpp support](https://ryzenai.docs.amd.com/en/latest/whisper_cpp.html), [Ryzen AI 1.8 release notes](https://ryzenai.docs.amd.com/en/latest/relnotes.html), [amd/whisper.cpp](https://github.com/amd/whisper.cpp), [FastFlowLM](https://github.com/FastFlowLM/FastFlowLM), [Whisper on an AMD NPU under Linux](https://dev.to/jac-76/running-whisper-llms-on-an-amd-npu-under-linux-2o1h), [llama.cpp with ROCm on Windows](https://rocm.docs.amd.com/projects/radeon-ryzen/en/latest/docs/advanced/advancedrad/windows/llm/llamacpp.html).
- Qualcomm: [llama.cpp Snapdragon backends](https://github.com/ggml-org/llama.cpp/blob/master/docs/backend/snapdragon/README.md) (CPU, Adreno OpenCL, Hexagon), [ORT on Snapdragon NPU](https://onnxruntime.ai/docs/genai/howto/build-models-for-snapdragon.html).
- Intel Macs: [whisper.cpp Metal on Intel Macs](https://github.com/ggml-org/whisper.cpp/issues/1292) falls back to CPU.
