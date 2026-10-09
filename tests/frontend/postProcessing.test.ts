import { expect, mock, test } from "bun:test";
import { renderHook } from "./hookHarness";

type Call = { cmd: string; args?: Record<string, unknown> };
let calls: Call[] = [];
let unloaded: (() => void) | undefined;
let initGate: Promise<void> = Promise.resolve();

const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({
    ...realCore,
    invoke: async (cmd: string, args?: Record<string, unknown>) => {
        calls.push({ cmd, args });
        if (cmd === "check_grammar_llm_available") return true;
        if (cmd === "init_llm") {
            await initGate;
            return "FlowScribe loaded";
        }
        return null;
    },
}));
const realEvent = { ...(await import("@tauri-apps/api/event")) };
mock.module("@tauri-apps/api/event", () => ({ ...realEvent, listen: async (name: string, callback: () => void) => { if (name === "grammar-llm-unloaded") unloaded = callback; return () => {}; } }));

const { usePostProcessing } = await import("../../src/hooks/usePostProcessing");

function start(settings: Record<string, unknown>) {
    calls = [];
    const store = {
        get: async (k: string) => settings[k] ?? null,
        set: async (k: string, v: unknown) => { settings[k] = v; },
        save: async () => {},
    };
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    return renderHook(() => usePostProcessing(() => {}, undefined, { current: store } as any));
}

const llmCalls = () =>
    calls
        .filter((c) => c.cmd === "init_llm" || c.cmd === "unload_llm")
        .map((c) => (c.cmd === "unload_llm" ? "unload" : `init:${c.args?.useGpu ? "gpu" : "cpu"}`));

test("FlowScribe can stay on an iGPU while speech changes GPUs", async () => {
    const settings = { enable_grammar_lm: true, llm_backend: "gpu", compute_device: "cuda:RTX", llm_compute_device: "vulkan:iGPU" };
    const { handle, settle } = start(settings);
    await settle();
    expect(calls.find(c => c.cmd === "init_llm")?.args?.deviceId).toBe("vulkan:iGPU");
    await handle.current.setComputeDevice("cuda:other"); await settle();
    expect(llmCalls()).toEqual(["init:gpu"]);
    expect(handle.current.llmComputeDevice).toBe("vulkan:iGPU");
});

test("changing only cleanup hardware reloads FlowScribe without changing speech hardware", async () => {
    const settings: Record<string, unknown> = { enable_grammar_lm: true, llm_backend: "gpu", compute_device: "cuda:RTX" };
    const { handle, settle } = start(settings);
    await settle();
    handle.current.setLlmComputeDevice("vulkan:iGPU"); await settle();
    expect(llmCalls()).toEqual(["init:gpu", "unload", "init:gpu"]);
    expect(calls.filter(c => c.cmd === "init_llm").at(-1)?.args?.deviceId).toBe("vulkan:iGPU");
    expect(calls.some(c => c.cmd === "set_compute_device")).toBe(false);
    expect(handle.current.computeDevice).toBe("cuda:RTX");
    expect(settings.llm_compute_device).toBe("vulkan:iGPU");
    handle.current.setLlmComputeDevice("same"); await settle();
    expect(calls.filter(c => c.cmd === "init_llm").at(-1)?.args?.deviceId).toBe("cuda:RTX");
});

test("the cleanup CPU choice persists without moving speech off its GPU", async () => {
    const settings: Record<string, unknown> = { enable_grammar_lm: true, llm_backend: "gpu", compute_device: "cuda:RTX" };
    const { handle, settle } = start(settings);
    await settle();
    handle.current.setLlmComputeDevice("cpu"); await settle();
    expect(llmCalls()).toEqual(["init:gpu", "unload", "init:cpu"]);
    expect(settings.llm_backend).toBe("cpu");
    expect(settings.llm_compute_device).toBe("cpu");
    expect(handle.current.computeDevice).toBe("cuda:RTX");
});

test("switching FlowScribe to the CPU reloads it on the CPU", async () => {
    const { handle, settle } = start({ enable_grammar_lm: true, llm_backend: "gpu" });
    await settle();
    expect(handle.current.llmStatus).toBe("Loaded");
    expect(llmCalls()).toEqual(["init:gpu"]);

    handle.current.setLlmBackend("cpu");
    await settle();
    expect(llmCalls()).toEqual(["init:gpu", "unload", "init:cpu"]);
    expect(handle.current.llmStatus).toBe("Loaded");
});

test("a switch made while FlowScribe is loading applies once it has loaded", async () => {
    let release!: () => void;
    initGate = new Promise((r) => { release = r; });
    const { handle, settle } = start({ enable_grammar_lm: true, llm_backend: "gpu" });
    await settle();
    expect(handle.current.llmStatus).toBe("Loading...");

    handle.current.setLlmBackend("cpu");
    await settle();
    expect(llmCalls()).toEqual(["init:gpu"]);

    initGate = Promise.resolve();
    release();
    await settle();
    expect(llmCalls()).toEqual(["init:gpu", "unload", "init:cpu"]);
});

test("the saved backend is used from the start, without a reload", async () => {
    const { handle, settle } = start({ enable_grammar_lm: true, llm_backend: "cpu" });
    await settle();
    expect(handle.current.llmStatus).toBe("Loaded");
    expect(llmCalls()).toEqual(["init:cpu"]);
});

test("a storage move unloading FlowScribe triggers reload on the selected backend", async () => {
    const { handle, settle } = start({ enable_grammar_lm: true, llm_backend: "cpu" });
    await settle();
    expect(handle.current.llmStatus).toBe("Loaded");
    unloaded?.();
    await settle();
    expect(llmCalls()).toEqual(["init:cpu", "init:cpu"]);
    expect(handle.current.llmStatus).toBe("Loaded");
});


test("changing the GPU reloads FlowScribe after the backend applies the device", async () => {
    const { handle, settle } = start({ enable_grammar_lm: true, llm_backend: "gpu", compute_device: "gpu:0" });
    await settle();
    await handle.current.setComputeDevice("gpu:1"); await settle();
    expect(handle.current.computeDevice).toBe("gpu:1");
    expect(llmCalls()).toEqual(["init:gpu", "unload", "init:gpu"]);
    const selected = calls.findIndex(c => c.cmd === "set_compute_device");
    expect(selected).toBeLessThan(calls.findIndex(c => c.cmd === "unload_llm"));
    await handle.current.setComputeDevice("gpu:1"); await settle();
    expect(llmCalls()).toEqual(["init:gpu", "unload", "init:gpu"]);
});

test("a GPU change leaves CPU FlowScribe resident until GPU mode is selected", async () => {
    const { handle, settle } = start({ enable_grammar_lm: true, llm_backend: "cpu" });
    await settle(); await handle.current.setComputeDevice("gpu:1"); await settle();
    expect(llmCalls()).toEqual(["init:cpu"]);
    handle.current.setLlmBackend("gpu"); await settle();
    expect(llmCalls()).toEqual(["init:cpu", "unload", "init:gpu"]);
});

test("a GPU change during FlowScribe loading is applied after that load completes", async () => {
    let release!: () => void;
    initGate = new Promise(r => { release = r; });
    const { handle, settle } = start({ enable_grammar_lm: true, llm_backend: "gpu" });
    await settle(); await handle.current.setComputeDevice("gpu:1"); await settle();
    expect(llmCalls()).toEqual(["init:gpu"]);
    initGate = Promise.resolve(); release(); await settle();
    expect(llmCalls()).toEqual(["init:gpu", "unload", "init:gpu"]);
});
