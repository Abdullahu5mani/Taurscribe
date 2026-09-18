#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== Compiling and Running CUDA Mock & Fault-Injection Suite ==="

# 1. Compile shared mock library
gcc -O2 -fPIC -shared "$REPO_ROOT/scripts/tests/cuda_mock_driver.c" -o /tmp/libcuda_mock.dylib || \
gcc -O2 -fPIC -shared "$REPO_ROOT/scripts/tests/cuda_mock_driver.c" -o /tmp/libcuda_mock.so

# 2. Compile test harness
if [ -f /tmp/libcuda_mock.dylib ]; then
  gcc -O2 "$REPO_ROOT/scripts/tests/test_cuda_mock_harness.c" /tmp/libcuda_mock.dylib -o /tmp/test_cuda_mock_harness
  /tmp/test_cuda_mock_harness all
else
  gcc -O2 "$REPO_ROOT/scripts/tests/test_cuda_mock_harness.c" -L/tmp -lcuda_mock -Wl,-rpath,/tmp -o /tmp/test_cuda_mock_harness
  /tmp/test_cuda_mock_harness all
fi
