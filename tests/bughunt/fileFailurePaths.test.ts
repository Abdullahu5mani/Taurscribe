import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "../frontend/hookHarness";
import { testTrace } from "./trace";
const trace = testTrace("file-failure-paths");
let lostSaveReply = 0;
let cancelFailure = false;
let pickerFailure = false;
let cancellationReply: Promise<unknown> | null = null;
let pickerReply: Promise<string | string[] | null> | null = null;
const savedRows = new Map<string, any>();
let unnamedSave = 0;

function deferred<T>() {
    let resolve!: (v: T) => void, reject!: (e: unknown) => void;
    const promise = new Promise<T>((a, b) => { resolve = a; reject = b; });
    return { promise, resolve, reject };
}
const calls: { cmd: string; args: any }[] = [];
const jobs: { args: any; resolve: (v: any) => void; reject: (e: unknown) => void }[] = [];
let historyFailure = false;
let historyPending: Promise<unknown> | null = null;
let pickerResult: string[] | string | null = null;
let registrationPending = false;
const registrations: (() => void)[] = [];
const progressListeners = new Set<(e: any) => void>();
const dropListeners = new Set<(e: any) => void>();
const frames = new Map<number, (time: number) => void>();
let nextFrame = 0;

const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({ ...realCore, invoke: async (cmd: string, args: any) => {
    calls.push({ cmd, args }); trace("invoke", { cmd, args });
    if (cmd === "transcribe_file") {
        const pending = deferred<any>(); jobs.push({ args, ...pending }); return pending.promise;
    }
    if (cmd === "save_transcript_history") {
        if (historyFailure) throw new Error("disk full");
        const key = args.jobId ?? `unidentified-${++unnamedSave}`;
        if (!savedRows.has(key)) savedRows.set(key, args);
        if (lostSaveReply-- > 0) throw new Error("reply lost after history commit");
        return historyPending;
    }
    if (cmd === "cancel_file_transcription") {
        if (cancelFailure) throw new Error("cancel command unavailable");
        return cancellationReply;
    }
    if (["finish_file_transcription_batch", "cancel_file_transcription"].includes(cmd)) return null;
    throw new Error(`Unexpected command ${cmd}`);
} }));
const register = async (set: Set<any>, fn: any) => {
    set.add(fn);
    if (registrationPending) await new Promise<void>(r => registrations.push(r));
    return () => set.delete(fn);
};
const realEvent = { ...(await import("@tauri-apps/api/event")) };
mock.module("@tauri-apps/api/event", () => ({ ...realEvent, listen: (name: string, fn: any) => {
    expect(name).toBe("file-transcription-progress"); return register(progressListeners, fn);
} }));
const realWebview = { ...(await import("@tauri-apps/api/webview")) };
mock.module("@tauri-apps/api/webview", () => ({ ...realWebview,
    getCurrentWebview: () => ({ onDragDropEvent: (fn: any) => register(dropListeners, fn) }),
}));
const realDialog = { ...(await import("@tauri-apps/plugin-dialog")) };
mock.module("@tauri-apps/plugin-dialog", () => ({ ...realDialog, open: async () => { if (pickerFailure) throw new Error("file picker unavailable"); return pickerReply ?? pickerResult; } }));
(globalThis as any).requestAnimationFrame = (fn: any) => { frames.set(++nextFrame, fn); return nextFrame; };
(globalThis as any).cancelAnimationFrame = (id: number) => frames.delete(id);

const { FileTranscriptionPanel } = await import("../../src/components/FileTranscriptionPanel");
const nodes = (root: any): any[] => root == null || typeof root !== "object" ? []
    : Array.isArray(root) ? root.flatMap(nodes) : [root, ...nodes(root.props?.children)];
const text = (root: any): string => root == null || typeof root === "boolean" ? ""
    : typeof root !== "object" ? String(root) : Array.isArray(root) ? root.map(text).join(" ").replace(/\s+%/g, "%") : text(root.props?.children);
