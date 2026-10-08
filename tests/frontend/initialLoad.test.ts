import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "./hookHarness";

// Launch / restore: the whole useInitialLoad sequence against a fake backend
// and settings store — first launch, returning user, a saved model that is
// gone, unreadable settings, start hidden, a backend failure, and the window
// closing mid-startup.
type Calls = { cmd: string; args?: any }[];
let calls: Calls;
let saved: Record<string, unknown>;
let storeFails: boolean;
let backendFails: boolean;
let granite = [{ id: "granite-speech-5-nc" }, { id: "granite-speech-5-q4" }];
let engineState: any;
let holdLoad: Promise<void> | null;
let loadError: string | null;

const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({
    ...realCore,
    invoke: async (cmd: string, args?: any) => {
        calls.push({ cmd, args });
        switch (cmd) {
            case "get_backend_info": if (backendFails) throw "backend unavailable"; return "Metal";
            case "get_download_status": return [];
            case "list_models": return [{ id: "base.en" }, { id: "small.en" }];
            case "list_granite_models": return granite;
            case "list_qwen3_models": return [];
            case "get_engine_selection_state": return engineState;
            case "init_granite":
                if (holdLoad) await holdLoad;
                return loadError ? { ok: false, error: { code: "model_load_failed", message: loadError } } : { ok: true, data: "" };
            default: return null;
        }
    },
}));
const realStore = { ...(await import("@tauri-apps/plugin-store")) };
mock.module("@tauri-apps/plugin-store", () => ({
    ...realStore,
    Store: {
        load: async () => {
            if (storeFails) throw new Error("settings.json: expected value at line 1 column 1");
            return {
                get: async (key: string) => saved[key],
                set: async (key: string, value: unknown) => { saved[key] = value; },
                delete: async (key: string) => { delete saved[key]; },
                save: async () => {},
            };
        },
    },
}));
const { useInitialLoad } = await import("../../src/hooks/useInitialLoad");

let state: Record<string, unknown>;
let statuses: string[];
const params = () => {
    const set = (key: string) => (v: unknown) => { state[key] = v; };
    return {
        setModels: set("models"), setCurrentModel: set("currentModel"),
        setGraniteModels: set("graniteModels"), setCurrentGraniteModel: set("currentGranite"),
        setQwen3Models: set("qwen3Models"), setCurrentQwen3Model: set("currentQwen3"),
        setSettingsModels: () => {},
        setLoadedEngine: set("loadedEngine"), setActiveEngine: set("activeEngine"),
        activeEngineRef: { current: "whisper" as any }, isLoadingRef: { current: false },
        setIsLoading: set("isLoading"), setLoadingMessage: set("loadingMessage"),
        setBackendInfo: set("backend"), setHeaderStatus: (msg: string) => { statuses.push(msg); },
        setShowSetupWizard: set("showSetup"), setIsInitialLoading: set("initialLoading"),
        setCloseBehavior: set("closeBehavior"), storeRef: { current: null as any },
    };
};
const launch = async () => {
    const view = renderHook(() => useInitialLoad(params() as any));
    await view.settle();
    return view;
};
const invoked = (cmd: string) => calls.filter((c) => c.cmd === cmd);

beforeEach(() => {
    calls = [];
    saved = {};
    state = {};
    statuses = [];
    storeFails = false;
    backendFails = false;
    holdLoad = null;
    loadError = null;
    granite = [{ id: "granite-speech-5-nc" }, { id: "granite-speech-5-q4" }];
    engineState = { active_engine: "whisper", selected_model_id: null, loaded_engine: null, loaded_model_id: null };
});

test("a first launch opens the setup wizard and shows the window", async () => {
    await launch();
    expect(state.showSetup).toBe(true);
    expect(state.initialLoading).toBe(false);
    expect(invoked("show_main_window")).toHaveLength(1);
    expect(invoked("init_granite")).toHaveLength(0);
});

test("a returning user skips setup and gets settings and the saved Granite model back", async () => {
    saved = {
        setup_complete: true, active_engine: "granite", granite_model: "granite-speech-5-q4", asr_backend: "cpu",
        hotkey_binding: { keys: ["Control", "Alt"] }, input_device: "USB Mic", close_behavior: "quit",
    };
    await launch();
    expect(state.showSetup).toBe(false);
    expect(state.activeEngine).toBe("granite");
    expect(invoked("set_hotkey")[0].args).toEqual({ binding: { keys: ["Control", "Alt"] } });
    expect(invoked("set_input_device")[0].args).toEqual({ name: "USB Mic" });
    expect(state.closeBehavior).toBe("quit");
    expect(invoked("init_granite")[0].args).toMatchObject({ modelId: "granite-speech-5-q4", useGpu: false });
    expect(state.loadedEngine).toBe("granite");
    expect(state.isLoading).toBe(false);
});

test("a saved model that was deleted falls back to an installed one", async () => {
    saved = { setup_complete: true, active_engine: "granite", granite_model: "granite-removed" };
    await launch();
    expect(invoked("init_granite")[0].args).toMatchObject({ modelId: "granite-speech-5-nc" });
    expect(state.currentGranite).toBe("granite-speech-5-nc");

    calls = [];
    saved = { setup_complete: true, active_engine: "whisper", whisper_model: "large-v3-removed" };
    await launch();
    expect(state.currentModel).toBe("base.en");
});

test("a retired engine name is migrated instead of lost", async () => {
    saved = { setup_complete: true, active_engine: "parakeet", cohere_model: "x" };
    await launch();
    expect(saved.active_engine).toBe("granite");
    expect("cohere_model" in saved).toBe(false);
    expect(state.activeEngine).toBe("granite");
});

test("unreadable settings open setup instead of a broken app", async () => {
    storeFails = true;
    await launch();
    expect(state.showSetup).toBe(true);
    expect(invoked("show_main_window")).toHaveLength(1);
});

test("Start hidden keeps the window closed, but never before setup is done", async () => {
    saved = { setup_complete: true, start_hidden: true };
    await launch();
    expect(invoked("show_main_window")).toHaveLength(0);

    calls = [];
    saved = { start_hidden: true };
    await launch();
    expect(invoked("show_main_window")).toHaveLength(1);
});

test("a failed model load is reported and leaves the app usable", async () => {
    saved = { setup_complete: true, active_engine: "granite" };
    granite = [{ id: "granite-speech-5-nc" }];
    loadError = "model file is corrupt";
    await launch();
    expect(statuses).toContain("Failed to auto-load Granite: model file is corrupt");
    expect(state.isLoading).toBe(false);
    expect(state.loadedEngine).toBeUndefined();
    expect(invoked("show_main_window")).toHaveLength(1);
});

test("a backend that cannot start still shows the window with the error", async () => {
    backendFails = true;
    await launch();
    expect(state.backend).toBe("Unknown");
    expect(statuses[0]).toContain("Error loading models: backend unavailable");
    expect(state.initialLoading).toBe(false);
    expect(invoked("show_main_window")).toHaveLength(1);
});

test("closing during startup stops the sequence: no late state, no window", async () => {
    saved = { setup_complete: true, active_engine: "granite" };
    let release!: () => void;
    holdLoad = new Promise((r) => { release = r; });
    const view = renderHook(() => useInitialLoad(params() as any));
    for (let i = 0; i < 20 && invoked("init_granite").length === 0; i++) await new Promise((r) => setTimeout(r, 0));
    expect(state.isLoading).toBe(true);
    view.unmount();
    const before = { ...state };
    release();
    for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
    expect(state).toEqual(before);
    expect(invoked("show_main_window")).toHaveLength(0);
});
