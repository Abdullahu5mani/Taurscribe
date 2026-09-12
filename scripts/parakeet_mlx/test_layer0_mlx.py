import mlx.core as mx
import numpy as np
import onnxruntime as ort
from safetensors import safe_open
import math

# 1. Run ONNX
sess = ort.InferenceSession("/tmp/layer0_debug.onnx")
mel = np.ones((1, 128, 65), dtype=np.float32)
length = np.array([65], dtype=np.int64)
c_ch = np.zeros((24, 1, 70, 1024), dtype=np.float32)
c_tm = np.zeros((24, 1, 1024, 8), dtype=np.float32)
c_len = np.zeros((1,), dtype=np.int64)

onnx_res = sess.run([
    "/encoder/layers.0/norm_out/LayerNormalization_output_0",
    "/encoder/layers.0/conv/batch_norm/LayerNormalization_output_0"
], {
    "processed_signal": mel, "processed_signal_length": length,
    "cache_last_channel": c_ch, "cache_last_time": c_tm, "cache_last_channel_len": c_len
})
onnx_l0_out = onnx_res[0]

# 2. MLX
sf_path = "/Users/abdullahusmani/Library/Application Support/Taurscribe/models/parakeet-nemotron-mlx/model.safetensors"
weights = {}
with safe_open(sf_path, framework="numpy") as f:
    for k in f.keys():
        if "pre_encode" in k or "layers.0." in k or "pos_enc" in k:
            weights[k] = mx.array(f.get_tensor(k), dtype=mx.float32)

# Subsampling
x = mx.array(mel.transpose(0, 2, 1)[:, :, :, None])
def pad_2x2(t):
    return mx.pad(t, [(0, 0), (2, 1), (2, 1), (0, 0)])

x = pad_2x2(x)
w0 = weights["encoder.pre_encode.conv.0.weight"].transpose(0, 2, 3, 1)
b0 = weights["encoder.pre_encode.conv.0.bias"]
x = mx.conv2d(x, w0, stride=(2, 2)) + b0
x = mx.maximum(x, 0.0)

x = pad_2x2(x)
w2 = weights["encoder.pre_encode.conv.2.weight"].transpose(0, 2, 3, 1)
b2 = weights["encoder.pre_encode.conv.2.bias"]
x = mx.conv2d(x, w2, stride=(2, 2), groups=256) + b2

w3 = weights["encoder.pre_encode.conv.3.weight"].transpose(0, 2, 3, 1)
b3 = weights["encoder.pre_encode.conv.3.bias"]
x = mx.conv2d(x, w3, stride=(1, 1)) + b3
x = mx.maximum(x, 0.0)

x = pad_2x2(x)
w5 = weights["encoder.pre_encode.conv.5.weight"].transpose(0, 2, 3, 1)
b5 = weights["encoder.pre_encode.conv.5.bias"]
x = mx.conv2d(x, w5, stride=(2, 2), groups=256) + b5

w6 = weights["encoder.pre_encode.conv.6.weight"].transpose(0, 2, 3, 1)
b6 = weights["encoder.pre_encode.conv.6.bias"]
x = mx.conv2d(x, w6, stride=(1, 1)) + b6
x = mx.maximum(x, 0.0)

x = x.transpose(0, 1, 3, 2).reshape(1, 9, 4352)
w_out = weights["encoder.pre_encode.out.weight"]
b_out = weights["encoder.pre_encode.out.bias"]
x = x @ w_out.T + b_out # [1, 9, 1024]

# Slice [:, 2:, :]
x = x[:, 2:, :] # [1, 7, 1024]
B, T_q, D = x.shape

def layer_norm(t, w, b, eps=1e-5):
    mean = mx.mean(t, axis=-1, keepdims=True)
    var = mx.var(t, axis=-1, keepdims=True)
    return (t - mean) / mx.sqrt(var + eps) * w + b

def silu(t):
    return t * mx.sigmoid(t)

# Step 1: feed_forward1
res1 = x
ff1 = layer_norm(x, weights["encoder.layers.0.norm_feed_forward1.weight"], weights["encoder.layers.0.norm_feed_forward1.bias"])
ff1 = silu(ff1 @ weights["encoder.layers.0.feed_forward1.linear1.weight"].T)
ff1 = ff1 @ weights["encoder.layers.0.feed_forward1.linear2.weight"].T
x = res1 + 0.5 * ff1

# Step 2: self_attn
res2 = x
x_norm = layer_norm(x, weights["encoder.layers.0.norm_self_att.weight"], weights["encoder.layers.0.norm_self_att.bias"])
cache_ch = mx.array(c_ch[0]) # [1, 70, 1024]
k_in = mx.concatenate([cache_ch, x_norm], axis=1) # [1, 77, 1024]
T_k = k_in.shape[1]

q = x_norm @ weights["encoder.layers.0.self_attn.linear_q.weight"].T # [1, 7, 1024]
k = k_in @ weights["encoder.layers.0.self_attn.linear_k.weight"].T   # [1, 77, 1024]
v = k_in @ weights["encoder.layers.0.self_attn.linear_v.weight"].T   # [1, 77, 1024]

