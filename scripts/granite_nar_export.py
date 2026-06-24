"""Export scaffold for IBM Granite Speech 4.1 2B NAR.

This model is not a simple autoregressive decoder. Its PyTorch `transcribe()`
path performs dynamic CTC collapse, variable-length slot insertion, and list
splitting in Python. A production ONNX bundle should therefore be split into
static subgraphs and driven by Rust glue:

  1. encoder.onnx       input_features + attention_mask -> BPE logits + selected hidden states
  2. projector.onnx     selected hidden states -> audio embeddings
  3. embed_tokens.onnx  text token ids -> text embeddings
  4. editor.onnx        concat(audio embeddings, text embeddings) -> edit logits

Fallback is not encoded in ONNX. Taurscribe/ORT should create sessions with
CUDA, then DirectML, then CPU for the same exported ONNX files.

This script has deliberately small commands:

  python scripts/granite_nar_export.py inspect --model-dir <local-model-dir>
  python scripts/granite_nar_export.py smoke --model-dir <local-model-dir>
  python scripts/granite_nar_export.py export --model-dir <local-model-dir> --out-dir <onnx-out-dir>

The export command requires the full Hugging Face weights and compatible
PyTorch/Transformers installs. For this model IBM recommends torch>=2.9.1 and
transformers>=5.5.3.
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
from pathlib import Path

EXPORT_FRAMES = 800


def _prepend_model_dir(model_dir: Path) -> None:
    sys.path.insert(0, str(model_dir.resolve()))


def inspect_model(model_dir: Path) -> None:
    config_path = model_dir / "config.json"
    if not config_path.exists():
        raise SystemExit(f"missing config.json in {model_dir}")
    config = json.loads(config_path.read_text(encoding="utf-8"))
    files = sorted(p.name for p in model_dir.iterdir())
    print("model_dir:", model_dir)
    print("has_weights:", (model_dir / "model.safetensors").exists())
    print("model_type:", config.get("model_type"))
    print("architecture:", config.get("architectures"))
    print("encoder_layers:", config["encoder_config"]["num_layers"])
    print("editor_layers:", config["text_config"]["num_hidden_layers"])
    print("vocab_size:", config["text_config"]["vocab_size"])
    print("files:")
    for name in files:
        path = model_dir / name
        print(f"  {name}\t{path.stat().st_size if path.is_file() else 0}")


def load_model_and_processor(model_dir: Path, device: str = "cuda", dtype: str = "bfloat16"):
    _prepend_model_dir(model_dir)
    import torch
    from transformers import AutoModel, AutoProcessor

    torch_dtype = getattr(torch, dtype)
    model = AutoModel.from_pretrained(
        str(model_dir),
        trust_remote_code=True,
        attn_implementation="eager",
        torch_dtype=torch_dtype,
    ).to(device)
    model.eval()
    processor = AutoProcessor.from_pretrained(str(model_dir), trust_remote_code=True)
    return model, processor


def smoke(model_dir: Path, audio_path: Path | None, device: str) -> None:
    import numpy as np
    import soundfile as sf
    import torch

    if audio_path is None:
        audio_path = model_dir / "10226_10111_000000.wav"
    if not audio_path.exists():
        raise SystemExit(f"missing audio file: {audio_path}")

    model, processor = load_model_and_processor(model_dir, device=device)
    waveform_np, sample_rate = sf.read(str(audio_path), dtype="float32", always_2d=True)
    waveform_np = waveform_np.mean(axis=1)
    if sample_rate != 16000:
        raise SystemExit(f"expected 16 kHz WAV for smoke test, got {sample_rate}")
    waveform = torch.from_numpy(np.ascontiguousarray(waveform_np))

    with torch.inference_mode():
        inputs = processor([waveform], device=device)
        output = model.transcribe(**inputs)
        text = processor.batch_decode(output.preds)[0]
    print(text)


def export(model_dir: Path, out_dir: Path, device: str) -> None:
    import torch
    import torch.nn.functional as F

    # Export in fp32 first. It is larger than bf16/fp16, but it is the most
    # portable dtype for validating CUDA -> DirectML -> CPU fallback sessions.
    model, _processor = load_model_and_processor(model_dir, device=device, dtype="float32")
    out_dir.mkdir(parents=True, exist_ok=True)

    class EncoderExportWrapper(torch.nn.Module):
        def __init__(self, wrapped):
            super().__init__()
            self.wrapped = wrapped

        def forward(self, input_features, attention_mask):
            encoder = self.wrapped.encoder
            hidden_states = encoder.input_linear(input_features.to(encoder.dtype))
            all_hidden_states = [hidden_states]
            blank_probs = None

            context_size = encoder.config.context_size
            seq = torch.arange(context_size, device=hidden_states.device)
            relpos_dist = seq.view(-1, 1) - seq.view(1, -1)
            attention_dists = torch.clamp(relpos_dist, -context_size, context_size) + encoder.config.max_pos_emb

            for layer_idx, layer in enumerate(encoder.layers, start=1):
                hidden_states = layer(hidden_states, attention_dists=attention_dists)

                if layer_idx == encoder.config.self_conditioning_layer:
                    mid_logits = encoder.out(encoder.dropout(hidden_states))
                    mid_probs = torch.softmax(mid_logits.float(), dim=-1)
                    blank_probs = mid_probs[:, :, 0]
                    hidden_states = hidden_states + encoder.out_mid(mid_probs.to(hidden_states.dtype))

                all_hidden_states.append(hidden_states)

            hidden_states = encoder.dropout(hidden_states)

            pool_window = encoder.config.bpe_pooling_window
            importance = 1.0 - blank_probs
            pad_len = (pool_window - hidden_states.shape[1] % pool_window) % pool_window
            if pad_len > 0:
                hidden_states = F.pad(hidden_states.float(), (0, 0, 0, pad_len))
                importance = F.pad(importance.float(), (0, pad_len))
            else:
                hidden_states = hidden_states.float()
                importance = importance.float()

            batch_size, padded_frames, hidden_dim = hidden_states.shape
            num_windows = padded_frames // pool_window
            pooled_hidden = hidden_states.view(batch_size, num_windows, pool_window, hidden_dim)
            pooled_importance = importance.view(batch_size, num_windows, pool_window)
            weights = pooled_importance / (pooled_importance.sum(dim=-1, keepdim=True) + 1e-8)
            pooled = (pooled_hidden * weights.unsqueeze(-1)).sum(dim=2).to(encoder.dtype)
            bpe_logits = encoder.out_bpe(pooled)

            selected = [all_hidden_states[i] for i in self.wrapped.config.encoder_layer_indices]
            return (bpe_logits, *selected)

    class EditorExportWrapper(torch.nn.Module):
        def __init__(self, wrapped):
            super().__init__()
            self.wrapped = wrapped

        def forward(self, inputs_embeds, position_ids):
            lm = self.wrapped.language_model
            decoder = lm.model
            hidden_states = inputs_embeds * decoder.embedding_multiplier
            position_embeddings = decoder.rotary_emb(hidden_states, position_ids=position_ids)

            for decoder_layer in decoder.layers:
                hidden_states = decoder_layer(
                    hidden_states,
                    attention_mask=None,
                    position_ids=position_ids,
                    position_embeddings=position_embeddings,
                    use_cache=False,
                )

            hidden_states = decoder.norm(hidden_states)
            logits = lm.lm_head(hidden_states)
            return logits / lm.config.logits_scaling

    class EmbedTokensExportWrapper(torch.nn.Module):
        def __init__(self, wrapped):
            super().__init__()
            self.embed = wrapped.language_model.model.embed_tokens

        def forward(self, token_ids):
            return self.embed(token_ids)

    # Granite's conformer encoder and QFormer projector contain Python shape
    # logic (`math.ceil`, `if rest > 0`) that does not produce a genuinely
    # dynamic ONNX graph. Export a fixed app bucket and have host code pad audio
    # features to this length before inference.
    input_features = torch.zeros(1, EXPORT_FRAMES, model.config.encoder_config.input_dim, device=device, dtype=torch.float32)
    attention_mask = torch.ones(1, EXPORT_FRAMES, device=device, dtype=torch.bool)
    concat_hidden = torch.zeros(
        1,
        EXPORT_FRAMES,
        model.config.projector_config.encoder_dim * model.config.projector_config.num_encoder_layers,
        device=device,
        dtype=torch.float32,
    )
    token_ids = torch.zeros(16, device=device, dtype=torch.long)
    inputs_embeds = torch.zeros(1, 64, model.config.text_config.hidden_size, device=device, dtype=torch.float32)
    position_ids = torch.arange(64, device=device, dtype=torch.long).unsqueeze(0)

    torch.onnx.export(
        EncoderExportWrapper(model),
        (input_features, attention_mask),
        str(out_dir / "encoder.onnx"),
        input_names=["input_features", "attention_mask"],
        output_names=["bpe_logits", "hidden_4", "hidden_8", "hidden_12", "hidden_last"],
        opset_version=20,
        dynamo=False,
        external_data=True,
    )

    torch.onnx.export(
        model.projector,
        (concat_hidden,),
        str(out_dir / "projector.onnx"),
        input_names=["multilayer_features"],
        output_names=["audio_embeds"],
        opset_version=20,
        dynamo=False,
        external_data=True,
    )

    torch.onnx.export(
        EmbedTokensExportWrapper(model),
        (token_ids,),
        str(out_dir / "embed_tokens.onnx"),
        input_names=["token_ids"],
        output_names=["text_embeds"],
        dynamic_axes={"token_ids": {0: "tokens"}, "text_embeds": {0: "tokens"}},
        opset_version=20,
        dynamo=False,
        external_data=True,
    )

    torch.onnx.export(
        EditorExportWrapper(model),
        (inputs_embeds, position_ids),
        str(out_dir / "editor.onnx"),
        input_names=["inputs_embeds", "position_ids"],
        output_names=["logits"],
        dynamic_axes={
            "inputs_embeds": {1: "sequence"},
            "position_ids": {1: "sequence"},
            "logits": {1: "sequence"},
        },
        opset_version=20,
        dynamo=False,
        external_data=True,
    )

    metadata_files = [
        "config.json",
        "preprocessor_config.json",
        "processor_config.json",
        "tokenizer_config.json",
        "tokenizer.json",
        "vocab.json",
        "special_tokens_map.json",
        "generation_config.json",
    ]
    for name in metadata_files:
        src = model_dir / name
        if src.exists():
            shutil.copy2(src, out_dir / name)

    manifest = {
        "format": "taurscribe-granite-nar-onnx-bundle",
        "format_version": 1,
        "source_model": "ibm-granite/granite-speech-4.1-2b-nar",
        "export_dtype": "float32",
        "fixed_encoder_frames": EXPORT_FRAMES,
        "opset": 20,
        "execution_provider_preference": [
            "CUDAExecutionProvider",
            "DmlExecutionProvider",
            "CPUExecutionProvider",
        ],
        "graphs": {
            "encoder": {
                "file": "encoder.onnx",
                "inputs": ["input_features", "attention_mask"],
                "outputs": ["bpe_logits", "hidden_4", "hidden_8", "hidden_12", "hidden_last"],
                "notes": f"Fixed-shape app bucket: pad input_features to {EXPORT_FRAMES} frames. Trim bpe_logits to ceil(valid_frames / bpe_pooling_window) before CTC collapse.",
            },
            "projector": {
                "file": "projector.onnx",
                "inputs": ["multilayer_features"],
                "outputs": ["audio_embeds"],
            },
            "embed_tokens": {
                "file": "embed_tokens.onnx",
                "inputs": ["token_ids"],
                "outputs": ["text_embeds"],
            },
            "editor": {
                "file": "editor.onnx",
                "inputs": ["inputs_embeds", "position_ids"],
                "outputs": ["logits"],
                "notes": "Non-causal bidirectional editor. Host code should pass one flattened sample sequence at a time.",
            },
        },
        "host_pipeline": [
            f"Extract Granite log-mel features with the copied preprocessor config and pad/truncate to {EXPORT_FRAMES} encoder frames.",
            "Run encoder.onnx.",
            "Trim padded BPE logits using the valid frame count, argmax, unique-consecutive collapse, and remove blank_token_id.",
            "Concatenate hidden_4, hidden_8, hidden_12, and hidden_last, then run projector.onnx.",
            "Trim audio embeddings using floor(valid_frames / projector.downsample_rate).",
            "Insert blank edit slots around the encoder CTC token IDs.",
            "Run embed_tokens.onnx for the slotted token IDs.",
            "Concatenate audio embeddings and text embeddings, create monotonic position_ids, then run editor.onnx.",
            "Select the text segment logits, argmax, unique-consecutive collapse, remove blank_token_id, and decode with tokenizer.json.",
        ],
        "validation": {
            "onnx_checker": "run scripts/granite_nar_export.py validate --model-dir <onnx-bundle>",
            "ort_cpu_session_load": "run scripts/granite_nar_export.py validate --model-dir <onnx-bundle>",
        },
        "notes": [
            "Fallback is controlled by ONNX Runtime session creation in Rust, not encoded inside the ONNX files.",
            "This is a correctness-first fp32 export. A practical shipped bundle should likely add fp16 or quantized variants after transcript parity is verified.",
        ],
    }
    (out_dir / "taurscribe_granite_nar_manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n",
        encoding="utf-8",
    )
    print("wrote ONNX bundle:", out_dir)


def validate_bundle(model_dir: Path) -> None:
    import gc

    import onnx
    import onnxruntime as ort

    graphs = ["encoder.onnx", "projector.onnx", "embed_tokens.onnx", "editor.onnx"]
    session_options = ort.SessionOptions()
    session_options.graph_optimization_level = ort.GraphOptimizationLevel.ORT_DISABLE_ALL

    for name in graphs:
        path = model_dir / name
        if not path.exists():
            raise SystemExit(f"missing {path}")
        print(f"checking {name}...")
        onnx.checker.check_model(str(path))
        session = ort.InferenceSession(
            str(path),
            sess_options=session_options,
            providers=["CPUExecutionProvider"],
        )
        print("  providers:", session.get_providers())
        print("  inputs:", [(i.name, i.shape, i.type) for i in session.get_inputs()])
        print("  outputs:", [(o.name, o.shape, o.type) for o in session.get_outputs()])
        del session
        gc.collect()

    print("bundle validation ok:", model_dir)


def main() -> None:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("inspect")
    p.add_argument("--model-dir", type=Path, required=True)

    p = sub.add_parser("smoke")
    p.add_argument("--model-dir", type=Path, required=True)
    p.add_argument("--audio", type=Path)
    p.add_argument("--device", default="cuda")

    p = sub.add_parser("export")
    p.add_argument("--model-dir", type=Path, required=True)
    p.add_argument("--out-dir", type=Path, required=True)
    p.add_argument("--device", default="cuda")

    p = sub.add_parser("validate")
    p.add_argument("--model-dir", type=Path, required=True)

    args = parser.parse_args()
    if args.cmd == "inspect":
        inspect_model(args.model_dir)
    elif args.cmd == "smoke":
        smoke(args.model_dir, args.audio, args.device)
    elif args.cmd == "export":
        export(args.model_dir, args.out_dir, args.device)
    elif args.cmd == "validate":
        validate_bundle(args.model_dir)


if __name__ == "__main__":
    main()
