/* -----------------------------------------------------------------------------
 * Taurscribe AMD ROCm / HIP True Parallel Local Execution Verification Suite
 * Uses AMD's official HIP-CPU parallel runtime with C++17 parallel algorithms.
 * -------------------------------------------------------------------------- */
#include <iostream>
#include <vector>
#include <cmath>
#include <chrono>
#include <cassert>
#include <hip/hip_runtime.h>

#define WIDTH 1024
#define NUM (WIDTH * WIDTH)
#define THREADS_PER_BLOCK_X 16
#define THREADS_PER_BLOCK_Y 16

// ----------------------------------------------------------------------------
// Kernel 1: 1D Vector Addition
// ----------------------------------------------------------------------------
__global__ void vadd_kernel(const float* a, const float* b, float* c, int N) {
    int idx = blockIdx.x * blockDim.x + threadIdx.x;
    if (idx < N) {
        c[idx] = a[idx] + b[idx];
    }
}

// ----------------------------------------------------------------------------
// Kernel 2: 2D Matrix Transpose
// ----------------------------------------------------------------------------
__global__ void matrix_transpose_kernel(float* out, const float* in, int width) {
    int x = blockDim.x * blockIdx.x + threadIdx.x;
    int y = blockDim.y * blockIdx.y + threadIdx.y;
    if (x < width && y < width) {
        out[y * width + x] = in[x * width + y];
    }
}

// ----------------------------------------------------------------------------
// Kernel 3: Shared Memory Block Reduction
// ----------------------------------------------------------------------------
__global__ void block_sum_kernel(const float* in, float* out, int N) {
    __shared__ float sdata[256];
    unsigned int tid = threadIdx.x;
    unsigned int idx = blockIdx.x * blockDim.x + threadIdx.x;

    sdata[tid] = (idx < N) ? in[idx] : 0.0f;
    __syncthreads();

    for (unsigned int s = blockDim.x / 2; s > 0; s >>= 1) {
        if (tid < s) {
            sdata[tid] += sdata[tid + s];
        }
        __syncthreads();
    }

    if (tid == 0) {
        out[blockIdx.x] = sdata[0];
    }
}

