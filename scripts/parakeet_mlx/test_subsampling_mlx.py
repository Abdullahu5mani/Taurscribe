import mlx.core as mx
import numpy as np
import onnxruntime as ort
from safetensors import safe_open
from pathlib import Path

# 1. Run ONNX pre_encode
onnx_path = "/tmp/encoder_pre_encode_debug2.onnx"
sess = ort.InferenceSession(onnx_path)

mel = np.ones((1, 128, 65), dtype=np.float32)
length = np.array([65], dtype=np.int64)
c_ch = np.zeros((24, 1, 70, 1024), dtype=np.float32)
c_tm = np.zeros((24, 1, 1024, 8), dtype=np.float32)
c_len = np.zeros((1,), dtype=np.int64)

onnx_out = sess.run(["/encoder/pre_encode/out/Add_output_0"], {
    "processed_signal": mel,
    "processed_signal_length": length,
    "cache_last_channel": c_ch,
    "cache_last_time": c_tm,
    "cache_last_channel_len": c_len
})[0] # [1, 9, 1024]

# 2. Run MLX pre_encode
sf_path = "/Users/abdullahusmani/Library/Application Support/Taurscribe/models/parakeet-nemotron-mlx/model.safetensors"
weights = {}
with safe_open(sf_path, framework="numpy") as f:
    for k in f.keys():
        if "pre_encode" in k:
            weights[k] = mx.array(f.get_tensor(k))

x = mx.array(mel.transpose(0, 2, 1)[:, :, :, None])

def pad_2x2(t):
    return mx.pad(t, [(0, 0), (2, 1), (2, 1), (0, 0)])

# conv.0
x = pad_2x2(x)
w0 = weights["encoder.pre_encode.conv.0.weight"].transpose(0, 2, 3, 1)
b0 = weights["encoder.pre_encode.conv.0.bias"]
x = mx.conv2d(x, w0, stride=(2, 2)) + b0
x = mx.maximum(x, 0.0)

# conv.2
x = pad_2x2(x)
w2 = weights["encoder.pre_encode.conv.2.weight"].transpose(0, 2, 3, 1)
b2 = weights["encoder.pre_encode.conv.2.bias"]
x = mx.conv2d(x, w2, stride=(2, 2), groups=256) + b2

# conv.3
w3 = weights["encoder.pre_encode.conv.3.weight"].transpose(0, 2, 3, 1)
b3 = weights["encoder.pre_encode.conv.3.bias"]
x = mx.conv2d(x, w3, stride=(1, 1)) + b3
x = mx.maximum(x, 0.0)

# conv.5
x = pad_2x2(x)
w5 = weights["encoder.pre_encode.conv.5.weight"].transpose(0, 2, 3, 1)
b5 = weights["encoder.pre_encode.conv.5.bias"]
x = mx.conv2d(x, w5, stride=(2, 2), groups=256) + b5

# conv.6
w6 = weights["encoder.pre_encode.conv.6.weight"].transpose(0, 2, 3, 1)
b6 = weights["encoder.pre_encode.conv.6.bias"]
x = mx.conv2d(x, w6, stride=(1, 1)) + b6
x = mx.maximum(x, 0.0)

x = x.transpose(0, 1, 3, 2).reshape(1, 9, 4352)
w_out = weights["encoder.pre_encode.out.weight"]
b_out = weights["encoder.pre_encode.out.bias"]
x = x @ w_out.T + b_out

corr = np.corrcoef(np.array(x).flatten(), onnx_out.flatten())[0, 1]
print(f"MLX Subsampling correlation with ONNX: {corr:.7f}")
assert corr > 0.9999, f"Correlation too low: {corr}"
print("SUCCESS: MLX Subsampling passes parity check!")
