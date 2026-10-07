import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "../frontend/hookHarness";

type Call = { cmd: string; args?: Record<string, unknown> };
let calls: Call[] = [];
let responses: Map<string, () => unknown>;
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({ ...realCore, invoke: async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args });
    if (responses.has(cmd)) return responses.get(cmd)!();
    if (cmd === "check_grammar_llm_available") return true;
    if (cmd === "init_llm") return "loaded";
    return null;
} }));
const realEvent = { ...(await import("@tauri-apps/api/event")) };
mock.module("@tauri-apps/api/event", () => ({ ...realEvent, listen: async () => () => {} }));
const { usePostProcessing } = await import("../../src/hooks/usePostProcessing");

function deferred<T>() {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>(r => { resolve = r; });
    return { promise, resolve };
}
function fixture(getGate?: Promise<unknown>) {
    const settings: Record<string, unknown> = { enable_grammar_lm: true, llm_backend: "gpu" };
    const store = { get: async (key: string) => { const value = settings[key] ?? null; if (getGate && key === "llm_backend") await getGate; return value; },
        set: async (key: string, value: unknown) => { settings[key] = value; }, save: async () => {} };
    const h = renderHook(() => usePostProcessing(() => {}, undefined, { current: store } as any));
    return { ...h, settings };
}
beforeEach(() => { calls = []; responses = new Map(); });

test("re-enabling FlowScribe retries after a transient initial load failure", async () => {
    responses.set("init_llm", () => { throw "temporary native load failure"; });
    const h = fixture(); await h.settle(); expect(h.handle.current.llmStatus).toBe("Error");
    responses.delete("init_llm");
    h.handle.current.setEnableGrammarLM(false); await h.settle();
    h.handle.current.setEnableGrammarLM(true); await h.settle();
    expect(h.handle.current.llmStatus).toBe("Loaded");
    expect(calls.filter(c => c.cmd === "init_llm")).toHaveLength(2);
});

test("switching FlowScribe to CPU recovers a failed GPU reload", async () => {
    const h = fixture(); await h.settle();
    responses.set("init_llm", () => { throw "GPU out of memory"; });
    await h.handle.current.setComputeDevice("gpu:1"); await h.settle();
    expect(h.handle.current.llmStatus).toBe("Error");
    responses.delete("init_llm");
    h.handle.current.setLlmBackend("cpu"); await h.settle();
    expect(h.handle.current.llmStatus).toBe("Loaded");
    expect(calls.filter(c => c.cmd === "init_llm").at(-1)?.args?.useGpu).toBe(false);
});

test("disabling FlowScribe before availability returns prevents loading weights", async () => {
    const pending = deferred<boolean>(); responses.set("check_grammar_llm_available", () => pending.promise);
    const h = fixture(); await h.settle();
    h.handle.current.setEnableGrammarLM(false); await h.settle();
    pending.resolve(true); await h.settle();
    expect(calls.filter(c => c.cmd === "init_llm")).toHaveLength(0);
    expect(h.handle.current.enableGrammarLM).toBe(false);
});

test("turning FlowScribe off and on during unloading returns it to loaded state", async () => {
    const h = fixture(); await h.settle();
    const pending = deferred<null>(); responses.set("unload_llm", () => pending.promise);
    h.handle.current.setEnableGrammarLM(false); await h.settle();
    h.handle.current.setEnableGrammarLM(true); await h.settle();
    pending.resolve(null); await h.settle();
    expect(h.handle.current.enableGrammarLM).toBe(true);
    expect(h.handle.current.llmStatus).toBe("Loaded");
});

test("a rejected native GPU selection preserves the applied and saved preference", async () => {
    const h = fixture(); await h.settle();
    responses.set("set_compute_device", () => { throw "device unavailable"; });
    await expect(h.handle.current.setComputeDevice("missing-gpu")).rejects.toBe("device unavailable");
    await h.settle();
    expect(h.handle.current.computeDevice).toBe("auto");
    expect(h.settings.compute_device).toBeUndefined();
    expect(calls.filter(c => c.cmd === "unload_llm")).toHaveLength(0);
});

test("startup settings cannot overwrite a newer CPU selection", async () => {
    const pending = deferred<null>();
    const h = fixture(pending.promise); await h.settle();
    h.handle.current.setLlmBackend("cpu"); await h.settle();
    pending.resolve(null); await h.settle();
    expect(h.handle.current.llmBackend).toBe("cpu");
    expect(h.settings.llm_backend).toBe("cpu");
    expect(calls.filter(c => c.cmd === "init_llm").at(-1)?.args?.useGpu).toBe(false);
});
