import { beforeEach, expect, mock, test } from "bun:test";
import { appendFileSync } from "node:fs";
import { renderHook } from "../frontend/hookHarness";

function deferred<T = void>() {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>(r => { resolve = r; });
    return { promise, resolve };
}
let scenario = "", sequence = 0;
const trace = (event: string, data: unknown = null) => {
    const row = { scenario, sequence: ++sequence, at: new Date().toISOString(), event, data };
    if (process.env.TAURSCRIBE_TEST_TRACE) appendFileSync(process.env.TAURSCRIBE_TEST_TRACE, JSON.stringify(row) + "\n");
};
let settings: Record<string, unknown>;
let readGate: Promise<void> | null;
let loadGate: Promise<void> | null;
let firstWriteGate: Promise<void> | null;
let deviceGate: Promise<void> | null;
let timeoutGate: Promise<void> | null;
let writes: number;
let rejectTimeout: boolean;
let nativeTimeout: number;
let nativeDevice: string;
let failWrite = false;
let failDevice = false;
const notices: string[] = [];
const store = {
    get: async (key: string) => {
        const value = settings[key] ?? null;
        trace("store.get", { key, value });
        if (key === "llm_backend" && readGate) await readGate;
        return value;
    },
    set: async (key: string, value: unknown) => {
        trace("store.set.start", { key, value });
        if (++writes === 1 && firstWriteGate) await firstWriteGate;
        if (failWrite) { failWrite = false; throw new Error("temporary store failure"); }
        settings[key] = value;
        trace("store.set.finish", { key, value });
    },
    save: async () => { trace("store.save", { ...settings }); },
};
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({ ...realCore, invoke: async (cmd: string, args: any) => {
    trace("invoke.start", { cmd, args });
    if (cmd === "set_compute_device") {
        if (args.id === "gpu:1" && deviceGate) await deviceGate;
        if (failDevice && args.id === "gpu:1") throw new Error("unavailable device");
        nativeDevice = args.id;
    }
    if (cmd === "set_auto_unload_timeout") {
        if (args.seconds === 300 && timeoutGate) await timeoutGate;
        if (rejectTimeout) throw new Error("native timeout unavailable");
        nativeTimeout = args.seconds;
    }
    trace("invoke.finish", { cmd, args, nativeTimeout, nativeDevice });
    if (cmd === "get_auto_unload_status") return { timeout_seconds: nativeTimeout, remaining_seconds: null, is_loaded: false, last_activity_epoch: 0 };
    return cmd === "check_grammar_llm_available" ? true : null;
} }));
const realStore = { ...(await import("@tauri-apps/plugin-store")) };
mock.module("@tauri-apps/plugin-store", () => ({ ...realStore, Store: { load: async () => {
    trace("store.load.start");
    if (loadGate) await loadGate;
    trace("store.load.finish");
    return store;
} } }));
const realEvent = { ...(await import("@tauri-apps/api/event")) };
mock.module("@tauri-apps/api/event", () => ({ ...realEvent, listen: async () => () => {} }));
const { usePostProcessing } = await import("../../src/hooks/usePostProcessing");
const { useAutoUnload } = await import("../../src/hooks/useAutoUnload");
const notice = (s: string) => { notices.push(s); trace("notice", s); };
const autoFixture = () => renderHook(() => useAutoUnload({ loadedEngine: null, setLoadedEngine: () => {}, setHeaderStatus: notice }));
beforeEach(() => {
    settings = {}; readGate = loadGate = firstWriteGate = deviceGate = timeoutGate = null;
    writes = 0; rejectTimeout = false; nativeTimeout = 1800; nativeDevice = "auto";
    notices.length = 0; sequence = 0;
    failWrite = failDevice = false;
});

