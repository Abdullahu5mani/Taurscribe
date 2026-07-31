"""Capture PyTorch reference activations for the Granite Speech NAR MLX port.

Runs the upstream HF implementation in float32 on CPU and dumps every
stage boundary to an .npz so the MLX port can be checked numerically,
stage by stage, instead of only at the final transcript.

  python scripts/granite_mlx/capture_reference.py --out ref.npz
"""

from __future__ import annotations

import argparse
from pathlib import Path

import numpy as np
import torch

MODEL_ID = "ibm-granite/granite-speech-4.1-2b-nar"


def resolve_audio(model_dir: Path, audio: Path | None) -> Path:
    if audio is not None:
        return audio
    sample = model_dir / "10226_10111_000000.wav"
    if not sample.exists():
        raise SystemExit(f"no audio given and sample missing at {sample}")
    return sample


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, default=Path("granite_nar_reference.npz"))
    parser.add_argument("--audio", type=Path, default=None)
    parser.add_argument("--dtype", default="float32", choices=["float32", "bfloat16"])
    args = parser.parse_args()

    from huggingface_hub import snapshot_download
    from transformers import AutoModel, AutoProcessor

    model_dir = Path(snapshot_download(MODEL_ID))
    audio_path = resolve_audio(model_dir, args.audio)

    dtype = getattr(torch, args.dtype)
    model = AutoModel.from_pretrained(
        MODEL_ID, trust_remote_code=True, dtype=dtype, attn_implementation="eager"
    )
    model.eval()
    processor = AutoProcessor.from_pretrained(MODEL_ID, trust_remote_code=True)

    import soundfile as sf

    wav, sr = sf.read(str(audio_path), dtype="float32", always_2d=True)
    if sr != 16000:
        raise SystemExit(f"expected 16 kHz audio, got {sr}")
    waveform = torch.from_numpy(np.ascontiguousarray(wav.mean(axis=1)))

    captured: dict[str, np.ndarray] = {}

    def grab(name):
        def hook(_module, _inputs, output):
            tensor = output[0] if isinstance(output, tuple) else output
            if torch.is_tensor(tensor):
                captured[name] = tensor.detach().float().cpu().numpy()

        return hook

    enc = model.encoder
    handles = [
        enc.input_linear.register_forward_hook(grab("enc_input_linear")),
        enc.out.register_forward_hook(grab("enc_out_mid_logits")),
        enc.out_bpe.register_forward_hook(grab("enc_bpe_logits")),
        model.projector.register_forward_hook(grab("audio_embeds")),
        model.language_model.lm_head.register_forward_hook(grab("editor_logits")),
    ]
    for idx in (4, 8, 12, 16):
        handles.append(enc.layers[idx - 1].register_forward_hook(grab(f"enc_layer_{idx}")))

    with torch.inference_mode():
        inputs = processor([waveform])
        captured["input_features"] = inputs["input_features"].float().cpu().numpy()
        captured["attention_mask"] = inputs["attention_mask"].cpu().numpy()
        output = model.transcribe(**inputs, output_encoder_logits=True)

    for handle in handles:
        handle.remove()

    captured["final_ids"] = output.preds[0].cpu().numpy()
    if output.encoder_preds is not None:
        captured["ctc_ids"] = output.encoder_preds[0].cpu().numpy()
    text = processor.batch_decode(output.preds)[0]

    np.savez_compressed(args.out, **captured, text=np.array(text))

    print(f"audio      : {audio_path.name} ({waveform.shape[-1] / 16000:.2f}s)")
    print(f"transcript : {text}")
    print(f"saved      : {args.out}")
    for key, value in sorted(captured.items()):
        print(f"  {key:22s} {str(value.shape):22s} {value.dtype}")


if __name__ == "__main__":
    main()
