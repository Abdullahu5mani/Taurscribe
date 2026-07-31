# Granite Speech NAR — MLX/Metal backend

An MLX implementation of `ibm-granite/granite-speech-4.1-2b-nar`, replacing the
ONNX Runtime path on Apple Silicon.

## Why this exists

The shipped bundle is an INT4 `MatMulNBits` ONNX export built for DirectML. On
macOS it was run through ONNX Runtime's CoreML execution provider, which was
**~41% slower than plain CPU**.

The compiled CoreML cache shows why: the encoder was split into **132
partitions**, and the op histogram across all of them contains only 32 `matmul`
ops — the attention score matmuls, which carry no weights and so were never
quantized. Every INT4 weight matmul fell back to CPU. Each forward pass ran a
small CoreML fragment, copied to CPU for the matmul, copied back, 132 times.

The deeper problem is the quantization choice. INT4 weight-only quantization
accelerates *memory-bound* autoregressive decode, where you stream 2B weights to
produce one token. This model is non-autoregressive: the editor consumes the
whole ~250-token sequence in a single bidirectional pass, which is
**compute-bound**. INT4 bought no speed, added a dequantize to every matmul, and
disqualified the graph from every GPU execution provider.

A roofline check confirms nothing was wrong with the CPU path itself:
~1.6 TFLOP/utterance ÷ 8.50 s ≈ 188 GFLOPS, which is what an M3's 8 CPU cores
deliver on fp32 GEMM. The model had simply never run on the GPU.

## What changed

| | ONNX bundle | this |
|---|---|---|
| numeric format | INT4 weight-only (`MatMulNBits`) | dense fp16 |
| sequence length | fixed 800-frame bucket | dynamic |
| tied embedding | exported twice (822 MB fp32 + again in editor) | stored once |
| BatchNorm | separate graph nodes | folded into depthwise conv |
| CTC head | all 200 pooled positions | valid positions only |

The fixed 800-frame bucket was also a correctness bug: 800 frames is 16 s, and
the model's own sample clip is 16.87 s, so it was being silently truncated.

## Results (M3, 8-core GPU, 16 GB)

Measured on LibriSpeech test-clean. Baselines are the previous session's
matched run on the same machine.

| backend | n | mean latency | RTF | WER |
|---|---|---|---|---|
| ONNX CoreML EP | 50 | 11.98 s | 1.715 | — |
| ONNX portable CPU | 50 | 8.50 s | 1.218 | 2.83% |
| MLX fp16, burst | 50 | 0.427 s | 0.057 | 0.82% |
| MLX fp16, sustained | 500 | 0.925 s | 0.122 | 1.36% |
| **MLX fp16 + text-only lm_head** | **500** | **0.745 s** | **0.098** | **1.36%** |

**11.4x faster than the CPU baseline**, at better WER.

Quote the 500-utterance row. The 50-utterance run is ~2x faster per second of
audio on identical average clip length, which is thermal: 500 utterances is
~11 minutes of continuous GPU load on a machine with limited sustained cooling.
Real dictation is bursty and idles between utterances, so live latency should
sit between the two rows — but the sustained figure is the honest floor.

The 50-utterance WER (0.82%) is likewise optimistic; the first 50 clips are an
easy subset. 1.36% over 500 is the number to trust.

fp16 and fp32 produce byte-identical transcripts across the 50-utterance set, so
fp16 is the default.

Note the WER figures come from different harnesses (this one vs the Rust
`librispeech_wer` path), so treat the WER *comparison* as indicative; the
latency comparison is same-machine and direct.

### Scaling

Latency is linear in audio length with a flat RTF, so there is no pathological
behavior at either end:

| audio | n | mean latency | RTF |
|---|---|---|---|
| 0–3 s | 70 | 0.432 s | 0.179 |
| 3–6 s | 177 | 0.617 s | 0.142 |
| 6–9 s | 105 | 0.876 s | 0.118 |
| 9–12 s | 62 | 1.145 s | 0.110 |
| 12–15 s | 35 | 1.528 s | 0.114 |
| 15–18 s | 28 | 1.678 s | 0.104 |

The elevated RTF on sub-3-second clips is fixed per-call overhead, not a
compute problem — it is ~0.4 s of floor regardless of length.

Dynamic shapes were checked as a possible source of per-shape kernel
recompilation cost. Utterances whose frame count had been seen before are
faster in aggregate, but once duration is normalized out that gap disappears
(per-bucket deltas scatter around zero), so it is a duration confound rather
than a compilation effect. No bucketing is needed.

## Where the time goes

Per-stage medians at fp16, measured with `profile_stages.py` (each stage forced
to evaluate before the clock is read, since MLX is lazy):

| stage | 2 s audio | 8 s | 16 s |
|---|---|---|---|
| conformer encoder x16 | 79.2 ms (50%) | 193.4 ms (55%) | 347.8 ms (58%) |
| editor backbone x40 | 62.6 ms (40%) | 127.7 ms (36%) | 198.7 ms (33%) |
| lm_head | 8.9 ms | 17.6 ms | 26.0 ms |
| projector | 5.6 ms | 13.7 ms | 24.3 ms |
| CTC argmax | 0.8 ms | 2.2 ms | 3.5 ms |

