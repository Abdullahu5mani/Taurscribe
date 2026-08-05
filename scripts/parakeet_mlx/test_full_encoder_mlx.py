import mlx.core as mx
import numpy as np
import onnxruntime as ort
from safetensors import safe_open
import math
import time

# 1. Run ONNX Full Encoder
onnx_path = "/Users/abdullahusmani/Library/Application Support/Taurscribe/models/parakeet-nemotron/encoder.onnx"
print("Loading ONNX encoder session...")
t0 = time.time()
sess = ort.InferenceSession(onnx_path)
print(f"ONNX session loaded in {time.time() - t0:.2f}s")

mel = np.ones((1, 128, 65), dtype=np.float32)
length = np.array([65], dtype=np.int64)
c_ch = np.zeros((24, 1, 70, 1024), dtype=np.float32)
c_tm = np.zeros((24, 1, 1024, 8), dtype=np.float32)
c_len = np.zeros((1,), dtype=np.int64)

t0 = time.time()
onnx_res = sess.run(None, {
    "processed_signal": mel, "processed_signal_length": length,
    "cache_last_channel": c_ch, "cache_last_time": c_tm, "cache_last_channel_len": c_len
})
onnx_time = time.time() - t0
onnx_encoded = onnx_res[0] # [1, 1024, 7]
onnx_next_ch = onnx_res[2] # [24, 1, 70, 1024]
onnx_next_tm = onnx_res[3] # [24, 1, 1024, 8]
print(f"ONNX full encoder inference time: {onnx_time*1000:.2f} ms")

# 2. MLX Full Encoder
sf_path = "/Users/abdullahusmani/Library/Application Support/Taurscribe/models/parakeet-nemotron-mlx/model.safetensors"
print("Loading MLX safetensors...")
weights = {}
with safe_open(sf_path, framework="numpy") as f:
    for k in f.keys():
        weights[k] = mx.array(f.get_tensor(k), dtype=mx.float32)

def layer_norm(t, w, b, eps=1e-5):
    mean = mx.mean(t, axis=-1, keepdims=True)
    var = mx.var(t, axis=-1, keepdims=True)
    return (t - mean) / mx.sqrt(var + eps) * w + b

def silu(t):
    return t * mx.sigmoid(t)

def pad_2x2(t):
    return mx.pad(t, [(0, 0), (2, 1), (2, 1), (0, 0)])

# Pre-encode
x = mx.array(mel.transpose(0, 2, 1)[:, :, :, None])
x = pad_2x2(x)
w0 = weights["encoder.pre_encode.conv.0.weight"].transpose(0, 2, 3, 1)
b0 = weights["encoder.pre_encode.conv.0.bias"]
x = mx.maximum(mx.conv2d(x, w0, stride=(2, 2)) + b0, 0.0)

x = pad_2x2(x)
w2 = weights["encoder.pre_encode.conv.2.weight"].transpose(0, 2, 3, 1)
b2 = weights["encoder.pre_encode.conv.2.bias"]
x = mx.conv2d(x, w2, stride=(2, 2), groups=256) + b2

w3 = weights["encoder.pre_encode.conv.3.weight"].transpose(0, 2, 3, 1)
b3 = weights["encoder.pre_encode.conv.3.bias"]
x = mx.maximum(mx.conv2d(x, w3, stride=(1, 1)) + b3, 0.0)

x = pad_2x2(x)
w5 = weights["encoder.pre_encode.conv.5.weight"].transpose(0, 2, 3, 1)
b5 = weights["encoder.pre_encode.conv.5.bias"]
x = mx.conv2d(x, w5, stride=(2, 2), groups=256) + b5

w6 = weights["encoder.pre_encode.conv.6.weight"].transpose(0, 2, 3, 1)
b6 = weights["encoder.pre_encode.conv.6.bias"]
x = mx.maximum(mx.conv2d(x, w6, stride=(1, 1)) + b6, 0.0)

x = x.transpose(0, 1, 3, 2).reshape(1, 9, 4352)
x = x @ weights["encoder.pre_encode.out.weight"].T + weights["encoder.pre_encode.out.bias"]
x = x[:, 2:, :] # [1, 7, 1024]
B, T_q, D = x.shape

# Conformer 24 layers
pos_table = weights["encoder.pos_enc.pos_emb"] # [1, 9999, 1024]
# slice 153 frames
pos_slice = pos_table[:, 4923:5076, :] # [1, 153, 1024]

mlx_next_ch_list = []
mlx_next_tm_list = []

