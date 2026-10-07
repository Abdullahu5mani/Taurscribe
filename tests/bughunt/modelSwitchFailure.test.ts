import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "../frontend/hookHarness";
let failQwen = false;
let nativeLoaded: string | null;
const calls: string[] = [];
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({ ...realCore, invoke: async (cmd: string) => {
    calls.push(cmd);
    if (cmd === "init_granite") { nativeLoaded = "granite"; return { ok: true, data: "loaded" }; }
    if (cmd === "init_qwen3") {
        // commands/models.rs init_gguf unloads the competing engines first,
        // then can return a load error for its target.
        nativeLoaded = null;
        if (failQwen) return { ok: false, error: { code: "model_load_failed", message: "Unable to allocate target model" } };
        nativeLoaded = "qwen3"; return { ok: true, data: "loaded" };
    }
    if (cmd === "get_backend_info") return "test CPU";
    if (cmd === "get_granite_status") return { loaded: nativeLoaded === "granite", model_id: "granite-test" };
    if (cmd === "get_qwen3_status") return { loaded: nativeLoaded === "qwen3", model_id: "qwen3-test" };
    throw new Error(`Unexpected ${cmd}`);
} }));
const { useEngineSwitch } = await import("../../src/hooks/useEngineSwitch");
const noop = () => {};
function fixture() {
    return renderHook(() => useEngineSwitch({
        models: [], graniteModels: [{ id: "granite-test" }], qwen3Models: [{ id: "qwen3-test" }],
        currentModel: null, currentGraniteModel: "granite-test", currentQwen3Model: "qwen3-test",
        setCurrentModel: noop, setCurrentGraniteModel: noop, setCurrentQwen3Model: noop,
        setBackendInfo: noop, setHeaderStatus: noop, setTrayState: async () => {},
        storeRef: { current: null }, asrBackend: "cpu", setAsrBackend: noop,
        isRecordingRef: { current: false }, downloadProgressRef: { current: {} },
    } as any));
}
beforeEach(() => { failQwen = false; nativeLoaded = null; calls.length = 0; });

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
