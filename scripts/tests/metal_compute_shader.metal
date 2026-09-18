#include <metal_stdlib>
using namespace metal;

kernel void metal_tensor_compute(device const float *inA [[buffer(0)]],
                                 device const float *inB [[buffer(1)]],
                                 device float *out [[buffer(2)]],
                                 uint id [[thread_position_in_grid]]) {
    float a = inA[id];
    float b = inB[id];
    // Non-linear polynomial activation matching neural ASR attention/feedforward
    out[id] = (a * b) + sin(a) - cos(b);
}
