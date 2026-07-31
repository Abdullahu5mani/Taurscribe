"""MLX/Metal implementation of IBM Granite Speech 4.1 2B NAR.

Ported from the upstream HF `modeling_granite_speech_nar.py` reference.

Design notes that differ from the ONNX bundle this replaces:

* Fully dynamic sequence lengths. The ONNX export baked a fixed 800-frame
  bucket, so a 4-second utterance paid for 16 seconds of encoder compute and
  anything past 16 seconds was silently truncated. MLX shapes the graph per
  call, so cost tracks the real audio length.
* Dense fp16 rather than INT4 weight-only quantization. This model is
  non-autoregressive: the editor sees the whole sequence in one pass, so it is
  compute-bound, not memory-bound. INT4 weight-only quant buys nothing here and
  costs a dequantize on every matmul.
* BatchNorm is folded into the preceding depthwise conv at load time.
* The 100352-wide CTC head runs only over valid pooled positions.

Batch size is fixed at 1, which is what the desktop dictation path needs and
removes all padding and masking from the hot loop.
"""

from __future__ import annotations

import json
import math
from pathlib import Path
from typing import Any

import mlx.core as mx
import mlx.nn as nn

MODEL_ID = "ibm-granite/granite-speech-4.1-2b-nar"


# --------------------------------------------------------------------------
# Conformer encoder
# --------------------------------------------------------------------------


class ConformerFeedForward(nn.Module):
    def __init__(self, dim: int, mult: int):
        super().__init__()
        self.pre_norm = nn.LayerNorm(dim)
        self.up_proj = nn.Linear(dim, dim * mult)
        self.down_proj = nn.Linear(dim * mult, dim)

    def __call__(self, x: mx.array) -> mx.array:
        return self.down_proj(nn.silu(self.up_proj(self.pre_norm(x))))


class ConformerAttention(nn.Module):
    """Block-local attention with Shaw-style relative position embeddings."""

    def __init__(self, dim: int, num_heads: int, dim_head: int, context_size: int, max_pos_emb: int):
        super().__init__()
        inner = num_heads * dim_head
        self.num_heads = num_heads
        self.dim_head = dim_head
        self.context_size = context_size
        self.max_pos_emb = max_pos_emb
        self.scale = dim_head**-0.5
        self.pre_norm = nn.LayerNorm(dim)
        self.to_q = nn.Linear(dim, inner, bias=False)
        self.to_kv = nn.Linear(dim, inner * 2, bias=False)
        self.to_out = nn.Linear(inner, dim)
        self.rel_pos_emb = nn.Embedding(2 * max_pos_emb + 1, dim_head)

    def __call__(self, x: mx.array, attention_dists: mx.array) -> mx.array:
        x = self.pre_norm(x)
        bsz, seq_len, _ = x.shape
        ctx = self.context_size

        num_blocks = math.ceil(seq_len / ctx)
        remainder = seq_len % ctx
        if remainder > 0:
            x = mx.pad(x, [(0, 0), (0, ctx - remainder), (0, 0)])
        padded_len = x.shape[1]

        q = self.to_q(x)
        kv = self.to_kv(x)
        k, v = mx.split(kv, 2, axis=-1)

        def blocks(t: mx.array) -> mx.array:
            # (B, T, inner) -> (B*nblocks, heads, ctx, dim_head).
            # Blocks fold into the batch axis because MLX's fused attention
            # kernel takes rank-4 inputs only.
            t = t.reshape(bsz * num_blocks, ctx, self.num_heads, self.dim_head)
            return t.transpose(0, 2, 1, 3)

        q, k, v = blocks(q), blocks(k), blocks(v)

        # Shaw relative-position bias, supplied as an additive attention mask.
        # (ctx, ctx, dim_head) against (BM, H, ctx, dim_head) -> (BM, H, ctx, ctx)
        rel = self.rel_pos_emb(attention_dists)
        pos_attn = mx.einsum("nhcd,crd->nhcr", q, rel) * self.scale

        if remainder > 0:
            # Positions past the true end of the final block must not attend.
            keep = mx.arange(ctx) < remainder
            valid = (keep[:, None] & keep[None, :]).astype(pos_attn.dtype)
            neg = mx.finfo(pos_attn.dtype).min
            pos_attn = pos_attn.reshape(bsz, num_blocks, self.num_heads, ctx, ctx)
            last = pos_attn[:, -1] * valid + (1.0 - valid) * neg
            pos_attn = mx.concatenate([pos_attn[:, :-1], last[:, None]], axis=1)
            pos_attn = pos_attn.reshape(bsz * num_blocks, self.num_heads, ctx, ctx)

        out = mx.fast.scaled_dot_product_attention(q, k, v, scale=self.scale, mask=pos_attn)
        out = out.transpose(0, 2, 1, 3).reshape(bsz, padded_len, -1)
        return self.to_out(out[:, :seq_len, :])


