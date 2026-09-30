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