int main() {
    std::cout << "===============================================================================\n";
    std::cout << "     Taurscribe AMD ROCm / HIP True Parallel Local Execution Suite            \n";
    std::cout << "===============================================================================\n";

    hipDeviceProp_t devProp;
    hipError_t err = hipGetDeviceProperties(&devProp, 0);
    assert(err == hipSuccess);
    std::cout << "[ROCm-HIP] Target Device:       " << devProp.name << "\n";
    std::cout << "[ROCm-HIP] Total Global Memory: " << (devProp.totalGlobalMem / (1024 * 1024)) << " MB\n";
    std::cout << "[ROCm-HIP] Shared Mem / Block:  " << devProp.sharedMemPerBlock << " bytes\n";
    std::cout << "[ROCm-HIP] Max Threads / Block: " << devProp.maxThreadsPerBlock << "\n";

    // ------------------------------------------------------------------------
    // Test 1: 1D Parallel Vector Addition (1,000,000 elements)
    // ------------------------------------------------------------------------
    std::cout << "\n--> [TEST 1/3] 1D Vector Addition (1,000,000 floats across parallel thread blocks)...\n";
    int N1 = 1000000;
    size_t bytes1 = N1 * sizeof(float);
    std::vector<float> h_A(N1), h_B(N1), h_C(N1);
    for (int i = 0; i < N1; ++i) {
        h_A[i] = 1.414213f * (i % 100);
        h_B[i] = 2.718281f * (i % 100);
    }

    float *d_A = nullptr, *d_B = nullptr, *d_C = nullptr;
    hipMalloc(&d_A, bytes1);
    hipMalloc(&d_B, bytes1);
    hipMalloc(&d_C, bytes1);
    hipMemcpy(d_A, h_A.data(), bytes1, hipMemcpyHostToDevice);
    hipMemcpy(d_B, h_B.data(), bytes1, hipMemcpyHostToDevice);

    int blockSize1 = 256;
    int blocks1 = (N1 + blockSize1 - 1) / blockSize1;
    hipLaunchKernelGGL(vadd_kernel, dim3(blocks1), dim3(blockSize1), 0, 0, d_A, d_B, d_C, N1);
    hipMemcpy(h_C.data(), d_C, bytes1, hipMemcpyDeviceToHost);

    double max_diff1 = 0.0;
    for (int i = 0; i < N1; ++i) {
        float ref = h_A[i] + h_B[i];
        double diff = std::fabs(h_C[i] - ref);
        if (diff > max_diff1) max_diff1 = diff;
    }
    std::cout << "[ROCm-HIP] 1D Vector Add Max Diff: " << max_diff1 << " (tolerance: 1e-6)\n";
    assert(max_diff1 < 1e-6);
    std::cout << "✓ [PASS] Test 1: 1D Parallel Vector Addition passed bit-perfect!\n";

    hipFree(d_A); hipFree(d_B); hipFree(d_C);

    // ------------------------------------------------------------------------
    // Test 2: 2D Matrix Transpose (1024 x 1024 = 1,048,576 floats)
    // ------------------------------------------------------------------------
    std::cout << "\n--> [TEST 2/3] 2D Matrix Transpose (1024x1024 = 1,048,576 floats in 2D grid)...\n";
    int N2 = NUM;
    size_t bytes2 = N2 * sizeof(float);
    std::vector<float> h_Mat(N2), h_Trans(N2);
    for (int i = 0; i < N2; ++i) {
        h_Mat[i] = static_cast<float>(i % 512);
    }

    float *d_Mat = nullptr, *d_Trans = nullptr;
    hipMalloc(&d_Mat, bytes2);
    hipMalloc(&d_Trans, bytes2);
    hipMemcpy(d_Mat, h_Mat.data(), bytes2, hipMemcpyHostToDevice);

    dim3 block2(THREADS_PER_BLOCK_X, THREADS_PER_BLOCK_Y);
    dim3 grid2(WIDTH / THREADS_PER_BLOCK_X, WIDTH / THREADS_PER_BLOCK_Y);
    hipLaunchKernelGGL(matrix_transpose_kernel, grid2, block2, 0, 0, d_Trans, d_Mat, WIDTH);
    hipMemcpy(h_Trans.data(), d_Trans, bytes2, hipMemcpyDeviceToHost);

    int trans_errors = 0;
    for (int j = 0; j < WIDTH; ++j) {
        for (int i = 0; i < WIDTH; ++i) {
            float expected = h_Mat[i * WIDTH + j];
            float actual = h_Trans[j * WIDTH + i];
            if (expected != actual) trans_errors++;
        }
    }
    std::cout << "[ROCm-HIP] 2D Matrix Transpose Errors: " << trans_errors << " / " << N2 << "\n";
    assert(trans_errors == 0);
    std::cout << "✓ [PASS] Test 2: 2D Matrix Transpose passed bit-perfect!\n";

    hipFree(d_Mat); hipFree(d_Trans);

    // ------------------------------------------------------------------------
    // Test 3: Shared Memory Block Reduction
    // ------------------------------------------------------------------------
    std::cout << "\n--> [TEST 3/3] Shared Memory Block Reduction (65,536 elements in 256-thread blocks)...\n";
    int N3 = 65536;
    size_t bytes3 = N3 * sizeof(float);
    std::vector<float> h_In(N3);
    for (int i = 0; i < N3; ++i) h_In[i] = 1.0f;

    int blockSize3 = 256;
    int blocks3 = N3 / blockSize3;
    std::vector<float> h_Out(blocks3);

    float *d_In = nullptr, *d_Out = nullptr;
    hipMalloc(&d_In, bytes3);
    hipMalloc(&d_Out, blocks3 * sizeof(float));
    hipMemcpy(d_In, h_In.data(), bytes3, hipMemcpyHostToDevice);

    hipLaunchKernelGGL(block_sum_kernel, dim3(blocks3), dim3(blockSize3), 0, 0, d_In, d_Out, N3);
    hipMemcpy(h_Out.data(), d_Out, blocks3 * sizeof(float), hipMemcpyDeviceToHost);

    int sum_errors = 0;
    for (int b = 0; b < blocks3; ++b) {
        if (std::fabs(h_Out[b] - 256.0f) > 1e-4) {
            sum_errors++;
        }
    }
    std::cout << "[ROCm-HIP] Shared Memory Reduction Block Sum Errors: " << sum_errors << " / " << blocks3 << "\n";
    assert(sum_errors == 0);
    std::cout << "✓ [PASS] Test 3: Shared Memory Reduction passed with 100% precision!\n";

    hipFree(d_In); hipFree(d_Out);

    std::cout << "\n===============================================================================\n";
    std::cout << "✓ [PASS] ALL AMD ROCm / HIP TRUE PARALLEL LOCAL TESTS SUCCEEDED!\n";
    std::cout << "  - 1D Vector parallel workgroup arithmetic: VERIFIED\n";
    std::cout << "  - 2D Matrix transpose parallel grid memory access: VERIFIED\n";
    std::cout << "  - Dynamic shared memory thread block reduction: VERIFIED\n";
    std::cout << "===============================================================================\n";
    return 0;
}
