#include <stdio.h>
#include <cuda_runtime.h>

#define TILE_WIDTH 16

// ----------------------------------------------------------------------------
// Kernel 1: 1D Vector Addition with Grid-Stride Loop
// ----------------------------------------------------------------------------
extern "C" __global__ void cuda_vadd(const float *a, const float *b, float *c, int n) {
    int index = blockIdx.x * blockDim.x + threadIdx.x;
    int stride = blockDim.x * gridDim.x;
    for (int i = index; i < n; i += stride) {
        c[i] = a[i] + b[i];
    }
}

// ----------------------------------------------------------------------------
// Kernel 2: Tiled Matrix Multiplication (GEMM) with Shared Memory & Sync
// ----------------------------------------------------------------------------
extern "C" __global__ void cuda_gemm_tiled(const float *A, const float *B, float *C, int width) {
    __shared__ float s_A[TILE_WIDTH][TILE_WIDTH];
    __shared__ float s_B[TILE_WIDTH][TILE_WIDTH];

    int bx = blockIdx.x, by = blockIdx.y;
    int tx = threadIdx.x, ty = threadIdx.y;

    int row = by * TILE_WIDTH + ty;
    int col = bx * TILE_WIDTH + tx;

    float pvalue = 0.0f;

    for (int ph = 0; ph < width / TILE_WIDTH; ++ph) {
        s_A[ty][tx] = A[row * width + ph * TILE_WIDTH + tx];
        s_B[ty][tx] = B[(ph * TILE_WIDTH + ty) * width + col];
        __syncthreads();

        #pragma unroll
        for (int k = 0; k < TILE_WIDTH; ++k) {
            pvalue += s_A[ty][k] * s_B[k][tx];
        }
        __syncthreads();
    }

    if (row < width && col < width) {
        C[row * width + col] = pvalue;
    }
}

// ----------------------------------------------------------------------------
// Kernel 3: Warp-Level Reduction using __shfl_down_sync
// ----------------------------------------------------------------------------
extern "C" __global__ void cuda_warp_reduce(const float *input, float *output, int n) {
    unsigned int mask = 0xffffffff;
    int tid = threadIdx.x;
    float val = (tid < n) ? input[tid] : 0.0f;

    for (int offset = 16; offset > 0; offset /= 2) {
        val += __shfl_down_sync(mask, val, offset);
    }

    if (tid == 0) {
        output[blockIdx.x] = val;
    }
}

int main(int argc, char **argv) {
    printf("CUDA device and host symbols compiled cleanly.\n");
    return 0;
}
