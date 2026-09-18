#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

// CUDA Result codes
#define CUDA_SUCCESS 0
#define CUDA_ERROR_INVALID_VALUE 1
#define CUDA_ERROR_OUT_OF_MEMORY 2
#define CUDA_ERROR_NOT_INITIALIZED 3
#define CUDA_ERROR_DEINITIALIZED 4
#define CUDA_ERROR_NO_DEVICE 100
#define CUDA_ERROR_INVALID_DEVICE 101
#define CUDA_ERROR_INSUFFICIENT_DRIVER 35

// CUDA Runtime errors
#define cudaSuccess 0
#define cudaErrorMemoryAllocation 2
#define cudaErrorInitializationError 3
#define cudaErrorInsufficientDriver 35
#define cudaErrorNoDevice 100
#define cudaErrorInvalidDevice 101

typedef int CUresult;
typedef int cudaError_t;
typedef int CUdevice;
typedef void *CUcontext;
typedef uintptr_t CUdeviceptr;
typedef void *cudaStream_t;

typedef struct cudaDeviceProp {
    char name[256];
    size_t totalGlobalMem;
    size_t sharedMemPerBlock;
    int regsPerBlock;
    int warpSize;
    size_t memPitch;
    int maxThreadsPerBlock;
    int maxThreadsDim[3];
    int maxGridSize[3];
    int clockRate;
    size_t totalConstMem;
    int major;
    int minor;
    int multiProcessorCount;
    int integrated;
    int canMapHostMemory;
} cudaDeviceProp;

static const char *get_mock_mode(void) {
    const char *mode = getenv("MOCK_CUDA_MODE");
    return mode ? mode : "normal";
}

// ----------------------------------------------------------------------------
// CUDA Driver API Implementations
// ----------------------------------------------------------------------------

__attribute__((visibility("default")))
CUresult cuInit(unsigned int Flags) {
    const char *mode = get_mock_mode();
    printf("[CUDA-MOCK] cuInit(Flags=%u) [mode=%s]\n", Flags, mode);
    if (strcmp(mode, "driver_mismatch") == 0) {
        fprintf(stderr, "[CUDA-MOCK] Simulated fault: CUDA_ERROR_INSUFFICIENT_DRIVER\n");
        return CUDA_ERROR_INSUFFICIENT_DRIVER;
    }
    if (strcmp(mode, "no_device") == 0) {
        return CUDA_SUCCESS;
    }
    return CUDA_SUCCESS;
}

__attribute__((visibility("default")))
CUresult cuDriverGetVersion(int *driverVersion) {
    if (!driverVersion) return CUDA_ERROR_INVALID_VALUE;
    const char *mode = get_mock_mode();
    if (strcmp(mode, "driver_mismatch") == 0) {
        *driverVersion = 11000; // Old 11.0 driver
    } else {
        *driverVersion = 12090; // Modern 12.9 driver
    }
    printf("[CUDA-MOCK] cuDriverGetVersion() -> %d\n", *driverVersion);
    return CUDA_SUCCESS;
}

__attribute__((visibility("default")))
CUresult cuDeviceGetCount(int *count) {
    if (!count) return CUDA_ERROR_INVALID_VALUE;
    const char *mode = get_mock_mode();
    if (strcmp(mode, "no_device") == 0) {
        *count = 0;
    } else {
        *count = 1;
    }
    printf("[CUDA-MOCK] cuDeviceGetCount() -> %d\n", *count);
    return CUDA_SUCCESS;
}

__attribute__((visibility("default")))
CUresult cuDeviceGet(CUdevice *device, int ordinal) {
    if (!device) return CUDA_ERROR_INVALID_VALUE;
    const char *mode = get_mock_mode();
    if (strcmp(mode, "no_device") == 0 || ordinal < 0 || ordinal >= 1) {
        return CUDA_ERROR_INVALID_DEVICE;
    }
    *device = ordinal;
    return CUDA_SUCCESS;
}

