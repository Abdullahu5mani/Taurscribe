import { expect, mock, test } from "bun:test";

const realCore = await import("@tauri-apps/api/core");
mock.module("@tauri-apps/api/core", () => ({ ...realCore, invoke: async () => null }));
const { deviceChoices, deviceLabel, fastest, selectedDeviceId } = await import("../../src/utils/computeDevices");
type Report = Parameters<typeof deviceChoices>[0];

const dev = (id: string, kind: "gpu" | "igpu" | "cpu" | "npu", usable = true) => ({
    id, name: id.split(":")[1] ?? "CPU", backend: id.split(":")[0].toUpperCase(), kind, memory_gb: null, usable, note: null,
});
const report = (...devices: ReturnType<typeof dev>[]): Report => ({ build_flavor: "nvidia", preferred_device: "auto", compute_devices: devices });

test("one GPU keeps the plain GPU / CPU switch", () => {
    const c = deviceChoices(report(dev("cuda:RTX 5070", "gpu"), dev("cpu", "cpu")));
    expect(c.showPicker).toBe(false);
    expect(c.gpus.map((g) => g.id)).toEqual(["cuda:RTX 5070"]);
});

test("CUDA and Vulkan, or a card and an iGPU, get the picker", () => {
    const c = deviceChoices(report(dev("cuda:RTX 5070", "gpu"), dev("vulkan:RTX 5070", "gpu"), dev("vulkan:Radeon 780M", "igpu"), dev("cpu", "cpu"), dev("npu:NPU Compute Accelerator Device", "npu", false)));
    expect(c.showPicker).toBe(true);
    expect(c.gpus).toHaveLength(3);
    expect(c.npus).toHaveLength(1);
});

test("no report means no GPUs and no picker", () => {
    expect(deviceChoices(null)).toEqual({ gpus: [], npus: [], showPicker: false });
});

test("the selection falls back to the first GPU, and CPU wins when chosen", () => {
    const gpus = deviceChoices(report(dev("cuda:A", "gpu"), dev("vulkan:B", "igpu"))).gpus;
    expect(selectedDeviceId("gpu", "auto", gpus)).toBe("cuda:A");
    expect(selectedDeviceId("gpu", "vulkan:B", gpus)).toBe("vulkan:B");
    expect(selectedDeviceId("gpu", "vulkan:unplugged", gpus)).toBe("cuda:A");
    expect(selectedDeviceId("cpu", "vulkan:B", gpus)).toBe("cpu");
    expect(selectedDeviceId("gpu", "auto", [])).toBe("cpu");
});

test("labels drop vendor prefixes and trademarks", () => {
    expect(deviceLabel({ name: "NVIDIA GeForce RTX 5070", backend: "CUDA", kind: "gpu" })).toBe("GeForce RTX 5070 · CUDA");
    expect(deviceLabel({ name: "AMD Radeon(TM) Graphics", backend: "Vulkan", kind: "igpu" })).toBe("Radeon Graphics · Vulkan");
    expect(deviceLabel({ name: "CPU", backend: "CPU", kind: "cpu" })).toBe("CPU");
});

test("fastest ignores failures", () => {
    const r = (device_id: string, realtime: number, error: string | null = null) => ({ device_id, name: "", backend: "", load_s: 1, transcribe_s: 1, realtime, error });
    expect(fastest([r("a", 20), r("b", 90, "out of memory"), r("c", 35)])?.device_id).toBe("c");
    expect(fastest([r("a", 0, "x")])).toBeNull();
});