class ConformerConvModule(nn.Module):
    """Conv module. BatchNorm is folded into depth_conv at load time."""

    def __init__(self, dim: int, expansion: int, kernel_size: int):
        super().__init__()
        inner = dim * expansion
        self.norm = nn.LayerNorm(dim)
        # 1x1 convs are pointwise; in MLX's NLC layout they are plain Linears.
        self.up_conv = nn.Linear(dim, inner * 2)
        self.depth_conv = nn.Conv1d(
            inner, inner, kernel_size, padding=kernel_size // 2, groups=inner, bias=True
        )
        self.down_conv = nn.Linear(inner, dim)

    def __call__(self, x: mx.array) -> mx.array:
        x = self.up_conv(self.norm(x))
        a, b = mx.split(x, 2, axis=-1)
        x = a * mx.sigmoid(b)
        x = nn.silu(self.depth_conv(x))
        return self.down_conv(x)


class ConformerBlock(nn.Module):
    def __init__(self, cfg: dict[str, Any]):
        super().__init__()
        dim = cfg["hidden_dim"]
        self.ff1 = ConformerFeedForward(dim, cfg["feedforward_mult"])
        self.attn = ConformerAttention(
            dim, cfg["num_heads"], cfg["dim_head"], cfg["context_size"], cfg["max_pos_emb"]
        )
        self.conv = ConformerConvModule(dim, cfg["conv_expansion_factor"], cfg["conv_kernel_size"])
        self.ff2 = ConformerFeedForward(dim, cfg["feedforward_mult"])
        self.post_norm = nn.LayerNorm(dim)

    def __call__(self, x: mx.array, attention_dists: mx.array) -> mx.array:
        x = 0.5 * self.ff1(x) + x
        x = self.attn(x, attention_dists) + x
        x = self.conv(x) + x
        x = 0.5 * self.ff2(x) + x
        return self.post_norm(x)


