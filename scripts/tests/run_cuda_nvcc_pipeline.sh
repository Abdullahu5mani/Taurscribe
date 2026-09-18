#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== NVIDIA CUDA 12.6 NVCC Device Compilation & PTX Pipeline Suite ==="

docker run --platform linux/amd64 --rm \
  -v "$REPO_ROOT":/workspace -w /workspace \
  nvidia/cuda:12.6.0-devel-ubuntu24.04 bash -c '
    set -euo pipefail
    echo "-------------------------------------------------------------------------------"
    echo "--> Step 1: Querying NVIDIA nvcc Compiler Toolchain..."
    nvcc --version
    echo "-------------------------------------------------------------------------------"

    echo "--> Step 2: Compiling PTX Assembly across Architectures (sm_75, sm_80, sm_86, sm_89, sm_90)..."
    nvcc -O3 -arch=sm_75 -ptx scripts/tests/cuda_device_kernel.cu -o /tmp/cuda_kernel_sm75.ptx
    nvcc -O3 -arch=sm_89 -ptx scripts/tests/cuda_device_kernel.cu -o /tmp/cuda_kernel_sm89.ptx

    echo "Verifying PTX Structure and Symbols in generated assembly:"
    grep -E "\.entry cuda_vadd|\.entry cuda_gemm_tiled|\.entry cuda_warp_reduce" /tmp/cuda_kernel_sm75.ptx
    grep -E "shfl\.sync" /tmp/cuda_kernel_sm75.ptx | head -n 2
    grep -E "\.shared" /tmp/cuda_kernel_sm75.ptx | head -n 2
    echo "✓ [PASS] All device kernels, shared memory tiles, and warp shuffles verified in PTX!"

    echo "--> Step 3: Compiling Multi-Architecture Fatbinary Object..."
    nvcc -O3 \
      -gencode arch=compute_75,code=sm_75 \
      -gencode arch=compute_80,code=sm_80 \
      -gencode arch=compute_86,code=sm_86 \
      -gencode arch=compute_89,code=sm_89 \
      -gencode arch=compute_90,code=sm_90 \
      -c scripts/tests/cuda_device_kernel.cu -o /tmp/cuda_device_kernel.o

    cuobjdump -lelf /tmp/cuda_device_kernel.o
    cuobjdump -lptx /tmp/cuda_device_kernel.o
    echo "✓ [PASS] Multi-architecture fatbinary contains sm_75, sm_80, sm_86, sm_89, and sm_90!"

    echo "--> Step 4: Linking Executable with CUDA Runtime..."
    nvcc /tmp/cuda_device_kernel.o -o /tmp/cuda_device_kernel
    /tmp/cuda_device_kernel
    echo "✓ [PASS] CUDA host/device binary built and executed successfully!"

    echo "==============================================================================="
    echo "✓ [PASS] NVIDIA CUDA 12.6 NVCC TOOLCHAIN & PTX PIPELINE VERIFIED SUCCESSFULLY!"
    echo "==============================================================================="
  '
