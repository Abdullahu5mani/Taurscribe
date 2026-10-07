import { expect, mock, test } from "bun:test";
import { renderHook } from "./hookHarness";

let calls: string[] = [];
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({ ...realCore, invoke: async (cmd: string) => {
    calls.push(cmd);
    throw new Error("Busy capture must not query or load models");
} }));
const { useEngineSwitch } = await import("../../src/hooks/useEngineSwitch");

test("capture and enrollment busy state blocks all engine selectors, even empty model tabs", async () => {
    for (const installed of [true, false]) {
        calls = [];
        const noop = () => {};
        const h = renderHook(() => useEngineSwitch({
            models: installed ? [{ id: "whisper-test" }] : [],
            graniteModels: installed ? [{ id: "granite-test" }] : [],
            qwen3Models: installed ? [{ id: "qwen3-test" }] : [],
            currentModel: null, currentGraniteModel: null, currentQwen3Model: null,
            setCurrentModel: noop, setCurrentGraniteModel: noop, setCurrentQwen3Model: noop,
            setBackendInfo: noop, setHeaderStatus: noop, setTrayState: async () => {},
            storeRef: { current: null }, asrBackend: "gpu", setAsrBackend: noop,
            isRecordingRef: { current: true }, downloadProgressRef: { current: {} },
        } as any));
        await h.handle.current.handleModelChange("whisper-test");
        await h.handle.current.handleSwitchToWhisper();
        await h.handle.current.handleSwitchToGranite();
        await h.handle.current.handleSwitchToQwen3();
        await h.handle.current.handleToggleAsrBackend("cpu");
        await h.settle();
        expect(calls).toHaveLength(0);
        expect(h.handle.current.activeEngine).toBe("whisper");
        expect(h.handle.current.isLoading).toBe(false);
    }
});