class CTCEncoder(nn.Module):
    def __init__(self, cfg: dict[str, Any]):
        super().__init__()
        self.cfg = cfg
        dim = cfg["hidden_dim"]
        self.input_linear = nn.Linear(cfg["input_dim"], dim)
        self.layers = [ConformerBlock(cfg) for _ in range(cfg["num_layers"])]
        self.out = nn.Linear(dim, cfg["output_dim"])
        self.out_mid = nn.Linear(cfg["output_dim"], dim)
        self.out_bpe = nn.Linear(dim, cfg["bpe_output_dim"])

        ctx = cfg["context_size"]
        seq = mx.arange(ctx)
        dists = mx.clip(seq[:, None] - seq[None, :], -ctx, ctx) + cfg["max_pos_emb"]
        self._attention_dists = dists.astype(mx.int32)

    def __call__(
        self,
        features: mx.array,
        layer_indices: list[int],
        capture: dict[str, mx.array] | None = None,
    ) -> tuple[mx.array, mx.array]:
        """Returns (bpe_logits over valid pooled positions, concatenated multilayer features).

        `capture` collects raw stage outputs for numerical parity checks. Note the
        layer-8 entry is recorded before self-conditioning is added, matching where
        an HF forward hook on the layer module fires.
        """
        x = self.input_linear(features)
        if capture is not None:
            capture["enc_input_linear"] = x
        dists = self._attention_dists
        selected: dict[int, mx.array] = {}
        blank_probs = None
        num_layers = len(self.layers)

        # Upstream indexes a hidden-state tuple whose entry 0 is the pre-layer
        # input, so entry i is the output of layer i and -1 is the last layer.
        def resolve(i: int) -> int:
            return i if i >= 0 else num_layers + 1 + i

        wanted = {resolve(i) for i in layer_indices}

        for idx, layer in enumerate(self.layers, start=1):
            x = layer(x, dists)
            if capture is not None and idx in wanted:
                capture[f"enc_layer_{idx}"] = x

            if idx == self.cfg["self_conditioning_layer"]:
                mid_logits = self.out(x)
                if capture is not None:
                    capture["enc_out_mid_logits"] = mid_logits
                mid_probs = mx.softmax(mid_logits.astype(mx.float32), axis=-1)
                blank_probs = mid_probs[:, :, 0]
                x = x + self.out_mid(mid_probs.astype(x.dtype))

            # `all_hidden_states` upstream is prepended with the pre-layer input,
            # so upstream index i corresponds to the output of layer i here.
            if idx in wanted:
                selected[idx] = x

        multilayer = mx.concatenate([selected[resolve(i)] for i in layer_indices], axis=-1)

        # Posterior-weighted pooling, then the CTC head over valid positions only.
        pooled = _posterior_weighted_pool(x, 1.0 - blank_probs, self.cfg["bpe_pooling_window"])
        bpe_logits = self.out_bpe(pooled)
        return bpe_logits, multilayer


def _posterior_weighted_pool(hidden: mx.array, importance: mx.array, window: int) -> mx.array:
    bsz, seq_len, dim = hidden.shape
    pad = (window - seq_len % window) % window
    if pad:
        hidden = mx.pad(hidden, [(0, 0), (0, pad), (0, 0)])
        importance = mx.pad(importance, [(0, 0), (0, pad)])
    num_windows = hidden.shape[1] // window
    hidden = hidden.astype(mx.float32).reshape(bsz, num_windows, window, dim)
    importance = importance.astype(mx.float32).reshape(bsz, num_windows, window)
    weights = importance / (importance.sum(axis=-1, keepdims=True) + 1e-8)
    return (hidden * weights[..., None]).sum(axis=2)


# --------------------------------------------------------------------------
# Windowed Q-Former projector
# --------------------------------------------------------------------------


class QFormerLayer(nn.Module):
    def __init__(self, cfg: dict[str, Any]):
        super().__init__()
        dim = cfg["hidden_size"]
        self.num_heads = cfg["num_heads"]
        self.head_dim = dim // cfg["num_heads"]
        eps = cfg["layernorm_eps"]
        bias = cfg["attn_bias"]
        mlp_hidden = int(dim * cfg["mlp_ratio"])

        self.attn_norm = nn.LayerNorm(dim, eps=eps)
        self.q_proj = nn.Linear(dim, dim, bias=bias)
        self.k_proj = nn.Linear(dim, dim, bias=bias)
        self.v_proj = nn.Linear(dim, dim, bias=bias)
        self.o_proj = nn.Linear(dim, dim, bias=bias)
        self.mlp_norm = nn.LayerNorm(dim, eps=eps)
        self.fc1 = nn.Linear(dim, mlp_hidden, bias=cfg["mlp_bias"])
        self.fc2 = nn.Linear(mlp_hidden, dim, bias=cfg["mlp_bias"])

    def __call__(self, x: mx.array, enc: mx.array) -> mx.array:
        h = self.attn_norm(x)
        bsz, qlen, dim = h.shape
        enc_len = enc.shape[1]

        def heads(t: mx.array, length: int) -> mx.array:
            return t.reshape(bsz, length, self.num_heads, self.head_dim).transpose(0, 2, 1, 3)

        q = heads(self.q_proj(h), qlen)
        k = heads(self.k_proj(enc), enc_len)
        v = heads(self.v_proj(enc), enc_len)

        attn = mx.fast.scaled_dot_product_attention(q, k, v, scale=self.head_dim**-0.5)
        attn = attn.transpose(0, 2, 1, 3).reshape(bsz, qlen, dim)
        x = x + self.o_proj(attn)
        return x + self.fc2(nn.silu(self.fc1(self.mlp_norm(x))))