The encoder dominates, not the editor. Within a conformer block at 800 frames
(`profile_conformer.py`), the conv module is the weak spot:

| ff1 | attn | conv | ff2 | post_norm |
|---|---|---|---|---|
| 5.44 ms | 3.92 ms | 7.82 ms | 5.42 ms | 0.18 ms |

and inside conv, the depthwise convolution is 3.59 of 7.70 ms — **47% of the
module's time for 0.5% of its FLOPs**. MLX's grouped-conv path with
`groups == channels` runs at roughly 14 GB/s effective, far under both the
memory ceiling and the ~2.5 TFLOPS the dense projections achieve.

### Optimizations tried

| change | result |
|---|---|
| text-only `lm_head` | **kept** — 19.5% faster, 0 transcript changes |
| `mx.compile` on blocks/layers | rejected — a wash (-16% to +7%), not dispatch-bound |
| depthwise conv as shifted mul-adds | rejected — a wash (+13%/-13%), and 1e-3 drift |
| bf16 instead of fp16 | rejected — identical speed |

The dense projections run at ~2.5 TFLOPS, roughly 60% of this M3's practical
fp16 GEMM ceiling, so the remaining headroom is modest. The depthwise conv is
the one clear inefficiency left and it resisted the obvious rewrite.

## Footprint tradeoff

Dropping INT4 costs disk and memory: **4.63 GB fp16 vs 2.40 GB** for the INT4
bundle. `load_model(..., bits=8)` and `bits=4` use MLX's native groupwise affine
quantization, which — unlike ORT's `MatMulNBits` under CoreML — has real fused
Metal kernels. Treat it as a footprint knob only; this workload is
compute-bound, so quantization will not make it faster.

## Layout

| file | purpose |
|---|---|
| `granite_nar_mlx.py` | model, weight conversion, quantization |
| `capture_reference.py` | dump PyTorch fp32 activations to `.npz` |
| `check_parity.py` | per-stage numerical comparison against that reference |
| `bench.py` | `latency` and `wer` subcommands |

## Usage

```bash
uv venv .venv-granite-mlx --python 3.12
uv pip install --python .venv-granite-mlx/bin/python \
    "transformers>=5.5.3" torch torchaudio soundfile mlx numpy huggingface_hub

# ground truth, then stage-by-stage parity
.venv-granite-mlx/bin/python scripts/granite_mlx/capture_reference.py --out ref.npz
.venv-granite-mlx/bin/python scripts/granite_mlx/check_parity.py --ref ref.npz --dtype float32

# benchmarks
.venv-granite-mlx/bin/python scripts/granite_mlx/bench.py latency --runs 5
.venv-granite-mlx/bin/python scripts/granite_mlx/bench.py wer \
    --root taurscribe-runtime/librispeech/LibriSpeech/test-clean --limit 50
```

The model's custom code requires `transformers>=5.5.3`, which is why this uses a
separate venv rather than the repo's global Python.

## Parity

`check_parity.py` feeds the reference's own `input_features` into the MLX model,
excluding the mel front end so the ported graph is isolated. In fp32 every stage
matches to ≤3.5e-6 relative error, with identical CTC ids, final ids, and
transcript.

In fp16 the conformer drifts enough to change one intermediate CTC token
(45 vs 46), but the editor corrects it and the final output is unchanged —
which is the NAR editor doing exactly its job.

Non-obvious details that a reimplementation has to get right are documented
inline; the ones that decode to plausible-but-wrong text rather than crashing
are called out in comments at their site.

## Optimisation attempts — all dead ends

Measured end-to-end on the real `transcribe_ids` path (M3, fp16). Everything
lands inside run-to-run noise, so the port is already near the hardware ceiling:

| lever | end-to-end | output tokens | script |
|---|---|---|---|
| whole-encoder `mx.compile` | +0.4% | identical | `end2end_compile.py` |
| 8-bit weights | +0.9% | **0 of 44 change** | `quant_test.py` |
| 4-bit weights | +1.2% | **44 of 44 wrong** | `quant_test.py` |
| depthwise conv as shifts | ~1% (isolated) | identical | `profile_conv.py` |

4-bit destroys the output: a NAR editor has no autoregressive loop to
self-correct, the same reason the INT4 ONNX bundle needed the editor left alone.
8-bit is free accuracy-wise and roughly halves editor + embedding weight memory,
so it is worth having for low-RAM Macs — just don't expect it to be faster.

### The `mx.eval` profiling trap

`profile_stages.py` puts an `mx.eval()` barrier between stages so each can be
timed. That **overcounts**: it defeats MLX's lazy cross-stage scheduling. It
attributed 58% of latency to the encoder and implied a 41% win from compiling
it; on the real path that win is +0.4%. Treat per-stage numbers as upper bounds
and confirm anything promising end-to-end before believing it.

## Not done yet

Rust integration. The app calls ONNX Runtime through `ort`; using this from
`src-tauri` needs either the [`mlx-rs`](https://github.com/oxideai/mlx-rs) crate
(0.25.3, tracks an older MLX than the 0.32 used here) or a small C shim over
`mlx-c`. The Python implementation is the validated reference to port against.