const byLabel = (root: any, label: string) => nodes(root).find(n => n.props?.["aria-label"] === label);
const cards = (root: any) => nodes(root).filter(n => n.props?.role === "article");
const progress = (job: number, percent: number, status = "transcribing", id = jobs[job].args.jobId) => {
    for (const fn of progressListeners) fn({ payload: { path: jobs[job].args.path, job_id: id, percent, status } });
};
const flushFrames = () => { const pending = [...frames.values()]; frames.clear(); for (const fn of pending) fn(0); };
const drop = (...paths: string[]) => { for (const fn of dropListeners) fn({ payload: { type: "drop", paths } }); };
const result = (transcript = "Recognized file speech") => ({ transcript, audio_duration_ms: 12000, processing_time_ms: 300 });
function fixture(strictEffects = false) {
    let props: any = { activeEngine: "granite", currentGraniteModel: "granite-test", asrBackend: "cpu" };
    const processing: boolean[] = [];
    props.onFileProcessingChange = (v: boolean) => processing.push(v);
    const view = renderHook(() => (FileTranscriptionPanel as any).type(props), { strictEffects });
    return { ...view, processing, setProps: (next: any) => { props = { ...props, ...next }; view.rerender(); } };
}
beforeEach(() => {
    lostSaveReply = 0; cancelFailure = pickerFailure = false; savedRows.clear(); unnamedSave = 0;
    cancellationReply = pickerReply = null;
    calls.length = 0; jobs.length = 0; historyFailure = false; historyPending = null; pickerResult = null;
    registrationPending = false; registrations.length = 0; progressListeners.clear(); dropListeners.clear(); frames.clear();
});


test("retrying a save whose reply was lost cannot duplicate file history", async () => {
    trace("scenario", "lost-history-reply");
    lostSaveReply = 1;
    const h = fixture(); drop("/memo.wav"); await h.settle();
    jobs[0].resolve(result()); await h.settle();
    expect(savedRows.size).toBe(1);
    const retry = nodes(h.handle.current).find(n => n.props?.title === "Retry saving transcript to history");
    expect(retry).toBeDefined(); retry.props.onClick(); await h.settle();
    trace("final.save-retry", { rows: [...savedRows.values()], calls });
    expect(savedRows.size).toBe(1);
    expect(calls.filter(c => c.cmd === "save_transcript_history")[0].args.jobId).toBe(jobs[0].args.jobId);
    expect(calls.filter(c => c.cmd === "save_transcript_history")[1].args).toEqual(calls.filter(c => c.cmd === "save_transcript_history")[0].args);
});

test("a fresh rerun gets a distinct history row even when its text is identical", async () => {
    const h = fixture(); drop("/memo.wav"); await h.settle(); jobs[0].resolve(result()); await h.settle();
    nodes(h.handle.current).find(n => n.props?.["aria-label"]?.startsWith("Re-transcribe memo.wav")).props.onClick();
    await h.settle(); jobs[1].resolve(result()); await h.settle();
    expect(savedRows.size).toBe(2);
    const saves = calls.filter(c => c.cmd === "save_transcript_history");
    expect(saves[0].args.jobId).not.toBe(saves[1].args.jobId);
});

test("a failed file cancellation shows its error and keeps the active job retryable", async () => {
    cancelFailure = true;
    const h = fixture(); drop("/long.wav"); await h.settle();
    const cancel = nodes(h.handle.current).find(n => n.props?.["aria-label"]?.startsWith("Cancel transcription for"));
    cancel.props.onClick(); await h.settle();
    const shown = nodes(h.handle.current).some(n => n.props?.role === "alert" && text(n).includes("cancel command unavailable"));
    const busy = h.processing.at(-1);
    cancelFailure = false; cancel.props.onClick(); await h.settle();
    jobs[0].reject("Transcription cancelled"); await h.settle();
    trace("final.cancel-retry", { shown, busy, processing: h.processing, calls });
    expect(shown).toBe(true);
    expect(busy).toBe(true);
    expect(calls.filter(c => c.cmd === "cancel_file_transcription")).toHaveLength(2);
    expect(h.processing.at(-1)).toBe(false);
});

test("a rejected file picker is reported and a later selection still works", async () => {
    pickerFailure = true;
    const h = fixture();
    let unhandled: unknown = null;
    try { await byLabel(h.handle.current, "Browse audio files").props.onClick(); } catch (error) { unhandled = error; }
    await h.settle();
    const shown = nodes(h.handle.current).some(n => n.props?.role === "alert" && text(n).includes("file picker unavailable"));
    pickerFailure = false; pickerResult = "/ready.wav";
    await byLabel(h.handle.current, "Browse audio files").props.onClick(); await h.settle();
    jobs[0].resolve(result()); await h.settle();
    expect(unhandled).toBeNull(); expect(shown).toBe(true);
    expect(jobs).toHaveLength(1);
});

