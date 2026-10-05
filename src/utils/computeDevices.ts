import { invoke } from "@tauri-apps/api/core";

/** A device the speech engines can run on (see gpu.rs `ComputeDevice`). */
export interface ComputeDevice {
    /** "cpu", or "<backend>:<name>" e.g. "vulkan:AMD Radeon(TM) Graphics". */
    id: string;
    name: string;
    /** "CUDA", "Vulkan", "Metal", "ROCm", "CPU" or "NPU". */
    backend: string;
    kind: "gpu" | "igpu" | "cpu" | "npu";
    memory_gb: number | null;
    usable: boolean;
    note: string | null;
}

export interface DeviceReport {
    build_flavor: string;
    compute_devices: ComputeDevice[];
    preferred_device: string;
}

/** One device's result from `benchmark_compute_devices`. */
export interface DeviceSpeed {
    device_id: string;
    name: string;
    backend: string;
    load_s: number;
    transcribe_s: number;
    /** Seconds of audio per second of compute. */
    realtime: number;
    error: string | null;
}

export interface SpeedReport {
    engine: string;
    model: string;
    clip_seconds: number;
    results: DeviceSpeed[];
}

export const COMPUTE_DEVICE_CHANGED_EVENT = "taurscribe:compute-device-changed";

let cached: Promise<DeviceReport | null> | null = null;

/** The device list, fetched once (detection shells out on Windows). */
export function loadDeviceReport(): Promise<DeviceReport | null> {
    cached ??= invoke<DeviceReport>("get_gpu_report").catch(() => {
        cached = null;
        return null;
    });
    return cached;
}

export interface DeviceChoices {
    /** Usable GPUs, ggml's first (the automatic pick) first. */
    gpus: ComputeDevice[];
    /** NPUs, listed so the user sees why they aren't offered. */
    npus: ComputeDevice[];
    /** More than one GPU: offer the full picker instead of GPU / CPU. */
    showPicker: boolean;
}

export function deviceChoices(report: DeviceReport | null): DeviceChoices {
    const devices = report?.compute_devices ?? [];
    const gpus = devices.filter((d) => d.usable && (d.kind === "gpu" || d.kind === "igpu"));
    return { gpus, npus: devices.filter((d) => d.kind === "npu"), showPicker: gpus.length > 1 };
}

/** The device id the UI shows as selected. "auto" means ggml's first GPU. */
export function selectedDeviceId(asrBackend: "gpu" | "cpu", computeDevice: string, gpus: ComputeDevice[]): string {
    if (asrBackend === "cpu" || gpus.length === 0) return "cpu";
    return gpus.some((g) => g.id === computeDevice) ? computeDevice : gpus[0].id;
}

/** "RTX 5070 · CUDA", or "CPU". */
export function deviceLabel(d: Pick<ComputeDevice, "name" | "backend" | "kind">): string {
    if (d.kind === "cpu") return "CPU";
    const name = d.name.replace(/^(NVIDIA|AMD|Intel\(R\)|Intel)\s+/i, "").replace(/\((TM|R)\)/g, "").replace(/\s+/g, " ").trim();
    return `${name} · ${d.backend}`;
}

/** The fastest result, by real-time factor; null when none succeeded. */
export function fastest(results: DeviceSpeed[]): DeviceSpeed | null {
    return results.filter((r) => !r.error && r.realtime > 0).reduce<DeviceSpeed | null>((best, r) => (!best || r.realtime > best.realtime ? r : best), null);
}
