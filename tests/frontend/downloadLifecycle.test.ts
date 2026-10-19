import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "./hookHarness";

const listeners = new Set<(event: any) => void>();
const registrations: (() => void)[] = [];
const calls: { command: string; args: any }[] = [];
const errors: string[] = [];
let delayRegistration = false;
let commandResult: Promise<unknown>;
let cancelResult: Promise<boolean>;
const realEvent = { ...(await import("@tauri-apps/api/event")) };
mock.module("@tauri-apps/api/event", () => ({ ...realEvent, listen: async (_name: string, cb: (event: any) => void) => {
    listeners.add(cb);
    if (delayRegistration) await new Promise<void>(resolve => registrations.push(resolve));
    return () => listeners.delete(cb);
} }));
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({ ...realCore, invoke: async (command: string, args: any) => {
    calls.push({ command, args });
    if (command === "download_model") return commandResult;
    if (command === "cancel_download") return cancelResult;
    throw new Error(command);
} }));
// Avoid loading Sonner's CSS injector into other tests' partial document fixtures.
mock.module("sonner", () => ({ Toaster: () => null, toast: { info: () => {}, success: () => {}, error: (message: string) => errors.push(message) } }));
const { useDownloads } = await import("../../src/hooks/useDownloads");
const emit = (status: string, bytes = 0, requestId = calls.filter(c => c.command === "download_model").at(-1)?.args.requestId, error?: string) => {
    for (const cb of listeners) cb({ payload: { model_id: "whisper-base", request_id: requestId, error, status, total_bytes: 100, downloaded_bytes: bytes } });
};
beforeEach(() => {
    listeners.clear(); registrations.length = 0; calls.length = 0; delayRegistration = false;
    errors.length = 0;
    commandResult = new Promise(() => {}); cancelResult = Promise.resolve(true);
});

test("a terminal error event retains the native failure reason and reports it once", async () => {
    let reject!: (error: Error) => void;
    commandResult = new Promise((_, fail) => { reject = fail; });
    const view = renderHook(() => useDownloads(() => {})); await view.settle();
    const pending = view.handle.current.handleDownload("whisper-base", "Base");
    emit("error", 0, undefined, "No space left on device"); await view.settle();
    reject(new Error("No space left on device")); await pending; await view.settle();
    expect(errors).toHaveLength(1);
    expect(errors[0]).toContain("No space left on device");
    expect(view.handle.current.downloadProgress["whisper-base"]).toBeUndefined();
    view.unmount();
});

test("a throwing failure callback cannot prevent progress cleanup or retry", async () => {
    commandResult = Promise.reject(new Error("connection lost"));
    const view = renderHook(() => useDownloads(() => {}, () => { throw new Error("refresh failed"); }));
    const pending = view.handle.current.handleDownload("whisper-base", "Base");
    await pending; await view.settle();
    expect(view.handle.current.downloadProgress["whisper-base"]).toBeUndefined();
    commandResult = new Promise(() => {});
    void view.handle.current.handleDownload("whisper-base", "Base"); await view.settle();
    expect(calls.filter(c => c.command === "download_model")).toHaveLength(2);
    await view.handle.current.handleCancelDownload("whisper-base");
    expect(calls.find(c => c.command === "cancel_download")?.args.requestId).toBe(calls.filter(c => c.command === "download_model").at(-1)?.args.requestId);
    view.unmount();
});

test("progress delivered after the failure reply cannot resurrect a failed download", async () => {
    let reject!: (error: Error) => void;
    commandResult = new Promise((_, fail) => { reject = fail; });
    const view = renderHook(() => useDownloads(() => {})); await view.settle();
    const pending = view.handle.current.handleDownload("whisper-base", "Base");
    reject(new Error("connection lost")); await pending; await view.settle();
    emit("downloading", 5); await view.settle();
    expect(view.handle.current.downloadProgress["whisper-base"]).toBeUndefined();
    commandResult = new Promise(() => {});
    void view.handle.current.handleDownload("whisper-base", "Base"); await view.settle();
    expect(calls.filter(c => c.command === "download_model")).toHaveLength(2);
    view.unmount();
});

