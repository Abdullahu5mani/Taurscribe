---
license: apache-2.0
base_model: Qwen/Qwen3.5-0.8B
language:
- en
pipeline_tag: text-generation
tags:
- dictation
- speech-to-text
- text-normalization
- gguf
- taurscribe
---

# FlowScribe v3 (Qwen3.5-0.8B)

FlowScribe turns raw speech-recognition output into the text the speaker meant. It is the on-device clean-up model in [Taurscribe](https://github.com/Abdullahu5mani/Taurscribe), a private, offline dictation app.

Given a transcript and a few tags, it:

- removes fillers, stutters and false starts ("um", "the the")
- resolves self-corrections ("at three, no wait, four" → "at 4")
- writes numbers, dates, times, money, emails, URLs and file paths properly
- applies spoken punctuation and layout ("comma", "new paragraph", "bullet point"), but leaves those words alone when they're meant literally
- spells custom dictionary terms correctly even when the recognizer misheard them ("tory" → "Tauri")
- formats for the app being typed into (email, chat, notes, code editor, terminal, …)

It never summarizes, rewrites tone or adds content.

## Files

| File | What |
|---|---|
| `flowscribe-v3-f16.gguf` | Full-precision GGUF for llama.cpp (what Taurscribe downloads) |
| `hf/` | Merged Hugging Face checkpoint (same architecture as Qwen3.5-0.8B) |
| `lora/` | The LoRA adapter (MLX format) |
| `eval_report.md` | Full evaluation breakdown |

## Prompt format

Qwen3.5 chat template with thinking disabled. System prompt:

```
You are FlowScribe. Rewrite the dictation in <text> as the speaker meant it, following the tags. Output only the result.
```

User message:

```
<engine=granite|whisper|qwen3> <level=verbatim|clean|formatted> <app=email|chat|notes|document|code_editor|terminal|ai_prompt|search|calendar_task|generic> <vocab=Term1; Term2>
<prev>optional text already in the document</prev>
<text>the transcript</text>
```

- `engine`: which recognizer produced the text (`granite` = raw lowercase, no punctuation; `whisper`/`qwen3` = punctuated).
- `level`: `verbatim` keeps every word (only punctuation, casing and written forms); `clean` removes disfluencies and resolves corrections; `formatted` also applies the app's conventions.
- `vocab` and `prev` are optional.

Rendered prompt (what llama.cpp receives):

```
<|im_start|>system
You are FlowScribe. …<|im_end|>
<|im_start|>user
<engine=granite> <level=clean> <app=chat> <vocab=Tauri>
<text>um lets meet at three no wait four in the tory channel</text><|im_end|>
<|im_start|>assistant
<think>

</think>

```

Use greedy decoding.

## Evaluation

339 held-out records (113 dictations × 3 levels), run through Taurscribe's own inference code (llama.cpp, CPU, F16), against FlowScribe v2 (Qwen2.5-0.5B, Q4_K_M):

| | Exact match | Word error rate | Outputs with made-up words | Latency p50 (CPU) |
|---|---|---|---|---|
| FlowScribe v2 | 27.1% | 18.5% | 16.2% | 287 ms |
| **FlowScribe v3** | **41.3%** | **7.2%** | **4.4%** | 654 ms |
| v3, `clean` level | 46.0% | 7.2% | 0.9% | 642 ms |

v3 numbers include Taurscribe's two output guards (below). "Made-up words" counts outputs containing any word found in neither the input nor the reference; it includes some formatting disagreements, so it overstates real errors.

## Known limitations

- **Numbers.** About 1–2% of outputs change a spoken amount (e.g. "twelve hundred fifty dollars" → "$1,500", "twenty five percent" → "20%"). Taurscribe rejects any output containing a number that can't be traced to the transcript and pastes the transcript instead. Use the same safeguard if you deploy this model elsewhere.
- **Repetition loops.** Rarely (≈2%) greedy decoding falls into a loop ("I, I, I, …"). Taurscribe stops generation on repeated n-grams and falls back to the transcript.
- **English only**, trained on synthetic data; terminal commands and code identifiers are the weakest categories.
- Small typos can slip through (e.g. "lets" → "Let").

## Training

- **Base:** Qwen/Qwen3.5-0.8B (instruct), LoRA rank 32 on all layers (21.6M trainable parameters), merged into the base weights and exported at F16. No quantization.
- **Data:** 10,662 records from 3,554 synthetic dictations. Teachers: DeepSeek V4.1 Flash and GPT-6 Luna, each writing a raw transcript, a recognizer-style transcript and three targets (verbatim / clean / formatted) from randomized specs covering 10 apps, 15 English varieties, 35 roles and 12 speech phenomena. Every item passed automatic checks and an LLM judge (GPT-6 Luna); items with dates, times or amounts left as words were removed. Generation cost about $1.70.
- **Schedule:** ~1.65 epochs on an Apple M4 (16 GB) with MLX: AdamW, peak LR 1e-4, cosine decay, effective batch 16, prompt tokens masked. Final validation loss 0.123.

## License

Apache 2.0, same as the base model.
