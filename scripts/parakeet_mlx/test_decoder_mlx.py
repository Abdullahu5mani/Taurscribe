import mlx.core as mx
import numpy as np
import onnxruntime as ort
from safetensors import safe_open
from pathlib import Path

# 1. Run ONNX baseline
onnx_path = "/Users/abdullahusmani/Library/Application Support/Taurscribe/models/parakeet-nemotron/decoder_joint.onnx"
session = ort.InferenceSession(onnx_path)

enc_out = np.random.randn(1, 1024, 1).astype(np.float32)
targets = np.array([[42]], dtype=np.int32)
target_len = np.array([1], dtype=np.int32)
s1 = np.random.randn(2, 1, 640).astype(np.float32)
s2 = np.random.randn(2, 1, 640).astype(np.float32)

onnx_res = session.run(None, {
    "encoder_outputs": enc_out,
    "targets": targets,
    "target_length": target_len,
    "input_states_1": s1,
    "input_states_2": s2
})
onnx_logits = onnx_res[0]
onnx_s1 = onnx_res[2]
onnx_s2 = onnx_res[3]

# 2. Run MLX implementation
sf_path = "/Users/abdullahusmani/Library/Application Support/Taurscribe/models/parakeet-nemotron-mlx/model.safetensors"
weights = {}
with safe_open(sf_path, framework="mlx") as f:
    for k in f.keys():
        if k.startswith("decoder.") or k.startswith("joint."):
            weights[k] = f.get_tensor(k)

# Step 2a: Token embedding
embed_w = weights["decoder.prediction.embed.weight"] # [1025, 640]
token_id = int(targets[0, 0])
x = embed_w[token_id:token_id+1] # [1, 640]

# Step 2b: 2-layer LSTM
# PyTorch LSTM formula:
# gates = x @ W_ih.T + h_prev @ W_hh.T + b_ih + b_hh
# In ONNX, W is [1, 4*H, input_size], R is [1, 4*H, H], B is [1, 8*H]
# Gates order in ONNX LSTM: [i, o, f, c] (note: i, o, f, c or i, f, c, o?)
# Standard ONNX LSTM gates order: [i, o, f, c]!
# Let s verify LSTM gate order in ONNX specification:
# ONNX LSTM uses [i, o, f, c] where:
# i = input, o = output, f = forget, c = cell

def lstm_cell_onnx(x_t, h_prev, c_prev, W, R, B):
    # x_t: [1, in_dim], h_prev: [1, H], c_prev: [1, H]
    # W: [1, 4*H, in_dim], R: [1, 4*H, H], B: [1, 8*H]
    W = W[0] # [4*H, in_dim]
    R = R[0] # [4*H, H]
    B = B[0] # [8*H]
    W_b = B[:4*640]
    R_b = B[4*640:]
    
    gates = (x_t @ W.T) + (h_prev @ R.T) + W_b + R_b # [1, 4*640]
    # Split into 4 gates: i, o, f, c
    i, o, f, c = mx.split(gates, 4, axis=-1)
    
    i_gate = mx.sigmoid(i)
    f_gate = mx.sigmoid(f)
    c_gate = mx.tanh(c)
    o_gate = mx.sigmoid(o)
    
    c_t = f_gate * c_prev + i_gate * c_gate
    h_t = o_gate * mx.tanh(c_t)
    return h_t, c_t

h0 = mx.array(s1[0]) # [1, 640]
c0 = mx.array(s2[0])
h1 = mx.array(s1[1])
c1 = mx.array(s2[1])

h0_next, c0_next = lstm_cell_onnx(
    x, h0, c0,
    weights["decoder.lstm.0.weight_ih"],
    weights["decoder.lstm.0.weight_hh"],
    weights["decoder.lstm.0.bias"]
)

h1_next, c1_next = lstm_cell_onnx(
    h0_next, h1, c1,
    weights["decoder.lstm.1.weight_ih"],
    weights["decoder.lstm.1.weight_hh"],
    weights["decoder.lstm.1.bias"]
)

# Step 2c: Joint network
# enc_out: [1, 1024, 1] -> transpose to [1, 1, 1024]
enc_x = mx.array(enc_out.transpose(0, 2, 1)) # [1, 1, 1024]
enc_proj = enc_x @ weights["joint.enc.weight"].T + weights["joint.enc.bias"] # [1, 1, 640]

pred_proj = h1_next @ weights["joint.pred.weight"].T + weights["joint.pred.bias"] # [1, 640]
pred_proj = pred_proj[:, None, :] # [1, 1, 640]

joint_sum = enc_proj + pred_proj
joint_act = mx.maximum(joint_sum, 0.0) # ReLU

logits = joint_act @ weights["joint.joint_net.2.weight"].T + weights["joint.joint_net.2.bias"] # [1, 1, 1025]
logits = logits[:, :, None, :] # [1, 1, 1, 1025]

# Compare with ONNX!
diff_logits = np.max(np.abs(np.array(logits) - onnx_logits))
diff_s1 = max(np.max(np.abs(np.array(h0_next) - onnx_s1[0])), np.max(np.abs(np.array(h1_next) - onnx_s1[1])))
diff_s2 = max(np.max(np.abs(np.array(c0_next) - onnx_s2[0])), np.max(np.abs(np.array(c1_next) - onnx_s2[1])))

print(f"Decoder Joint Parity Results:")
print(f"  Max Logits Diff: {diff_logits:.6f}")
print(f"  Max State 1 Diff: {diff_s1:.6f}")
print(f"  Max State 2 Diff: {diff_s2:.6f}")
if diff_logits < 0.05:
    print("SUCCESS: MLX Decoder Joint matches ONNX baseline!")
else:
    print("Mismatch detected!")