t_mlx_start = time.time()
for l in range(24):
    pfx = f"encoder.layers.{l}."
    
    # FF1
    res = x
    ff1 = layer_norm(x, weights[pfx + "norm_feed_forward1.weight"], weights[pfx + "norm_feed_forward1.bias"])
    ff1 = silu(ff1 @ weights[pfx + "feed_forward1.linear1.weight"].T)
    ff1 = ff1 @ weights[pfx + "feed_forward1.linear2.weight"].T
    x = res + 0.5 * ff1
    
    # Self Attn
    res = x
    x_norm = layer_norm(x, weights[pfx + "norm_self_att.weight"], weights[pfx + "norm_self_att.bias"])
    cache_ch_l = mx.array(c_ch[l]) # [1, 70, 1024]
    k_in = mx.concatenate([cache_ch_l, x_norm], axis=1) # [1, 77, 1024]
    T_k = k_in.shape[1]
    
    # Cache next: last 70 frames
    mlx_next_ch_list.append(k_in[:, -70:, :])
    
    q = (x_norm @ weights[pfx + "self_attn.linear_q.weight"].T).reshape(B, T_q, 8, 128)
    k = (k_in @ weights[pfx + "self_attn.linear_k.weight"].T).reshape(B, T_k, 8, 128)
    v = (k_in @ weights[pfx + "self_attn.linear_v.weight"].T).reshape(B, T_k, 8, 128)
    p = (pos_slice @ weights[pfx + "self_attn.linear_pos.weight"].T).reshape(1, 153, 8, 128)
    
    u = weights[pfx + "self_attn.pos_bias_u"][None, None, :, :]
    v_bias = weights[pfx + "self_attn.pos_bias_v"][None, None, :, :]
    
    q_u = (q + u).transpose(0, 2, 1, 3)
    q_v = (q + v_bias).transpose(0, 2, 1, 3)
    
    matrix_ac = q_u @ k.transpose(0, 2, 3, 1)
    matrix_bd = q_v @ p.transpose(0, 2, 3, 1)
    
    # rel_shift
    matrix_bd = mx.pad(matrix_bd, [(0, 0), (0, 0), (0, 0), (1, 0)])
    matrix_bd = matrix_bd.reshape(B, 8, 154, 7)
    matrix_bd = matrix_bd[:, :, 1:, :].reshape(B, 8, 7, 153)
    matrix_bd = matrix_bd[:, :, :, :T_k]
    
    scores = (matrix_ac + matrix_bd) * (1.0 / math.sqrt(128))
    scores = mx.concatenate([mx.full((B, 8, T_q, 70), -10000.0), scores[:, :, :, 70:]], axis=-1)
    attn = mx.softmax(scores, axis=-1)
    out = (attn @ v.transpose(0, 2, 1, 3)).transpose(0, 2, 1, 3).reshape(B, T_q, 1024)
    out = out @ weights[pfx + "self_attn.linear_out.weight"].T
    x = res + out
    
    # Conv
    res = x
    x_conv = layer_norm(x, weights[pfx + "norm_conv.weight"], weights[pfx + "norm_conv.bias"])
    w_pw1 = weights[pfx + "conv.pointwise_conv1.weight"].squeeze(-1)
    glu = x_conv @ w_pw1.T
    glu1, glu2 = mx.split(glu, 2, axis=-1)
    glu_out = glu1 * mx.sigmoid(glu2) # [1, 7, 1024]
    
    cache_tm_l = mx.array(c_tm[l]).transpose(0, 2, 1) # [1, 8, 1024]
    dw_in = mx.concatenate([cache_tm_l, glu_out], axis=1) # [1, 15, 1024]
    
    # Cache time next: last 8 frames, transposed back to [1, 1024, 8]
    mlx_next_tm_list.append(dw_in[:, -8:, :].transpose(0, 2, 1))
    
    w_dw = weights[pfx + "conv.depthwise_conv.weight"].transpose(0, 2, 1) # [1024, 9, 1]
    dw_out = mx.conv1d(dw_in, w_dw, stride=1, groups=1024)
    
    bn_out = layer_norm(dw_out, weights[pfx + "conv.batch_norm.weight"], weights[pfx + "conv.batch_norm.bias"])
    bn_act = silu(bn_out)
    w_pw2 = weights[pfx + "conv.pointwise_conv2.weight"].squeeze(-1)
    conv_out = bn_act @ w_pw2.T
    x = res + conv_out
    
    # FF2
    res = x
    ff2 = layer_norm(x, weights[pfx + "norm_feed_forward2.weight"], weights[pfx + "norm_feed_forward2.bias"])
    ff2 = silu(ff2 @ weights[pfx + "feed_forward2.linear1.weight"].T)
    ff2 = ff2 @ weights[pfx + "feed_forward2.linear2.weight"].T
    x = res + 0.5 * ff2
    
    # Norm out
    x = layer_norm(x, weights[pfx + "norm_out.weight"], weights[pfx + "norm_out.bias"])

mx.eval(x)
mlx_time = time.time() - t_mlx_start
print(f"MLX 24 layers forward pass: {mlx_time*1000:.2f} ms")

# Encoder out: transpose to [1, 1024, 7]
mlx_encoded = x.transpose(0, 2, 1)

corr = np.corrcoef(np.array(mlx_encoded).flatten(), onnx_encoded.flatten())[0, 1]
print(f"Full Encoder Correlation: {corr:.7f}")
abs_diff = np.max(np.abs(np.array(mlx_encoded) - onnx_encoded))
print(f"Full Encoder Max Abs Diff: {abs_diff:.6f}")
if corr > 0.999:
    print("SUCCESS: Full MLX Encoder matches ONNX!")
