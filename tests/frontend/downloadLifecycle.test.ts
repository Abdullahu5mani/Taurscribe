test("duplicate done events and a successful command reply share one completion", async () => {
    let reply!: () => void, finish!: () => void, completed = 0;
    commandResult = new Promise<void>(resolve => { reply = resolve; });
    const callback = () => { completed++; return new Promise<void>(resolve => { finish = resolve; }); };
    const r = renderHook(() => useDownloads(callback)); await r.settle();
    const run = r.handle.current.handleDownload("whisper-base", "Base");
    emit("done", 100); emit("done", 100); await r.settle();
    expect(completed).toBe(1);
    reply(); finish(); await run; await r.settle(); emit("done", 100); await r.settle();
    expect(completed).toBe(1); expect(r.handle.current.downloadProgress["whisper-base"]).toBeUndefined();
});

test("a missed done event is recovered from the successful command reply", async () => {
    commandResult = Promise.resolve("installed"); let completed = 0;
    const r = renderHook(() => useDownloads(() => { completed++; })); await r.settle();
    await r.handle.current.handleDownload("whisper-base", "Base"); await r.settle();
    expect(completed).toBe(1); expect(r.handle.current.downloadProgress["whisper-base"]).toBeUndefined();
});

test("a throwing completion callback releases the download so it can be retried", async () => {
    commandResult = Promise.resolve("installed");
    const r = renderHook(() => useDownloads(() => { throw new Error("status refresh failed"); })); await r.settle();
    await r.handle.current.handleDownload("whisper-base", "Base"); await r.settle();
    await r.handle.current.handleDownload("whisper-base", "Base"); await r.settle();
    expect(calls.filter(c => c.command === "download_model")).toHaveLength(2);
    expect(r.handle.current.downloadProgress["whisper-base"]).toBeUndefined();
});

test("an old rejected command cannot clear the progress of a retry", async () => {
    let reject!: (error: Error) => void;
    commandResult = new Promise((_, fail) => { reject = fail; });
    const r = renderHook(() => useDownloads(() => {})); await r.settle();
    const first = r.handle.current.handleDownload("whisper-base", "Base");
    emit("error"); await r.settle();
    commandResult = new Promise(() => {});
    void r.handle.current.handleDownload("whisper-base", "Base"); await r.settle();
    reject(new Error("first attempt failed")); await first; await r.settle();
    expect(r.handle.current.downloadProgress["whisper-base"]?.status).toBe("starting");
});

test("a delayed accepted cancellation cannot mark the retry as cancelling", async () => {
    let accept!: (accepted: boolean) => void;
    cancelResult = new Promise(resolve => { accept = resolve; });
    const r = renderHook(() => useDownloads(() => {})); await r.settle();
    void r.handle.current.handleDownload("whisper-base", "Base"); await r.settle();
    const cancelled = r.handle.current.handleCancelDownload("whisper-base");
    emit("cancelled"); await r.settle();
    void r.handle.current.handleDownload("whisper-base", "Base"); await r.settle();
    accept(true); await cancelled; await r.settle();
    expect(r.handle.current.downloadProgress["whisper-base"]?.status).toBe("starting");
});