class Projector(nn.Module):
    def __init__(self, cfg: dict[str, Any]):
        super().__init__()
        self.cfg = cfg
        eps = cfg["layernorm_eps"]
        self.layer_norms = [
            nn.LayerNorm(cfg["encoder_dim"], eps=eps) for _ in range(cfg["num_encoder_layers"])
        ]
        self.layer_projector = nn.Linear(
            cfg["encoder_dim"] * cfg["num_encoder_layers"], cfg["hidden_size"]
        )
        self.layers = [QFormerLayer(cfg) for _ in range(cfg["num_layers"])]
        self.out_norm = nn.LayerNorm(cfg["hidden_size"], eps=eps)
        self.out_linear = nn.Linear(cfg["hidden_size"], cfg["llm_dim"])
        self.query = mx.zeros((1, cfg["block_size"] // cfg["downsample_rate"], cfg["hidden_size"]))
        self.window_positions = mx.zeros((1, cfg["block_size"], cfg["hidden_size"]))

    def __call__(self, multilayer: mx.array) -> mx.array:
        cfg = self.cfg
        bsz, seq_len, _ = multilayer.shape
        x = multilayer.reshape(bsz, seq_len, cfg["num_encoder_layers"], cfg["encoder_dim"])
        x = mx.concatenate([norm(x[:, :, i]) for i, norm in enumerate(self.layer_norms)], axis=-1)
        x = nn.gelu(self.layer_projector(x))

        block = cfg["block_size"]
        nblocks = seq_len // block
        rest = seq_len % block
        if rest:
            x = mx.pad(x, [(0, 0), (0, block - rest), (0, 0)])
            nblocks += 1

        hidden = x.reshape(bsz * nblocks, block, cfg["hidden_size"])
        qlen = self.query.shape[1]
        mean_pool = hidden.reshape(
            bsz * nblocks, qlen, cfg["downsample_rate"], cfg["hidden_size"]
        ).mean(axis=-2)

        h = self.query + mean_pool
        enc = hidden + self.window_positions
        for layer in self.layers:
            h = layer(h, enc)

        h = h.reshape(bsz, nblocks * qlen, -1)
        return self.out_linear(self.out_norm(h))


# --------------------------------------------------------------------------
# Bidirectional Granite editor LM
# --------------------------------------------------------------------------


def _rope_theta(cfg: dict[str, Any]) -> float:
    """RoPE base, which moved under `rope_parameters` in newer Granite configs."""
    params = cfg.get("rope_parameters")
    if isinstance(params, dict) and "rope_theta" in params:
        return float(params["rope_theta"])
    return float(cfg.get("rope_theta", 10000.0))


class EditorLayer(nn.Module):
    def __init__(self, cfg: dict[str, Any]):
        super().__init__()
        dim = cfg["hidden_size"]
        self.num_heads = cfg["num_attention_heads"]
        self.num_kv_heads = cfg["num_key_value_heads"]
        self.head_dim = cfg.get("head_dim") or dim // self.num_heads
        self.scale = cfg["attention_multiplier"]
        self.residual_multiplier = cfg["residual_multiplier"]
        self.rope_theta = _rope_theta(cfg)
        bias = cfg["attention_bias"]

        self.input_layernorm_w = mx.ones((dim,))
        self.post_attention_layernorm_w = mx.ones((dim,))
        self.eps = cfg["rms_norm_eps"]

        self.q_proj = nn.Linear(dim, self.num_heads * self.head_dim, bias=bias)
        self.k_proj = nn.Linear(dim, self.num_kv_heads * self.head_dim, bias=bias)
        self.v_proj = nn.Linear(dim, self.num_kv_heads * self.head_dim, bias=bias)
        self.o_proj = nn.Linear(self.num_heads * self.head_dim, dim, bias=bias)

        inter = cfg["intermediate_size"]
        self.gate_proj = nn.Linear(dim, inter, bias=cfg["mlp_bias"])
        self.up_proj = nn.Linear(dim, inter, bias=cfg["mlp_bias"])
        self.down_proj = nn.Linear(inter, dim, bias=cfg["mlp_bias"])

    def __call__(self, x: mx.array) -> mx.array:
        bsz, seq_len, _ = x.shape
        h = mx.fast.rms_norm(x, self.input_layernorm_w, self.eps)

        q = self.q_proj(h).reshape(bsz, seq_len, self.num_heads, self.head_dim).transpose(0, 2, 1, 3)
        k = self.k_proj(h).reshape(bsz, seq_len, self.num_kv_heads, self.head_dim).transpose(0, 2, 1, 3)
        v = self.v_proj(h).reshape(bsz, seq_len, self.num_kv_heads, self.head_dim).transpose(0, 2, 1, 3)

        q = mx.fast.rope(q, self.head_dim, traditional=False, base=self.rope_theta, scale=1.0, offset=0)
        k = mx.fast.rope(k, self.head_dim, traditional=False, base=self.rope_theta, scale=1.0, offset=0)

        # Non-causal: the editor sees the whole sequence at once, so no mask.
        attn = mx.fast.scaled_dot_product_attention(q, k, v, scale=self.scale)
        attn = attn.transpose(0, 2, 1, 3).reshape(bsz, seq_len, -1)
        x = x + self.o_proj(attn) * self.residual_multiplier

        h = mx.fast.rms_norm(x, self.post_attention_layernorm_w, self.eps)
        h = self.down_proj(nn.silu(self.gate_proj(h)) * self.up_proj(h))
        return x + h * self.residual_multiplier


class Editor(nn.Module):
    def __init__(self, cfg: dict[str, Any]):
        super().__init__()
        self.cfg = cfg
        self.embed_tokens = nn.Embedding(cfg["vocab_size"], cfg["hidden_size"])
        self.layers = [EditorLayer(cfg) for _ in range(cfg["num_hidden_layers"])]
        self.norm_w = mx.ones((cfg["hidden_size"],))
        self.eps = cfg["rms_norm_eps"]
        # tie_word_embeddings is true for this checkpoint: there is no separate
        # lm_head tensor, so the output projection reuses the embedding matrix.
        self.tied_lm_head = cfg.get("tie_word_embeddings", True)
        if not self.tied_lm_head:
            self.lm_head = nn.Linear(cfg["hidden_size"], cfg["vocab_size"], bias=False)

    def _project_to_vocab(self, x: mx.array) -> mx.array:
        if not self.tied_lm_head:
            return self.lm_head(x)
        # After nn.quantize the embedding holds packed weights, so the tied
        # output projection has to go through its own dequantizing matmul.
        if isinstance(self.embed_tokens, nn.QuantizedEmbedding):
            return self.embed_tokens.as_linear(x)
        return x @ self.embed_tokens.weight.T

    def __call__(
        self,
        embeds: mx.array,
        capture: dict[str, mx.array] | None = None,
        logits_from: int = 0,
    ) -> mx.array:
        """`logits_from` drops leading positions before the vocabulary projection.

        Only the text tail of the sequence is ever read, and the projection is
        100352-wide, so projecting the audio prefix produces a large tensor that
        is immediately discarded. Skipping it is exact.
        """
        x = embeds * self.cfg["embedding_multiplier"]
        for layer in self.layers:
            x = layer(x)
        x = mx.fast.rms_norm(x, self.norm_w, self.eps)
        if capture is not None:
            # Parity runs need the full sequence, pre-scaling, to line up with
            # an HF forward hook on lm_head.
            capture["editor_logits"] = self._project_to_vocab(x)
        if logits_from:
            x = x[:, logits_from:]
        return self._project_to_vocab(x) / self.cfg["logits_scaling"]


# --------------------------------------------------------------------------
# Full ASR pipeline
# --------------------------------------------------------------------------


class GraniteSpeechNarMLX(nn.Module):
    def __init__(self, config: dict[str, Any]):
        super().__init__()
        self.config = config
        self.encoder = CTCEncoder(config["encoder_config"])
        self.projector = Projector(config["projector_config"])
        self.editor = Editor(config["text_config"])
        self.blank_token_id = config["blank_token_id"]

    def encode(
        self, features: mx.array, valid_frames: int, capture: dict[str, mx.array] | None = None
    ) -> tuple[mx.array, mx.array]:
        bpe_logits, multilayer = self.encoder(
            features, self.config["encoder_layer_indices"], capture
        )
        pooled_len = -(-valid_frames // self.config["encoder_config"]["bpe_pooling_window"])
        return bpe_logits[:, :pooled_len], multilayer

    def transcribe_ids(
        self, features: mx.array, capture: dict[str, mx.array] | None = None
    ) -> tuple[list[int], list[int]]:
        """features: (1, T, 160) log-mel. Returns (final_ids, ctc_ids)."""
        valid_frames = features.shape[1]

        bpe_logits, multilayer = self.encode(features, valid_frames, capture)
        if capture is not None:
            capture["enc_bpe_logits"] = bpe_logits[0]
        ctc_ids = _ctc_greedy(mx.argmax(bpe_logits[0], axis=-1), self.blank_token_id)

        audio_embeds = self.projector(multilayer)
        if capture is not None:
            capture["audio_embeds"] = audio_embeds
        if self.config["scale_projected_embeddings"]:
            audio_embeds = audio_embeds / self.config["text_config"]["embedding_multiplier"]

        audio_len = valid_frames // self.config["projector_config"]["downsample_rate"]
        audio_embeds = audio_embeds[:, :audio_len]

        slots = _add_insertion_slots(
            ctc_ids, self.blank_token_id, self.config["min_edit_sequence_length"]
        )
        text_embeds = self.editor.embed_tokens(mx.array(slots))[None]

        embeds = mx.concatenate([audio_embeds, text_embeds], axis=1)
        logits = self.editor(embeds, capture, logits_from=audio_len)

        final_ids = _ctc_greedy(mx.argmax(logits[0], axis=-1), self.blank_token_id)
        return final_ids, ctc_ids


def _ctc_greedy(ids: mx.array, blank_id: int) -> list[int]:
    """Collapse repeats then drop blanks."""
    values = ids.tolist()
    out: list[int] = []
    previous = None
    for value in values:
        if value != previous:
            if value != blank_id:
                out.append(value)
            previous = value
    return out


def _add_insertion_slots(token_ids: list[int], blank_id: int, min_length: int) -> list[int]:
    total = max(2 * len(token_ids) + 1, min_length)
    out = [blank_id] * total
    for i, token in enumerate(token_ids):
        out[2 * i + 1] = token
    return out


# --------------------------------------------------------------------------
# Weight loading
# --------------------------------------------------------------------------


def _fold_batch_norm(weights: dict[str, mx.array], prefix: str) -> tuple[mx.array, mx.array]:
    """Fold BatchNorm1d into the preceding bias-free depthwise conv.

    y = gamma * (conv(x) - mean) / sqrt(var + eps) + beta
      = conv_scaled(x) + (beta - gamma * mean / sqrt(var + eps))
    """
    eps = 1e-5
    gamma = weights[f"{prefix}.batch_norm.weight"].astype(mx.float32)
    beta = weights[f"{prefix}.batch_norm.bias"].astype(mx.float32)
    mean = weights[f"{prefix}.batch_norm.running_mean"].astype(mx.float32)
    var = weights[f"{prefix}.batch_norm.running_var"].astype(mx.float32)
    conv = weights[f"{prefix}.depth_conv.conv.weight"].astype(mx.float32)

    inv_std = gamma * mx.rsqrt(var + eps)
    # PyTorch depthwise weight is (channels, 1, kernel); MLX Conv1d wants (channels, kernel, 1).
    folded = (conv * inv_std[:, None, None]).transpose(0, 2, 1)
    return folded, beta - mean * inv_std


def quantize_model(model: GraniteSpeechNarMLX, bits: int, group_size: int = 64) -> None:
    """Quantize the large projections in place.

    This is a footprint play, not a speed one: the NAR editor sees the whole
    sequence at once, so inference is compute-bound and dequantization is not
    free. Depthwise convs and anything not a Linear/Embedding are left alone,
    as are layers whose dimensions are not a multiple of the group size.
    """

    def should_quantize(_path: str, module: nn.Module) -> bool:
        if not isinstance(module, (nn.Linear, nn.Embedding)):
            return False
        weight = module.weight
        return weight.ndim == 2 and weight.shape[-1] % group_size == 0

    nn.quantize(model, group_size=group_size, bits=bits, class_predicate=should_quantize)


def load_model(
    model_path: str | Path | None = None,
    dtype: mx.Dtype = mx.float16,
    bits: int | None = None,
    group_size: int = 64,
) -> tuple[GraniteSpeechNarMLX, dict[str, Any]]:
    from huggingface_hub import snapshot_download

    path = Path(model_path) if model_path else Path(snapshot_download(MODEL_ID))
    config = json.loads((path / "config.json").read_text())
    weights = mx.load(str(path / "model.safetensors"))

    model = GraniteSpeechNarMLX(config)
    flat: dict[str, mx.array] = {}

    enc_cfg = config["encoder_config"]
    for name in ("input_linear", "out", "out_mid", "out_bpe"):
        flat[f"encoder.{name}.weight"] = weights[f"encoder.{name}.weight"]
        flat[f"encoder.{name}.bias"] = weights[f"encoder.{name}.bias"]

    for i in range(enc_cfg["num_layers"]):
        src, dst = f"encoder.layers.{i}", f"encoder.layers.{i}"
        for ff in ("ff1", "ff2"):
            for part in ("pre_norm", "up_proj", "down_proj"):
                flat[f"{dst}.{ff}.{part}.weight"] = weights[f"{src}.{ff}.{part}.weight"]
                flat[f"{dst}.{ff}.{part}.bias"] = weights[f"{src}.{ff}.{part}.bias"]

        flat[f"{dst}.attn.pre_norm.weight"] = weights[f"{src}.attn.pre_norm.weight"]
        flat[f"{dst}.attn.pre_norm.bias"] = weights[f"{src}.attn.pre_norm.bias"]
        flat[f"{dst}.attn.to_q.weight"] = weights[f"{src}.attn.to_q.weight"]
        flat[f"{dst}.attn.to_kv.weight"] = weights[f"{src}.attn.to_kv.weight"]
        flat[f"{dst}.attn.to_out.weight"] = weights[f"{src}.attn.to_out.weight"]
        flat[f"{dst}.attn.to_out.bias"] = weights[f"{src}.attn.to_out.bias"]
        flat[f"{dst}.attn.rel_pos_emb.weight"] = weights[f"{src}.attn.rel_pos_emb.weight"]

        flat[f"{dst}.conv.norm.weight"] = weights[f"{src}.conv.norm.weight"]
        flat[f"{dst}.conv.norm.bias"] = weights[f"{src}.conv.norm.bias"]
        # 1x1 convs -> Linear: drop the trailing kernel axis.
        flat[f"{dst}.conv.up_conv.weight"] = weights[f"{src}.conv.up_conv.weight"].squeeze(-1)
        flat[f"{dst}.conv.up_conv.bias"] = weights[f"{src}.conv.up_conv.bias"]
        flat[f"{dst}.conv.down_conv.weight"] = weights[f"{src}.conv.down_conv.weight"].squeeze(-1)
        flat[f"{dst}.conv.down_conv.bias"] = weights[f"{src}.conv.down_conv.bias"]
        conv_w, conv_b = _fold_batch_norm(weights, src + ".conv")
        flat[f"{dst}.conv.depth_conv.weight"] = conv_w
        flat[f"{dst}.conv.depth_conv.bias"] = conv_b

        flat[f"{dst}.post_norm.weight"] = weights[f"{src}.post_norm.weight"]
        flat[f"{dst}.post_norm.bias"] = weights[f"{src}.post_norm.bias"]

    proj_cfg = config["projector_config"]
    for i in range(proj_cfg["num_encoder_layers"]):
        flat[f"projector.layer_norms.{i}.weight"] = weights[f"projector.layer_norms.{i}.weight"]
        flat[f"projector.layer_norms.{i}.bias"] = weights[f"projector.layer_norms.{i}.bias"]
    for name in ("layer_projector", "out_norm", "out_linear"):
        flat[f"projector.{name}.weight"] = weights[f"projector.{name}.weight"]
        flat[f"projector.{name}.bias"] = weights[f"projector.{name}.bias"]
    flat["projector.query"] = weights["projector.query"]
    flat["projector.window_positions"] = weights["projector.window_positions"]

    for i in range(proj_cfg["num_layers"]):
        src = f"projector.qformer.layers.{i}"
        dst = f"projector.layers.{i}"
        for part in ("attn_norm", "mlp_norm"):
            flat[f"{dst}.{part}.weight"] = weights[f"{src}.{part}.weight"]
            flat[f"{dst}.{part}.bias"] = weights[f"{src}.{part}.bias"]
        for part in ("q_proj", "k_proj", "v_proj", "o_proj"):
            flat[f"{dst}.{part}.weight"] = weights[f"{src}.cross_attention.{part}.weight"]
            if f"{src}.cross_attention.{part}.bias" in weights:
                flat[f"{dst}.{part}.bias"] = weights[f"{src}.cross_attention.{part}.bias"]
        for part in ("fc1", "fc2"):
            flat[f"{dst}.{part}.weight"] = weights[f"{src}.mlp.{part}.weight"]
            if f"{src}.mlp.{part}.bias" in weights:
                flat[f"{dst}.{part}.bias"] = weights[f"{src}.mlp.{part}.bias"]

    txt_cfg = config["text_config"]
    flat["editor.embed_tokens.weight"] = weights["language_model.model.embed_tokens.weight"]
    if "language_model.lm_head.weight" in weights:
        flat["editor.lm_head.weight"] = weights["language_model.lm_head.weight"]
    flat["editor.norm_w"] = weights["language_model.model.norm.weight"]
    for i in range(txt_cfg["num_hidden_layers"]):
        src = f"language_model.model.layers.{i}"
        dst = f"editor.layers.{i}"
        flat[f"{dst}.input_layernorm_w"] = weights[f"{src}.input_layernorm.weight"]
        flat[f"{dst}.post_attention_layernorm_w"] = weights[f"{src}.post_attention_layernorm.weight"]
        for part in ("q_proj", "k_proj", "v_proj", "o_proj"):
            flat[f"{dst}.{part}.weight"] = weights[f"{src}.self_attn.{part}.weight"]
        for part in ("gate_proj", "up_proj", "down_proj"):
            flat[f"{dst}.{part}.weight"] = weights[f"{src}.mlp.{part}.weight"]

    flat = {k: v.astype(dtype) for k, v in flat.items()}
    model.load_weights(list(flat.items()))
    if bits is not None:
        quantize_model(model, bits=bits, group_size=group_size)
    mx.eval(model.parameters())
    return model, config
