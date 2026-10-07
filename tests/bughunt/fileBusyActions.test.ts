import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "../frontend/hookHarness";

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
    calls.push({ cmd, args });
    if (cmd === "transcribe_file") {
        const pending = deferred<any>(); jobs.push({ args, ...pending }); return pending.promise;
    }
    if (cmd === "save_transcript_history") {
        if (historyFailure) throw new Error("disk full");
        return historyPending;
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
mock.module("@tauri-apps/plugin-dialog", () => ({ ...realDialog, open: async () => pickerResult }));
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
    calls.length = 0; jobs.length = 0; historyFailure = false; historyPending = null; pickerResult = null;
    registrationPending = false; registrations.length = 0; progressListeners.clear(); dropListeners.clear(); frames.clear();
});

test("voice enrollment blocks Re-run without losing the completed transcript", async () => {
    const h = fixture(); drop("/saved.wav"); await h.settle();
    jobs[0].resolve(result("Original completed transcript")); await h.settle();
    byLabel(h.handle.current, "Show transcript for saved.wav").props.onClick(); await h.settle();
    expect(text(h.handle.current)).toContain("Original completed transcript");
    h.setProps({ isVoiceEnrolling: true });
    const rerun = nodes(h.handle.current).find(n => n.props?.["aria-label"]?.startsWith("Re-transcribe saved.wav"));
    if (!rerun.props.disabled) await rerun.props.onClick();
    await h.settle();
    const jobCount = jobs.length;
    const visible = text(h.handle.current);
    if (jobs[1]) { jobs[1].resolve(result()); await h.settle(); }
    expect(jobCount).toBe(1);
    expect(visible).toContain("Original completed transcript");
});

test("model loading blocks Retry on an earlier failed file", async () => {
    const h = fixture(); drop("/failed.wav"); await h.settle();
    jobs[0].reject("temporary inference failure"); await h.settle();
    h.setProps({ isModelLoading: true });
    const retry = byLabel(h.handle.current, "Retry transcribing failed.wav");
    if (!retry.props.disabled) await retry.props.onClick();
    await h.settle();
    const jobCount = jobs.length;
    if (jobs[1]) { jobs[1].resolve(result()); await h.settle(); }
    expect(jobCount).toBe(1);
});