test("post-processing changes made while opening the store cannot overwrite later choices", async () => {
    scenario = "pending-hydration";
    const loading = deferred(), reading = deferred(); loadGate = loading.promise; readGate = reading.promise;
    const h = renderHook(() => usePostProcessing(notice));
    h.handle.current.setTranscriptionStyle("Verbatim"); await h.settle();
    loading.resolve(); await h.settle();
    h.handle.current.setTranscriptionStyle("Formatted"); await h.settle();
    reading.resolve(); await h.settle();
    trace("final", { ui: h.handle.current.transcriptionStyle, saved: settings.transcription_style });
    expect(h.handle.current.transcriptionStyle).toBe("Formatted");
    expect(settings.transcription_style).toBe("Formatted");
});

test("rapid transcription style changes remain ordered when a store write is slow", async () => {
    scenario = "slow-store-write";
    const h = renderHook(() => usePostProcessing(notice, undefined, { current: store } as any)); await h.settle();
    const gate = deferred(); firstWriteGate = gate.promise;
    h.handle.current.setTranscriptionStyle("Verbatim"); await h.settle();
    h.handle.current.setTranscriptionStyle("Formatted"); await h.settle();
    gate.resolve(); await h.settle();
    trace("final", { ui: h.handle.current.transcriptionStyle, saved: settings.transcription_style });
    expect(settings.transcription_style).toBe("Formatted");
});

test("rapid GPU choices finish on the latest requested device", async () => {
    scenario = "overlapping-gpu-selection";
    const h = renderHook(() => usePostProcessing(notice, undefined, { current: store } as any)); await h.settle();
    const gate = deferred(); deviceGate = gate.promise;
    const first = h.handle.current.setComputeDevice("gpu:1"); await h.settle();
    const second = h.handle.current.setComputeDevice("gpu:2"); await h.settle();
    gate.resolve(); await Promise.all([first, second]); await h.settle();
    trace("final", { ui: h.handle.current.computeDevice, saved: settings.compute_device, nativeDevice });
    expect(h.handle.current.computeDevice).toBe("gpu:2");
    expect(settings.compute_device).toBe("gpu:2");
    expect(nativeDevice).toBe("gpu:2");
});

test("an in-flight startup timeout cannot replace a newer Never preference", async () => {
    scenario = "startup-timeout-in-flight";
    settings.auto_unload_timeout_secs = 300;
    const gate = deferred(); timeoutGate = gate.promise;
    const h = autoFixture(); await h.settle();
    const update = h.handle.current.updateTimeout(0); await h.settle();
    gate.resolve(); await update; await h.settle();
    trace("final", { ui: h.handle.current.timeoutSeconds, saved: settings.auto_unload_timeout_secs, nativeTimeout });
    expect(h.handle.current.timeoutSeconds).toBe(0);
    expect(nativeTimeout).toBe(0);
    expect(settings.auto_unload_timeout_secs).toBe(0);
});

test("rapid auto-unload updates finish on the latest selected timeout", async () => {
    scenario = "overlapping-timeouts";
    const h = autoFixture(); await h.settle();
    const gate = deferred(); timeoutGate = gate.promise;
    const first = h.handle.current.updateTimeout(300); await h.settle();
    const second = h.handle.current.updateTimeout(0); await h.settle();
    gate.resolve(); await Promise.all([first, second]); await h.settle();
    trace("final", { ui: h.handle.current.timeoutSeconds, saved: settings.auto_unload_timeout_secs, nativeTimeout });
    expect(h.handle.current.timeoutSeconds).toBe(0);
    expect(nativeTimeout).toBe(0);
    expect(settings.auto_unload_timeout_secs).toBe(0);
});

test("a rejected timeout update does not claim Never was applied", async () => {
    scenario = "timeout-rejection";
    settings.auto_unload_timeout_secs = 300;
    const h = autoFixture(); await h.settle(); rejectTimeout = true;
    await h.handle.current.updateTimeout(0); await h.settle();
    trace("final", { ui: h.handle.current.timeoutSeconds, saved: settings.auto_unload_timeout_secs, nativeTimeout, notices });
    expect(h.handle.current.timeoutSeconds).toBe(300);
    expect(nativeTimeout).toBe(300);
    expect(settings.auto_unload_timeout_secs).toBe(300);
    expect(notices.some(s => s.includes("Failed"))).toBe(true);
});