__attribute__((visibility("default")))
CUresult cuDeviceGetName(char *name, int len, CUdevice dev) {
    if (!name || len <= 0) return CUDA_ERROR_INVALID_VALUE;
    strncpy(name, "NVIDIA GeForce RTX 4090 (Emulated via Mock Driver)", len - 1);
    name[len - 1] = '\0';
    return CUDA_SUCCESS;
}

__attribute__((visibility("default")))
CUresult cuDeviceTotalMem_v2(size_t *bytes, CUdevice dev) {
    if (!bytes) return CUDA_ERROR_INVALID_VALUE;
    const char *mode = get_mock_mode();
    if (strcmp(mode, "low_vram") == 0) {
        *bytes = (size_t)1024 * 1024 * 1024; // 1024 MB
    } else {
        *bytes = (size_t)24 * 1024 * 1024 * 1024ULL; // 24 GB
    }
    printf("[CUDA-MOCK] cuDeviceTotalMem_v2() -> %zu MB\n", *bytes / (1024 * 1024));
    return CUDA_SUCCESS;
}

__attribute__((visibility("default")))
CUresult cuMemGetInfo_v2(size_t *free, size_t *total) {
    const char *mode = get_mock_mode();
    if (strcmp(mode, "low_vram") == 0) {
        if (total) *total = (size_t)1024 * 1024 * 1024;
        if (free) *free = (size_t)512 * 1024 * 1024;
    } else {
        if (total) *total = (size_t)24 * 1024 * 1024 * 1024ULL;
        if (free) *free = (size_t)22 * 1024 * 1024 * 1024ULL;
    }
    printf("[CUDA-MOCK] cuMemGetInfo_v2() -> free: %zu MB, total: %zu MB\n",
           free ? *free / (1024 * 1024) : 0, total ? *total / (1024 * 1024) : 0);
    return CUDA_SUCCESS;
}

__attribute__((visibility("default")))
CUresult cuCtxCreate_v2(CUcontext *pctx, unsigned int flags, CUdevice dev) {
    if (!pctx) return CUDA_ERROR_INVALID_VALUE;
    *pctx = (void *)0xDEADBEEF;
    return CUDA_SUCCESS;
}

__attribute__((visibility("default")))
CUresult cuCtxDestroy_v2(CUcontext ctx) {
    return CUDA_SUCCESS;
}

__attribute__((visibility("default")))
CUresult cuMemAlloc_v2(CUdeviceptr *dptr, size_t bytesize) {
    if (!dptr) return CUDA_ERROR_INVALID_VALUE;
    const char *mode = get_mock_mode();
    if (strcmp(mode, "oom_inject") == 0) {
        fprintf(stderr, "[CUDA-MOCK] Simulated fault: cuMemAlloc_v2(%zu bytes) -> CUDA_ERROR_OUT_OF_MEMORY\n", bytesize);
        *dptr = 0;
        return CUDA_ERROR_OUT_OF_MEMORY;
    }
    void *ptr = malloc(bytesize);
    if (!ptr) return CUDA_ERROR_OUT_OF_MEMORY;
    *dptr = (CUdeviceptr)ptr;
    printf("[CUDA-MOCK] cuMemAlloc_v2(%zu bytes) -> 0x%lx\n", bytesize, (unsigned long)*dptr);
    return CUDA_SUCCESS;
}

__attribute__((visibility("default")))
CUresult cuMemFree_v2(CUdeviceptr dptr) {
    if (dptr) free((void *)dptr);
    return CUDA_SUCCESS;
}

__attribute__((visibility("default")))
CUresult cuMemcpyHtoD_v2(CUdeviceptr dstDevice, const void *srcHost, size_t ByteCount) {
    if (!dstDevice || !srcHost) return CUDA_ERROR_INVALID_VALUE;
    memcpy((void *)dstDevice, srcHost, ByteCount);
    return CUDA_SUCCESS;
}