# Positional encoding
pos_table = weights["encoder.pos_enc.pos_emb"] # [1, 9999, 1024]
# slice from 5000 - 77 = 4923 to 5000 + 77 - 1 = 5076 -> 153 frames
pos_slice = pos_table[:, 4923:5076, :]
p = pos_slice @ weights["encoder.layers.0.self_attn.linear_pos.weight"].T # [1, 153, 1024]

q = q.reshape(B, T_q, 8, 128)
k = k.reshape(B, T_k, 8, 128)
v = v.reshape(B, T_k, 8, 128)
p = p.reshape(1, 153, 8, 128)

u = weights["encoder.layers.0.self_attn.pos_bias_u"][None, None, :, :] # [1, 1, 8, 128]
v_bias = weights["encoder.layers.0.self_attn.pos_bias_v"][None, None, :, :]

q_u = (q + u).transpose(0, 2, 1, 3) # [1, 8, 7, 128]
q_v = (q + v_bias).transpose(0, 2, 1, 3) # [1, 8, 7, 128]

k_t = k.transpose(0, 2, 3, 1) # [1, 8, 128, 77]
matrix_ac = q_u @ k_t # [1, 8, 7, 77]

p_t = p.transpose(0, 2, 3, 1) # [1, 8, 128, 153]
matrix_bd = q_v @ p_t # [1, 8, 7, 153]

# rel_shift: pad left with 1 column -> [1, 8, 7, 154]
matrix_bd = mx.pad(matrix_bd, [(0, 0), (0, 0), (0, 0), (1, 0)])
matrix_bd = matrix_bd.reshape(B, 8, 154, 7)
matrix_bd = matrix_bd[:, :, 1:, :] # [1, 8, 153, 7]
matrix_bd = matrix_bd.reshape(B, 8, 7, 153)
matrix_bd = matrix_bd[:, :, :, :T_k] # [1, 8, 7, 77]

scores = (matrix_ac + matrix_bd) * (1.0 / math.sqrt(128))

# Mask: cache is empty (len=0), so mask first 70 cols
# In general: invalid cache columns are [: 70 - cache_len]
scores_masked = mx.concatenate([
    mx.full((B, 8, T_q, 70), -10000.0),
    scores[:, :, :, 70:]
], axis=-1)

attn = mx.softmax(scores_masked, axis=-1)
v_trans = v.transpose(0, 2, 1, 3) # [1, 8, 77, 128]
out = attn @ v_trans # [1, 8, 7, 128]
out = out.transpose(0, 2, 1, 3).reshape(B, T_q, 1024)
out = out @ weights["encoder.layers.0.self_attn.linear_out.weight"].T
x = res2 + out

# Step 3: conv
res3 = x
x_conv = layer_norm(x, weights["encoder.layers.0.norm_conv.weight"], weights["encoder.layers.0.norm_conv.bias"])
# In MLX conv1d: expects [N, T, C]!
# Pointwise conv1: [2048, 1024, 1]
w_pw1 = weights["encoder.layers.0.conv.pointwise_conv1.weight"].squeeze(-1) # [2048, 1024]
glu = x_conv @ w_pw1.T # [1, 7, 2048]
glu1, glu2 = mx.split(glu, 2, axis=-1)
glu_out = glu1 * mx.sigmoid(glu2) # [1, 7, 1024]

# Depthwise conv1d with kernel 9: prepend 8 frames from cache_time
# cache_tm is [1, 1024, 8] -> transpose to [1, 8, 1024]
cache_tm = mx.array(c_tm[0]).transpose(0, 2, 1) # [1, 8, 1024]
dw_in = mx.concatenate([cache_tm, glu_out], axis=1) # [1, 15, 1024]
w_dw = weights["encoder.layers.0.conv.depthwise_conv.weight"] # [1024, 1, 9] -> MLX conv1d expects [C_out, K, C_in/groups] = [1024, 9, 1]
w_dw = w_dw.transpose(0, 2, 1) # [1024, 9, 1]
dw_out = mx.conv1d(dw_in, w_dw, stride=1, groups=1024) # [1, 7, 1024]

# Batch norm (layer norm)
bn_out = layer_norm(dw_out, weights["encoder.layers.0.conv.batch_norm.weight"], weights["encoder.layers.0.conv.batch_norm.bias"])
bn_act = silu(bn_out)

w_pw2 = weights["encoder.layers.0.conv.pointwise_conv2.weight"].squeeze(-1) # [1024, 1024]
conv_out = bn_act @ w_pw2.T
x = res3 + conv_out

# Step 4: feed_forward2
res4 = x
ff2 = layer_norm(x, weights["encoder.layers.0.norm_feed_forward2.weight"], weights["encoder.layers.0.norm_feed_forward2.bias"])
ff2 = silu(ff2 @ weights["encoder.layers.0.feed_forward2.linear1.weight"].T)
ff2 = ff2 @ weights["encoder.layers.0.feed_forward2.linear2.weight"].T
x = res4 + 0.5 * ff2

# Step 5: norm_out
l0_out = layer_norm(x, weights["encoder.layers.0.norm_out.weight"], weights["encoder.layers.0.norm_out.bias"])

corr = np.corrcoef(np.array(l0_out).flatten(), onnx_l0_out.flatten())[0, 1]
print(f"Layer 0 MLX correlation with ONNX: {corr:.7f}")
abs_diff = np.max(np.abs(np.array(l0_out) - onnx_l0_out))
print(f"Layer 0 max absolute diff: {abs_diff:.6f}")