const settingsCases = [
    { key: "transcription_style", setter: "setTranscriptionStyle", state: "transcriptionStyle", values: ["Verbatim", "Clean", "Formatted"] },
    { key: "enable_denoise", setter: "setEnableDenoise", state: "enableDenoise", values: [false, true, false] },
    { key: "enable_overlay", setter: "setEnableOverlay", state: "enableOverlay", values: [false, true, false] },
    { key: "mute_background_audio", setter: "setMuteBackgroundAudio", state: "muteBackgroundAudio", values: [true, false, true] },
    { key: "llm_backend", setter: "setLlmBackend", state: "llmBackend", values: ["cpu", "gpu", "cpu"] },
    { key: "asr_backend", setter: "setAsrBackend", state: "asrBackend", values: ["cpu", "gpu", "cpu"] },
];
for (const item of settingsCases) for (const schedule of ["same-render", "between-renders", "store-opening", "read-in-flight"] as const) {
    test(`${item.key} keeps the last choice with ${schedule} scheduling`, async () => {
        scenario = `${item.key}/${schedule}`;
        const gate = deferred();
        if (schedule === "store-opening") loadGate = gate.promise;
        if (schedule === "read-in-flight") readGate = gate.promise;
        const h = renderHook(() => usePostProcessing(notice)); await h.settle();
        if (schedule === "same-render" || schedule === "between-renders") firstWriteGate = gate.promise;
        for (const value of item.values) {
            trace("user.select", { key: item.key, value });
            (h.handle.current as any)[item.setter](value);
            if (schedule === "between-renders") await h.settle();
        }
        gate.resolve(); await h.settle();
        trace("final", { ui: (h.handle.current as any)[item.state], saved: settings[item.key] });
        expect((h.handle.current as any)[item.state]).toBe(item.values.at(-1));
        expect(settings[item.key]).toBe(item.values.at(-1));
    });
}

test("a failed preference write does not block later choices from being saved", async () => {
    scenario = "store-error-recovery";
    const h = renderHook(() => usePostProcessing(notice, undefined, { current: store } as any)); await h.settle();
    failWrite = true;
    h.handle.current.setTranscriptionStyle("Verbatim");
    h.handle.current.setTranscriptionStyle("Formatted"); await h.settle();
    expect(settings.transcription_style).toBe("Formatted");
});

test("a rejected GPU choice does not block a newer working device", async () => {
    scenario = "device-error-recovery";
    const h = renderHook(() => usePostProcessing(notice, undefined, { current: store } as any)); await h.settle();
    const gate = deferred(); deviceGate = gate.promise; failDevice = true;
    const first = h.handle.current.setComputeDevice("gpu:1");
    const failed = first.catch(error => error);
    const second = h.handle.current.setComputeDevice("gpu:2"); await h.settle();
    gate.resolve(); const error = await failed; await second; await h.settle();
    expect(error.message).toBe("unavailable device");
    expect(h.handle.current.computeDevice).toBe("gpu:2");
    expect(nativeDevice).toBe("gpu:2");
    expect(settings.compute_device).toBe("gpu:2");
});

test("a failed timeout save retains the applied value and a later choice can save", async () => {
    scenario = "timeout-store-error-recovery";
    const h = autoFixture(); await h.settle(); failWrite = true;
    await h.handle.current.updateTimeout(0); await h.settle();
    expect(h.handle.current.timeoutSeconds).toBe(0);
    expect(nativeTimeout).toBe(0);
    expect(notices.some(s => s.includes("could not save"))).toBe(true);
    await h.handle.current.updateTimeout(900); await h.settle();
    expect(h.handle.current.timeoutSeconds).toBe(900);
    expect(nativeTimeout).toBe(900);
    expect(settings.auto_unload_timeout_secs).toBe(900);
});
