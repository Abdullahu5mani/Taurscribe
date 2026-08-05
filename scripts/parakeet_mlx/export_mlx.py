"""Export Parakeet Nemotron ONNX models (encoder.onnx + decoder_joint.onnx) to MLX safetensors.

Dequantizes INT4 MatMulNBits tensors to FP16 and exports all weights to
model.safetensors with clean PyTorch/NeMo hierarchical keys.
"""

import argparse
from pathlib import Path
import numpy as np
import onnx
from onnx import numpy_helper
from safetensors.numpy import save_file


def dequantize_matmul_nbits(q_raw: np.ndarray, scales: np.ndarray, zp_raw: np.ndarray, K: int, N: int, block_size: int = 128) -> np.ndarray:
    """Dequantize 4-bit MatMulNBits weights to float16 [N, K]."""
    num_blocks = K // block_size
    bytes_per_block = block_size // 2
    
    q_reshaped = q_raw.reshape(N, num_blocks, bytes_per_block)
    low = (q_reshaped & 0x0F).astype(np.float32)
    high = (q_reshaped >> 4).astype(np.float32)
    
    unpacked_w = np.stack([low, high], axis=-1).reshape(N, num_blocks, block_size)
    s = scales.reshape(N, num_blocks)
    
    if zp_raw is not None:
        zp_bytes_per_row = (num_blocks + 1) // 2
        zp_bytes = zp_raw.reshape(N, zp_bytes_per_row)
        zp_low = (zp_bytes & 0x0F).astype(np.float32)
        zp_high = (zp_bytes >> 4).astype(np.float32)
        zp = np.stack([zp_low, zp_high], axis=-1).reshape(N, -1)[:, :num_blocks]
    else:
        zp = np.zeros((N, num_blocks), dtype=np.float32)
        
    w_dequant = (unpacked_w - zp[:, :, None]) * s[:, :, None]
    return w_dequant.reshape(N, K).astype(np.float16)


def export_models(model_dir: Path, out_dir: Path):
    out_dir.mkdir(parents=True, exist_ok=True)
    encoder_path = model_dir / "encoder.onnx"
    decoder_path = model_dir / "decoder_joint.onnx"
    
    print(f"Loading {encoder_path}...")
    enc_model = onnx.load(str(encoder_path))
    enc_inits = {t.name: numpy_helper.to_array(t) for t in enc_model.graph.initializer}
    
    print(f"Loading {decoder_path}...")
    dec_model = onnx.load(str(decoder_path))
    dec_inits = {t.name: numpy_helper.to_array(t) for t in dec_model.graph.initializer}
    
    tensors = {}
    
    # 1. Process Encoder Initializers (named weights)
    print("Processing encoder weights...")
    for name, arr in enc_inits.items():
        if not name.startswith("onnx::"):
            tensors[name] = arr.astype(np.float16)
            
    # 2. Process Encoder MatMulNBits nodes
    for n in enc_model.graph.node:
        if n.op_type == "MatMulNBits":
            clean_name = n.name.strip("/").replace("/MatMul_Q4", ".weight").replace("/", ".")
            attrs = {a.name: onnx.helper.get_attribute_value(a) for a in n.attribute}
            K = attrs["K"]
            N = attrs["N"]
            bs = attrs.get("block_size", 128)
            
            q_name, s_name, zp_name = n.input[1], n.input[2], n.input[3] if len(n.input) > 3 else None
            q = enc_inits[q_name]
            s = enc_inits[s_name]
            zp = enc_inits[zp_name] if zp_name in enc_inits else None
            
            w = dequantize_matmul_nbits(q, s, zp, K, N, bs)
            tensors[clean_name] = w
            
    # Positional embeddings constant
    for n in enc_model.graph.node:
        if "Constant_2852" in n.name:
            tensors["encoder.pos_enc.pos_emb"] = numpy_helper.to_array(n.attribute[0].t).astype(np.float16)
            print(f"Extracted encoder.pos_enc.pos_emb shape: {tensors['encoder.pos_enc.pos_emb'].shape}")
            break
            
    # 3. Process Decoder & Joint Initializers
    print("Processing decoder & joint weights...")
    for name, arr in dec_inits.items():
        if not name.startswith("onnx::"):
            tensors[name] = arr.astype(np.float16)
            
    # Decoder LSTM initializers
    tensors["decoder.lstm.0.weight_ih"] = dec_inits["onnx::LSTM_205"].astype(np.float16)
    tensors["decoder.lstm.0.weight_hh"] = dec_inits["onnx::LSTM_206"].astype(np.float16)
    tensors["decoder.lstm.0.bias"] = dec_inits["onnx::LSTM_207"].astype(np.float16)
    
    tensors["decoder.lstm.1.weight_ih"] = dec_inits["onnx::LSTM_225"].astype(np.float16)
    tensors["decoder.lstm.1.weight_hh"] = dec_inits["onnx::LSTM_226"].astype(np.float16)
    tensors["decoder.lstm.1.bias"] = dec_inits["onnx::LSTM_227"].astype(np.float16)
    
    # Joint MatMulNBits nodes
    for n in dec_model.graph.node:
        if n.op_type == "MatMulNBits":
            attrs = {a.name: onnx.helper.get_attribute_value(a) for a in n.attribute}
            K = attrs["K"]
            N = attrs["N"]
            bs = attrs.get("block_size", 128)
            q_name, s_name, zp_name = n.input[1], n.input[2], n.input[3] if len(n.input) > 3 else None
            q = dec_inits[q_name]
            s = dec_inits[s_name]
            zp = dec_inits[zp_name] if zp_name in dec_inits else None
            w = dequantize_matmul_nbits(q, s, zp, K, N, bs)
            
            if "enc" in n.name:
                tensors["joint.enc.weight"] = w
            elif "pred" in n.name:
                tensors["joint.pred.weight"] = w
            elif "joint_net.2" in n.name:
                tensors["joint.joint_net.2.weight"] = w
                
    out_safetensors = out_dir / "model.safetensors"
    print(f"Saving {len(tensors)} tensors to {out_safetensors}...")
    save_file(tensors, str(out_safetensors))
    
    tok_src = model_dir / "tokenizer.model"
    if tok_src.exists():
        import shutil
        shutil.copy2(tok_src, out_dir / "tokenizer.model")
        print("Copied tokenizer.model")
        
    print(f"Export successful! Total file size: {out_safetensors.stat().st_size / 1e6:.1f} MB")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--model-dir", type=Path, default=Path("/Users/abdullahusmani/Library/Application Support/Taurscribe/models/parakeet-nemotron"))
    parser.add_argument("--out-dir", type=Path, default=Path("parakeet-nemotron-mlx"))
    args = parser.parse_args()
    export_models(args.model_dir, args.out_dir)
