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

test("Cancel all keeps the app busy until its active native job has stopped", async () => {
    const h = fixture(); drop("/long.wav", "/waiting.wav"); await h.settle();
    expect(jobs.length).toBe(1);
    expect(h.processing.at(-1)).toBe(true);
    byLabel(h.handle.current, "Cancel all queued and active file transcriptions").props.onClick();
    await h.settle();
    expect(calls.filter(c => c.cmd === "cancel_file_transcription").map(c => c.args.path)).toEqual(["/long.wav"]);
    expect(calls.find(c => c.cmd === "cancel_file_transcription")?.args.jobId).toBe(jobs[0].args.jobId);
    const busyBeforeNativeExit = h.processing.at(-1);
    const finishedBeforeExit = calls.some(c => c.cmd === "finish_file_transcription_batch");
    jobs[0].reject("Transcription cancelled"); await h.settle();
    expect(jobs.length).toBe(1);
    expect(finishedBeforeExit).toBe(false);
    expect(h.processing.at(-1)).toBe(false);
    expect(busyBeforeNativeExit).toBe(true);
});

test("cancelling one file keeps the app busy while the native job drains", async () => {
    const h = fixture(); drop("/one.wav"); await h.settle();
    const cancel = nodes(h.handle.current).find(n => n.props?.["aria-label"]?.startsWith("Cancel transcription for"));
    expect(cancel).toBeDefined();
    cancel.props.onClick(); await h.settle();
    expect(h.processing.at(-1)).toBe(true);
    jobs[0].reject("Transcription cancelled"); await h.settle();
    expect(h.processing.at(-1)).toBe(false);
});

test("file cancellation targets the exact run, so a delayed request cannot cancel a rerun", async () => {
    const h = fixture(); drop("/repeat.wav"); await h.settle();
    const cancel = nodes(h.handle.current).find(n => n.props?.["aria-label"]?.startsWith("Cancel transcription for"));
    cancel.props.onClick(); await h.settle();
    const request = calls.find(c => c.cmd === "cancel_file_transcription")!;
    expect(request.args).toEqual({ path: "/repeat.wav", jobId: jobs[0].args.jobId });
    jobs[0].reject("Transcription cancelled"); await h.settle();
    const retry = byLabel(h.handle.current, "Transcribe repeat.wav");
    expect(retry).toBeDefined();
    retry.props.onClick(); await h.settle();
    expect(jobs[1].args.jobId).not.toBe(request.args.jobId);
});

test("rerunning a finished file has a fresh identity and ignores progress from its earlier run", async () => {
    const h = fixture(); drop("/again.wav"); await h.settle();
    const previousId = jobs[0].args.jobId;
    jobs[0].resolve(result()); await h.settle();
    const rerun = nodes(h.handle.current).find(n => n.props?.["aria-label"]?.startsWith("Re-transcribe again.wav"));
    rerun.props.onClick(); await h.settle();
    expect(jobs[1].args.jobId).not.toBe(previousId);
    for (const fn of progressListeners) fn({payload:{path:jobs[1].args.path,job_id:jobs[1].args.jobId,percent:30,status:"transcribing",backend:"CUDA0"}});
    flushFrames(); await h.settle();
    for (const fn of progressListeners) fn({payload:{path:jobs[0].args.path,job_id:previousId,percent:99,status:"finalizing",backend:"CPU"}});
    flushFrames(); await h.settle();
    expect(text(cards(h.handle.current)[0])).toContain("CUDA");
    expect(text(cards(h.handle.current)[0])).not.toContain("CPU");
    expect(text(cards(h.handle.current)[0])).not.toContain("Finishing…");
});