__attribute__((visibility("default")))
CUresult cuMemcpyDtoH_v2(void *dstHost, CUdeviceptr srcDevice, size_t ByteCount) {
    if (!dstHost || !srcDevice) return CUDA_ERROR_INVALID_VALUE;
    memcpy(dstHost, (const void *)srcDevice, ByteCount);
    return CUDA_SUCCESS;
}

// ----------------------------------------------------------------------------
// CUDA Runtime API Implementations
// ----------------------------------------------------------------------------

__attribute__((visibility("default")))
cudaError_t cudaGetDeviceCount(int *count) {
    if (!count) return cudaErrorInitializationError;
    const char *mode = get_mock_mode();
    if (strcmp(mode, "no_device") == 0) {
        *count = 0;
        return cudaErrorNoDevice;
    }
    *count = 1;
    return cudaSuccess;
}

__attribute__((visibility("default")))
cudaError_t cudaSetDevice(int device) {
    if (device != 0) return cudaErrorInvalidDevice;
    return cudaSuccess;
}

__attribute__((visibility("default")))
cudaError_t cudaGetDeviceProperties(cudaDeviceProp *prop, int device) {
    if (!prop || device != 0) return cudaErrorInvalidDevice;
    memset(prop, 0, sizeof(cudaDeviceProp));
    strncpy(prop->name, "NVIDIA GeForce RTX 4090 (Mock Hardware Adapter)", sizeof(prop->name) - 1);
    const char *mode = get_mock_mode();
    if (strcmp(mode, "low_vram") == 0) {
        prop->totalGlobalMem = (size_t)1024 * 1024 * 1024;
    } else {
        prop->totalGlobalMem = (size_t)24 * 1024 * 1024 * 1024ULL;
    }
    prop->major = 8;
    prop->minor = 9;
    prop->multiProcessorCount = 128;
    return cudaSuccess;
}

__attribute__((visibility("default")))
cudaError_t cudaMalloc(void **devPtr, size_t size) {
    if (!devPtr) return cudaErrorInitializationError;
    const char *mode = get_mock_mode();
    if (strcmp(mode, "oom_inject") == 0) {
        fprintf(stderr, "[CUDA-MOCK] Simulated fault: cudaMalloc(%zu bytes) -> cudaErrorMemoryAllocation\n", size);
        *devPtr = NULL;
        return cudaErrorMemoryAllocation;
    }
    *devPtr = malloc(size);
    if (!*devPtr) return cudaErrorMemoryAllocation;
    printf("[CUDA-MOCK] cudaMalloc(%zu bytes) -> %p\n", size, *devPtr);
    return cudaSuccess;
}

__attribute__((visibility("default")))
cudaError_t cudaFree(void *devPtr) {
    if (devPtr) free(devPtr);
    return cudaSuccess;
}

__attribute__((visibility("default")))
cudaError_t cudaMemcpy(void *dst, const void *src, size_t count, int kind) {
    if (!dst || !src) return cudaErrorInitializationError;
    memcpy(dst, src, count);
    return cudaSuccess;
}

__attribute__((visibility("default")))
cudaError_t cudaStreamCreate(cudaStream_t *pStream) {
    if (!pStream) return cudaErrorInitializationError;
    *pStream = (void *)0xBAADF00D;
    return cudaSuccess;
}

__attribute__((visibility("default")))
cudaError_t cudaStreamSynchronize(cudaStream_t stream) {
    return cudaSuccess;
}

__attribute__((visibility("default")))
const char *cudaGetErrorString(cudaError_t error) {
    switch (error) {
        case cudaSuccess: return "cudaSuccess";
        case cudaErrorMemoryAllocation: return "out of memory";
        case cudaErrorInitializationError: return "initialization error";
        case cudaErrorInsufficientDriver: return "CUDA driver version is insufficient for CUDA runtime version";
        case cudaErrorNoDevice: return "no CUDA-capable device is detected";
        default: return "unknown error";
    }
}
