import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "../frontend/hookHarness";
let failQwen = false;
let nativeLoaded: string | null;
const calls: string[] = [];
function deferred<T>() { let resolve!: (v: T) => void; const promise = new Promise<T>(r => { resolve = r; }); return { promise, resolve }; }
let graniteStatusGate: ReturnType<typeof deferred<any>> | null = null;
let graniteLoadGate: ReturnType<typeof deferred<any>> | null = null;
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({ ...realCore, invoke: async (cmd: string) => {
    calls.push(cmd);
    if (cmd === "init_granite") { if (graniteLoadGate) return graniteLoadGate.promise; nativeLoaded = "granite"; return { ok: true, data: "loaded" }; }
    if (cmd === "init_qwen3") {
        // commands/models.rs init_gguf unloads the competing engines first,
        // then can return a load error for its target.
        nativeLoaded = null;
        if (failQwen) return { ok: false, error: { code: "model_load_failed", message: "Unable to allocate target model" } };
        nativeLoaded = "qwen3"; return { ok: true, data: "loaded" };
    }
    if (cmd === "get_engine_selection_state") return { loaded_engine: nativeLoaded };
    if (cmd === "get_backend_info") return "test CPU";
    if (cmd === "get_granite_status" && graniteStatusGate) return graniteStatusGate.promise;
    if (cmd === "get_granite_status") return { loaded: nativeLoaded === "granite", model_id: "granite-test" };
    if (cmd === "get_qwen3_status") return { loaded: nativeLoaded === "qwen3", model_id: "qwen3-test" };
    throw new Error(`Unexpected ${cmd}`);
} }));
const { useEngineSwitch } = await import("../../src/hooks/useEngineSwitch");
const noop = () => {};
function fixture(qwenInstalled = true) {
    return renderHook(() => useEngineSwitch({
        models: [], graniteModels: [{ id: "granite-test" }], qwen3Models: qwenInstalled ? [{ id: "qwen3-test" }] : [],
        currentModel: null, currentGraniteModel: "granite-test", currentQwen3Model: "qwen3-test",
        setCurrentModel: noop, setCurrentGraniteModel: noop, setCurrentQwen3Model: noop,
        setBackendInfo: noop, setHeaderStatus: noop, setTrayState: async () => {},
        storeRef: { current: null }, asrBackend: "cpu", setAsrBackend: noop,
        isRecordingRef: { current: false }, downloadProgressRef: { current: {} },
    } as any));
}
beforeEach(() => { failQwen = false; nativeLoaded = null; calls.length = 0; graniteStatusGate = null; graniteLoadGate = null; });

test("a successful engine switch updates the loaded engine", async () => {
    const h = fixture(); await h.handle.current.handleSwitchToGranite(); await h.settle();
    await h.handle.current.handleSwitchToQwen3(); await h.settle();
    expect(nativeLoaded).toBe("qwen3");
    expect(h.handle.current.loadedEngine).toBe("qwen3");
    expect(h.handle.current.isLoading).toBe(false);
});

test("a failed switch cannot advertise an engine that native code already unloaded", async () => {
    const h = fixture(); await h.handle.current.handleSwitchToGranite(); await h.settle();
    expect(h.handle.current.loadedEngine).toBe("granite");
    failQwen = true;
    await h.handle.current.handleSwitchToQwen3(); await h.settle();
    expect(nativeLoaded).toBeNull();
    expect(h.handle.current.isLoading).toBe(false);
    expect(h.handle.current.loadedEngine).toBeNull();
});

test("simultaneous reloads cannot both pass an asynchronous status preflight", async () => {
    const h = fixture(); await h.handle.current.handleSwitchToGranite(); await h.settle();
    nativeLoaded = null; calls.length = 0;
    graniteStatusGate = deferred<any>(); graniteLoadGate = deferred<any>();
    const first = h.handle.current.handleSwitchToGranite();
    const second = h.handle.current.handleSwitchToGranite(); await h.settle();
    graniteStatusGate.resolve({ loaded: false }); await h.settle();
    const dispatched = calls.filter(c => c === "init_granite").length;
    graniteLoadGate.resolve({ ok: true, data: "loaded" });
    await Promise.all([first, second]); await h.settle();
    expect(dispatched).toBe(1);
});

test("an empty engine tab cannot change selection during another engine load", async () => {
    const h = fixture(false); graniteLoadGate = deferred<any>();
    const loading = h.handle.current.handleSwitchToGranite(); await h.settle();
    expect(h.handle.current.isLoading).toBe(true);
    await h.handle.current.handleSwitchToQwen3(); await h.settle();
    const activeWhileLoading = h.handle.current.activeEngine;
    graniteLoadGate.resolve({ ok: true, data: "loaded" }); await loading; await h.settle();
    expect(activeWhileLoading).toBe("whisper");
});
