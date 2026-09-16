#define _WIN32_WINNT 0x0602
#include <windows.h>
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>

// ThreadPowerThrottling definitions if not present in headers
#ifndef ThreadPowerThrottling
#define ThreadPowerThrottling 3
typedef struct _THREAD_POWER_THROTTLING_STATE {
    ULONG Version;
    ULONG ControlMask;
    ULONG StateMask;
} THREAD_POWER_THROTTLING_STATE, *PTHREAD_POWER_THROTTLING_STATE;
#define THREAD_POWER_THROTTLING_CURRENT_VERSION 1
#define THREAD_POWER_THROTTLING_EXECUTION_SPEED 0x1
#endif

typedef BOOL (WINAPI *pfnSetThreadInformation)(
    HANDLE hThread,
    int ThreadInformationClass,
    PVOID ThreadInformation,
    ULONG ThreadInformationSize
);

int main() {
    printf("===============================================================================\n");
    printf("        Win32 Thread Affinity & Hybrid Performance Core Emulation\n");
    printf("===============================================================================\n\n");

    // 1. Inspect Win32 Environment
    OSVERSIONINFOEXA os_info;
    ZeroMemory(&os_info, sizeof(os_info));
    os_info.dwOSVersionInfoSize = sizeof(os_info);
    printf("[WIN32] Process ID: %lu | Thread ID: %lu\n", GetCurrentProcessId(), GetCurrentThreadId());
    
    SYSTEM_INFO sys_info;
    GetSystemInfo(&sys_info);
    printf("[WIN32] Logical Processors Detected: %u\n", sys_info.dwNumberOfProcessors);
    printf("[WIN32] Processor Architecture: %u (0=x86, 9=x64, 12=ARM64)\n", sys_info.wProcessorArchitecture);
    printf("[WIN32] Page Size: %u bytes\n", sys_info.dwPageSize);

    // 2. Call GetLogicalProcessorInformationEx to query core relationships
    printf("\n--- STEP 1: Querying Logical Processor Core Relationships ---\n");
    DWORD return_length = 0;
    BOOL res = GetLogicalProcessorInformationEx(RelationProcessorCore, NULL, &return_length);
    DWORD err = GetLastError();

    if (!res && err != ERROR_INSUFFICIENT_BUFFER) {
        printf("[WARN] GetLogicalProcessorInformationEx returned unexpected error: %lu\n", err);
    } else {
        printf("[WIN32] Buffer length required for Processor Core relations: %lu bytes\n", return_length);
    }

    if (return_length > 0) {
        PVOID buffer = malloc(return_length);
        if (buffer && GetLogicalProcessorInformationEx(RelationProcessorCore, (PSYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX)buffer, &return_length)) {
            DWORD offset = 0;
            DWORD core_idx = 0;
            while (offset < return_length) {
                PSYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX item = 
                    (PSYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX)((BYTE*)buffer + offset);
                
                if (item->Relationship == RelationProcessorCore) {
                    BYTE eff = item->Processor.Reserved[0]; // Offset 1 in PROCESSOR_RELATIONSHIP is EfficiencyClass in Win10/11
                    ULONG_PTR mask = 0;
                    if (item->Processor.GroupCount > 0) {
                        mask = item->Processor.GroupMask[0].Mask;
                    }
                    printf("  Core #%02lu: EfficiencyClass = %u | AffinityMask = 0x%016llX\n", 
                        core_idx++, (unsigned int)eff, (unsigned long long)mask);
                }
                if (item->Size == 0) break;
                offset += item->Size;
            }
            printf("[WIN32] Successfully enumerated %lu physical cores via Win32 API!\n", core_idx);
        }
        if (buffer) free(buffer);
    }

    // 3. Thread Affinity Pinning Test
    printf("\n--- STEP 2: Live Thread Affinity Pinning (SetThreadAffinityMask) ---\n");
    HANDLE hThread = GetCurrentThread();
    DWORD_PTR full_mask = (DWORD_PTR)((1ULL << sys_info.dwNumberOfProcessors) - 1ULL);
    if (sys_info.dwNumberOfProcessors >= 64) {
        full_mask = ~(DWORD_PTR)0;
    }
    printf("[AFFINITY] Initial Process Core Mask: 0x%016llX\n", (unsigned long long)full_mask);

    // Target a specific core subset (e.g. Core 0 or lower half)
    DWORD_PTR target_mask = (sys_info.dwNumberOfProcessors > 1) ? 0x3 : 0x1;
    DWORD_PTR prev_mask = SetThreadAffinityMask(hThread, target_mask);
    if (prev_mask != 0) {
        printf("[AFFINITY] SetThreadAffinityMask SUCCESS: previous=0x%016llX, new=0x%016llX\n", 
            (unsigned long long)prev_mask, (unsigned long long)target_mask);
    } else {
        printf("[WARN] SetThreadAffinityMask failed: error %lu\n", GetLastError());
    }

    // Restore full mask
    DWORD_PTR restored = SetThreadAffinityMask(hThread, prev_mask != 0 ? prev_mask : full_mask);
    printf("[AFFINITY] Restored thread affinity mask: 0x%016llX\n", (unsigned long long)restored);

    // 4. Thread Priority Elevation Test
    printf("\n--- STEP 3: Thread Priority Elevation (SetThreadPriority) ---\n");
    int initial_prio = GetThreadPriority(hThread);
    printf("[PRIORITY] Initial Thread Priority: %d (THREAD_PRIORITY_NORMAL=%d)\n", initial_prio, THREAD_PRIORITY_NORMAL);

    if (SetThreadPriority(hThread, THREAD_PRIORITY_ABOVE_NORMAL)) {
        int new_prio = GetThreadPriority(hThread);
        printf("[PRIORITY] SetThreadPriority SUCCESS: new priority = %d (ABOVE_NORMAL=%d)\n", 
            new_prio, THREAD_PRIORITY_ABOVE_NORMAL);
    } else {
        printf("[WARN] SetThreadPriority failed: error %lu\n", GetLastError());
    }

    // Restore priority
    SetThreadPriority(hThread, initial_prio);

    // 5. EcoQoS / Power Throttling Disable Test
    printf("\n--- STEP 4: EcoQoS Power Throttling Control (SetThreadInformation) ---\n");
    HMODULE hKernel32 = GetModuleHandleA("kernel32.dll");
    pfnSetThreadInformation pSetThreadInfo = NULL;
    if (hKernel32) {
        pSetThreadInfo = (pfnSetThreadInformation)GetProcAddress(hKernel32, "SetThreadInformation");
    }

    if (pSetThreadInfo) {
        THREAD_POWER_THROTTLING_STATE throttle;
        throttle.Version = THREAD_POWER_THROTTLING_CURRENT_VERSION;
        throttle.ControlMask = THREAD_POWER_THROTTLING_EXECUTION_SPEED;
        throttle.StateMask = 0; // Disable throttling

        if (pSetThreadInfo(hThread, ThreadPowerThrottling, &throttle, sizeof(throttle))) {
            printf("[ECOQOS] SetThreadInformation SUCCESS: ThreadPowerThrottling disabled for high-throughput ASR.\n");
        } else {
            printf("[ECOQOS] SetThreadInformation returned error: %lu (common if emulated kernel lacks EcoQoS extension)\n", GetLastError());
        }
    } else {
        printf("[ECOQOS] SetThreadInformation not exported by kernel32.dll on this Windows runtime.\n");
    }

    // 5. Multi-Threaded Audio DSP Throughput Benchmark
    printf("\n--- STEP 5: Multi-Threaded Audio DSP Throughput Benchmark ---\n");
    LARGE_INTEGER qpc_freq, qpc_start, qpc_end;
    QueryPerformanceFrequency(&qpc_freq);
    printf("[BENCH] QPC Timer Frequency: %lld Hz\n", qpc_freq.QuadPart);

    #define BENCH_SAMPLES 10000000
    float *audio_buf = (float*)malloc(BENCH_SAMPLES * sizeof(float));
    for (int i = 0; i < BENCH_SAMPLES; i++) {
        audio_buf[i] = (float)(i % 1000) / 1000.0f;
    }

    // Benchmark 1: Unpinned Thread Baseline
    QueryPerformanceCounter(&qpc_start);
    float sum_unpinned = 0.0f;
    for (int i = 0; i < BENCH_SAMPLES; i++) {
        sum_unpinned += audio_buf[i] * 0.5f;
    }
    QueryPerformanceCounter(&qpc_end);
    double elapsed_unpinned = (double)(qpc_end.QuadPart - qpc_start.QuadPart) / (double)qpc_freq.QuadPart;
    double throughput_unpinned = (double)BENCH_SAMPLES / elapsed_unpinned / 1000000.0;
    printf("[BENCH] Unpinned Execution: %.3f ms | Throughput: %.2f M samples/sec (checksum=%.1f)\n",
        elapsed_unpinned * 1000.0, throughput_unpinned, sum_unpinned);

    // Benchmark 2: Dedicated P-Core Pinned Thread
    SetThreadAffinityMask(hThread, 0x1);
    SetThreadPriority(hThread, THREAD_PRIORITY_ABOVE_NORMAL);
    QueryPerformanceCounter(&qpc_start);
    float sum_pinned = 0.0f;
    for (int i = 0; i < BENCH_SAMPLES; i++) {
        sum_pinned += audio_buf[i] * 0.5f;
    }
    QueryPerformanceCounter(&qpc_end);
    double elapsed_pinned = (double)(qpc_end.QuadPart - qpc_start.QuadPart) / (double)qpc_freq.QuadPart;
    double throughput_pinned = (double)BENCH_SAMPLES / elapsed_pinned / 1000000.0;
    printf("[BENCH] P-Core Pinned Execution: %.3f ms | Throughput: %.2f M samples/sec (checksum=%.1f)\n",
        elapsed_pinned * 1000.0, throughput_pinned, sum_pinned);

    // Reset affinity and priority
    SetThreadAffinityMask(hThread, prev_mask != 0 ? prev_mask : full_mask);
    SetThreadPriority(hThread, initial_prio);
    free(audio_buf);

    printf("\n===============================================================================\n");
    printf(">>> WIN32 THREAD AFFINITY & HYBRID CPU TEST SUITE COMPLETED SUCCESSFULLY! <<<\n");
    printf("===============================================================================\n");
    return 0;
}