for (const status of ["error", "cancelled", "done", "downloading"]) {
    test(`a delayed ${status} event from a failed download cannot alter its retry`, async () => {
        let reject!: (error: Error) => void;
        commandResult = new Promise((_, fail) => { reject = fail; });
        let completed = 0, failed = 0;
        const view = renderHook(() => useDownloads(() => { completed++; }, () => { failed++; }));
        await view.settle();
        const first = view.handle.current.handleDownload("whisper-base", "Base");
        const firstId = calls.find(c => c.command === "download_model")!.args.requestId ?? "old-attempt";
        reject(new Error("connection lost")); await first; await view.settle();
        commandResult = new Promise(() => {});
        void view.handle.current.handleDownload("whisper-base", "Base"); await view.settle();
        emit("downloading", 12); await view.settle();
        emit(status, 99, firstId); await view.settle();
        expect(view.handle.current.downloadProgress["whisper-base"]).toMatchObject({ status: "downloading", bytes: 12 });
        expect(completed).toBe(0); expect(failed).toBe(1);
        view.unmount();
    });
}

test("StrictMode replay disposes late listeners and completes a download once", async () => {
    delayRegistration = true;
    let completed = 0;
    const r = renderHook(() => useDownloads(() => { completed++; }), { strictEffects: true });
    registrations.forEach(resolve => resolve()); await r.settle();
    expect(listeners.size).toBe(1);
    emit("done", 100); await r.settle();
    expect(completed).toBe(1);
    r.unmount(); expect(listeners.size).toBe(0);
});

test("unmount before listener registration removes it and suppresses callbacks", async () => {
    delayRegistration = true;
    let completed = 0;
    const r = renderHook(() => useDownloads(() => { completed++; }));
    r.unmount(); emit("done", 100);
    registrations.forEach(resolve => resolve()); await r.settle();
    expect(completed).toBe(0); expect(listeners.size).toBe(0);
});

async function withClock(run: (advance: (ms: number) => void, tick: () => void) => Promise<void>) {
    const realNow = Date.now, realSet = globalThis.setInterval, realClear = globalThis.clearInterval;
    let now = 1_000_000;
    let tick: (() => void) | undefined;
    Date.now = () => now;
    (globalThis as any).setInterval = (cb: () => void) => { tick = cb; return 777; };
    (globalThis as any).clearInterval = () => {};
    try { await run(ms => { now += ms; }, () => tick!()); }
    finally { Date.now = realNow; globalThis.setInterval = realSet; globalThis.clearInterval = realClear; }
}

for (const phase of ["extracting", "verifying", "copying"]) {
    test(`healthy ${phase} lasting over 35 seconds is not cancelled as a network stall`, async () => {
        await withClock(async (advance, tick) => {
            const r = renderHook(() => useDownloads(() => {}));
            try {
                await r.settle(); void r.handle.current.handleDownload("whisper-base", "Base"); await r.settle();
                emit(phase); await r.settle(); advance(36_000); tick(); await r.settle();
                expect(calls.filter(c => c.command === "cancel_download")).toHaveLength(0);
            } finally { r.unmount(); }
        });
    });
}

test("a stalled network request is still cancelled, while slow byte progress keeps it alive", async () => {
    await withClock(async (advance, tick) => {
        const r = renderHook(() => useDownloads(() => {}));
        try {
            await r.settle(); void r.handle.current.handleDownload("whisper-base", "Base"); await r.settle();
            for (let i = 1; i <= 5; i++) { advance(20_000); emit("downloading", i); tick(); await r.settle(); }
            expect(calls.filter(c => c.command === "cancel_download")).toHaveLength(0);
            advance(36_000); tick(); await r.settle();
            expect(calls.filter(c => c.command === "cancel_download")).toHaveLength(1);
        } finally { r.unmount(); }
    });
});

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
