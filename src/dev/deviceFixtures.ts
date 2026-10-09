// Devices and speed-test results of a PC with an NVIDIA card (CUDA + Vulkan
// build), an AMD iGPU and an NPU, for the dev previews.
export const DEMO_DEVICES = [
    { id: "cuda:NVIDIA GeForce RTX 5070", name: "NVIDIA GeForce RTX 5070", backend: "CUDA", kind: "gpu", memory_gb: 12, usable: true, note: null },
    { id: "vulkan:NVIDIA GeForce RTX 5070", name: "NVIDIA GeForce RTX 5070", backend: "Vulkan", kind: "gpu", memory_gb: 12, usable: true, note: null },
    { id: "vulkan:AMD Radeon(TM) 780M Graphics", name: "AMD Radeon(TM) 780M Graphics", backend: "Vulkan", kind: "igpu", memory_gb: 2, usable: true, note: null },
    { id: "cpu", name: "CPU", backend: "CPU", kind: "cpu", memory_gb: null, usable: true, note: null },
    { id: "npu:NPU Compute Accelerator Device", name: "NPU Compute Accelerator Device", backend: "NPU", kind: "npu", memory_gb: null, usable: false, note: "ggml has no backend for this NPU yet" },
];

export const DEMO_SPEEDS = { engine: "whisper", model: "base.en-q5_1", clip_seconds: 11, rounds: 3, include_flowscribe: false, restore_error: null, results: [
    { device_id: "cuda:NVIDIA GeForce RTX 5070", name: "NVIDIA GeForce RTX 5070", backend: "CUDA", load_s: 0.4, transcribe_s: 0.31, realtime: 96.8, error: null },
    { device_id: "vulkan:NVIDIA GeForce RTX 5070", name: "NVIDIA GeForce RTX 5070", backend: "Vulkan", load_s: 0.9, transcribe_s: 0.45, realtime: 66.7, error: null },
    { device_id: "vulkan:AMD Radeon(TM) 780M Graphics", name: "AMD Radeon(TM) 780M Graphics", backend: "Vulkan", load_s: 1.1, transcribe_s: 2.6, realtime: 11.5, error: null },
    { device_id: "cpu", name: "CPU", backend: "CPU", load_s: 0.3, transcribe_s: 4.1, realtime: 7.3, error: null },
].map(r => ({ ...r, unloaded_total_s: r.load_s + r.transcribe_s, loaded_total_s: r.transcribe_s, flowscribe_load_s: 0, flowscribe_s: 0, flowscribe_device_id: null, realtime: 11 / r.transcribe_s })) };