async function withClipboard(writeText: (value: string) => Promise<void>, work: () => Promise<void>) {
    const previous = Object.getOwnPropertyDescriptor(globalThis, "navigator");
    Object.defineProperty(globalThis, "navigator", { configurable: true, value: { clipboard: { writeText } } });
    try { await work(); } finally {
        if (previous) Object.defineProperty(globalThis, "navigator", previous); else delete (globalThis as any).navigator;
    }
}

test("file transcript copy failure is visible without altering the completed transcript", async () => {
    await withClipboard(async () => { throw new Error("clipboard unavailable"); }, async () => {
        const h = fixture(); drop("/copied.wav"); await h.settle(); jobs[0].resolve(result("Synthetic original words")); await h.settle();
        await byLabel(h.handle.current, "Copy transcript for copied.wav").props.onClick(); await h.settle();
        expect(nodes(h.handle.current).some(n => n.props?.role === "alert" && text(n).includes("clipboard unavailable"))).toBe(true);
        byLabel(h.handle.current, "Show transcript for copied.wav").props.onClick(); await h.settle();
        expect(text(h.handle.current)).toContain("Synthetic original words");
    });
});

test("a late cancellation failure cannot replace a newer successful request", async () => {
    const pending = deferred<unknown>(); cancellationReply = pending.promise;
    const h = fixture(); drop("/long.wav"); await h.settle();
    const cancel = nodes(h.handle.current).find(n => n.props?.["aria-label"]?.startsWith("Cancel transcription for"));
    const oldRequest = cancel.props.onClick();
    cancellationReply = null; await cancel.props.onClick(); await h.settle();
    pending.reject(new Error("obsolete cancel error")); await oldRequest; await h.settle();
    expect(text(h.handle.current)).not.toContain("obsolete cancel error");
    expect(h.processing.at(-1)).toBe(true);
    jobs[0].reject("Transcription cancelled"); await h.settle();
});

test("a late cancellation failure does not mark a completed transcript as failed", async () => {
    const pending = deferred<unknown>(); cancellationReply = pending.promise;
    const h = fixture(); drop("/finished.wav"); await h.settle();
    const request = nodes(h.handle.current).find(n => n.props?.["aria-label"]?.startsWith("Cancel transcription for")).props.onClick();
    jobs[0].resolve(result()); await h.settle();
    pending.reject(new Error("obsolete cancel error")); await request; await h.settle();
    expect(text(h.handle.current)).not.toContain("obsolete cancel error");
    expect(savedRows.size).toBe(1); expect(h.processing.at(-1)).toBe(false);
});

test("a late clipboard failure cannot erase a newer successful copy", async () => {
    const pending = deferred<void>(); let attempts = 0;
    await withClipboard(async () => { if (++attempts === 1) return pending.promise; }, async () => {
        const h = fixture(); drop("/copied.wav"); await h.settle(); jobs[0].resolve(result()); await h.settle();
        const copy = byLabel(h.handle.current, "Copy transcript for copied.wav");
        const oldRequest = copy.props.onClick(); await copy.props.onClick(); await h.settle();
        pending.reject(new Error("obsolete clipboard error")); await oldRequest; await h.settle();
        expect(text(h.handle.current)).not.toContain("obsolete clipboard error");
        expect(attempts).toBe(2);
    });
});

test("a late picker failure cannot replace a newer successful selection", async () => {
    const pending = deferred<string | null>(); pickerReply = pending.promise;
    const h = fixture(); const oldRequest = byLabel(h.handle.current, "Browse audio files").props.onClick();
    pickerReply = null; pickerResult = "/selected.wav";
    await byLabel(h.handle.current, "Browse audio files").props.onClick(); await h.settle();
    pending.reject(new Error("obsolete picker error")); await oldRequest; await h.settle();
    expect(text(h.handle.current)).not.toContain("obsolete picker error");
    expect(jobs).toHaveLength(1); jobs[0].resolve(result()); await h.settle();
});
