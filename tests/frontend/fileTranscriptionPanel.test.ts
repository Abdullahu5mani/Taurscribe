import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "./hookHarness";

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
const progress = (job: number, percent: number, status = "transcribing", id = jobs[job].args.jobId, timing: Record<string, any> = {}) => {
    for (const fn of progressListeners) fn({ payload: { path: jobs[job].args.path, job_id: id, percent, status, ...timing } });
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

test("hidden Files panel does not consume meeting recording drops", async () => {
    const view = fixture(); await view.settle();
    view.setProps({ isVisible: false });
    drop("/audio/meeting.wav"); await view.settle();
    expect(jobs).toHaveLength(0);
    view.setProps({ isVisible: true });
    drop("/audio/dictation.wav"); await view.settle();
    expect(jobs).toHaveLength(1);
});

test("picker preserves original path, chosen engine/model and timing in text-only history", async () => {
    const h = fixture(); pickerResult = ["/Users/test/Interview.WAV"];
    await byLabel(h.handle.current, "Browse audio files").props.onClick(); await h.settle();
    expect(jobs[0].args).toMatchObject({ path: pickerResult[0], expectedEngine: "granite", expectedModelId: "granite-test", useGpu: false, deferAutoUnload: true });
    jobs[0].resolve(result()); await h.settle();
    expect(calls.find(c => c.cmd === "save_transcript_history")?.args).toMatchObject({ sourcePath: pickerResult[0], importMethod: "picker", audioSource: "Interview.WAV", engine: "granite", modelId: "granite-test", durationMs: 12000, processingTimeMs: 300 });
    expect(calls.filter(c => /copy|delete|remove.*audio/.test(c.cmd))).toHaveLength(0);
});

test("OS drop handles mixed extensions and Windows paths without transcribing unsupported files", async () => {
    const h = fixture(); drop("C:\\Calls\\brief.MP3", "/notes.txt", "/clip.FLAC"); await h.settle();
    expect(cards(h.handle.current)).toHaveLength(2);
    expect(jobs[0].args.path).toBe("C:\\Calls\\brief.MP3");
    jobs[0].resolve(result()); await h.settle();
    expect(calls.find(c => c.cmd === "save_transcript_history")?.args).toMatchObject({ importMethod: "drop", audioSource: "brief.MP3", sourcePath: "C:\\Calls\\brief.MP3" });
    jobs[1].resolve(result()); await h.settle();
});

test("model loading blocks OS drops, including events after a prop change", async () => {
    const h = fixture(); h.setProps({ isModelLoading: true }); drop("/blocked.wav"); await h.settle();
    expect(jobs).toHaveLength(0); expect(text(h.handle.current)).toContain("Loading model");
    h.setProps({ isModelLoading: false }); drop("/ready.wav"); await h.settle(); expect(jobs).toHaveLength(1);
});

test("queue runs one file at a time and releases the batch only after the last file", async () => {
    const h = fixture(); drop("/a.wav", "/b.wav", "/c.wav"); await h.settle(); expect(jobs).toHaveLength(1);
    for (let n = 0; n < 3; n++) {
        expect(jobs[n].args.path).toBe(`/${"abc"[n]}.wav`);
        expect(calls.filter(c => c.cmd === "finish_file_transcription_batch")).toHaveLength(0);
        jobs[n].resolve(result()); await h.settle();
    }
    expect(jobs).toHaveLength(3); expect(cards(h.handle.current).every(c => c.props.className.includes("--done"))).toBe(true);
    expect(calls.filter(c => c.cmd === "finish_file_transcription_batch")).toHaveLength(1);
    expect(h.processing.at(-1)).toBe(false);
});

test("same-path jobs ignore old, missing and foreign job identifiers", async () => {
    const h = fixture(); drop("/repeat.wav", "/repeat.wav"); await h.settle();
    jobs[0].resolve(result()); await h.settle();
    progress(1, 90, "transcribing", jobs[0].args.jobId); progress(1, 95, "transcribing", null as any);
    for (const fn of progressListeners) fn({ payload: { path: "/other.wav", job_id: jobs[1].args.jobId, percent: 95 } });
    flushFrames(); await h.settle();
    expect(text(cards(h.handle.current)[1])).toContain("Decoding…");
    progress(1, 70); flushFrames(); await h.settle(); expect(text(cards(h.handle.current)[1])).toContain("Transcribing…");
});

test("the pipeline does not display fabricated percentages", async () => {
    const h = fixture(); drop("/progress.wav"); await h.settle();
    progress(0, 90); progress(0, 20); flushFrames(); await h.settle();
    expect(text(cards(h.handle.current)[0])).not.toContain("%");
});

test("done progress does not expose incomplete results or unlock removal before the command returns", async () => {
    const h = fixture(); drop("/tail.wav"); await h.settle(); progress(0, 100, "done"); flushFrames(); await h.settle();
    expect(cards(h.handle.current)[0].props.className).toContain("--processing");
    expect(byLabel(h.handle.current, "Remove tail.wav").props.disabled).toBe(true);
    jobs[0].resolve(result()); await h.settle(); expect(cards(h.handle.current)[0].props.className).toContain("--done");
});

test("live speed uses measured chunks, survives stale events, and resets between queued files", async () => {
    const h = fixture(); drop("/first.wav", "/second.wav"); await h.settle();
    expect(text(cards(h.handle.current)[0])).toContain("Measuring speech speed");
    progress(0, 53, "transcribing", jobs[0].args.jobId, { transcribed_audio_ms: 60000, transcription_time_ms: 4000 });
    flushFrames(); await h.settle();
    expect(text(cards(h.handle.current)[0])).toContain("15.0× realtime");
    // Timing changes even while the pipeline's percent remains unchanged.
    progress(0, 53, "transcribing", jobs[0].args.jobId, { transcribed_audio_ms: 90000, transcription_time_ms: 9000 });
    progress(0, 53, "transcribing", jobs[0].args.jobId, { transcribed_audio_ms: 60000, transcription_time_ms: 4000 });
    flushFrames(); await h.settle();
    expect(text(cards(h.handle.current)[0])).toContain("10.0× realtime");
    progress(0, 53, "transcribing", "stale-job", { transcribed_audio_ms: 900000, transcription_time_ms: 1 });
    progress(0, 53, "transcribing", jobs[0].args.jobId, { transcribed_audio_ms: 90000, transcription_time_ms: 0 });
    flushFrames(); await h.settle();
    expect(text(cards(h.handle.current)[0])).toContain("10.0× realtime");
    jobs[0].resolve(result()); await h.settle();
    expect(text(cards(h.handle.current)[0])).toMatch(/40\.0×\s+overall speed/);
    expect(text(cards(h.handle.current)[1])).toContain("Measuring speech speed");
    expect(text(cards(h.handle.current)[1])).not.toContain("10.0×");
    jobs[1].resolve(result()); await h.settle();
});

test("file phases do not claim a speed during model loading or decoding", async () => {
    const h = fixture(); drop("/load.wav"); await h.settle();
    progress(0, 0, "loading model"); flushFrames(); await h.settle();
    expect(text(h.handle.current)).toContain("Loading model…");
    expect(text(h.handle.current)).not.toContain("× realtime");
    progress(0, 20, "decoding"); flushFrames(); await h.settle();
    expect(text(h.handle.current)).toContain("Decoding…");
    jobs[0].resolve(result()); await h.settle();
});

test("history failure preserves visible transcript and retry saves it without new inference", async () => {
    const h = fixture(); historyFailure = true; drop("/keep.wav"); await h.settle(); jobs[0].resolve(result()); await h.settle();
    byLabel(h.handle.current, "Show transcript for keep.wav").props.onClick(); await h.settle(); expect(text(h.handle.current)).toContain("Recognized file speech");
    historyFailure = false;
    const retry = nodes(h.handle.current).find(n => n.props?.title === "Retry saving transcript to history");
    retry.props.onClick(); await h.settle(); expect(jobs).toHaveLength(1);
    expect(calls.filter(c => c.cmd === "save_transcript_history")).toHaveLength(2);
    expect(text(h.handle.current)).not.toContain("disk full");
});

test("two retry-save clicks from the same render cannot create duplicate history rows", async () => {
    const h = fixture(); historyFailure = true; drop("/retry.wav"); await h.settle(); jobs[0].resolve(result()); await h.settle();
    historyFailure = false; const pending = deferred<unknown>(); historyPending = pending.promise;
    const retry = nodes(h.handle.current).find(n => n.props?.title === "Retry saving transcript to history");
    retry.props.onClick(); retry.props.onClick(); await h.settle();
    const count = calls.filter(c => c.cmd === "save_transcript_history").length;
    pending.resolve(null); await h.settle(); expect(count).toBe(2);
});

test("saving history blocks Re-run and removal until persistence settles", async () => {
    const h = fixture(); const pending = deferred<unknown>(); historyPending = pending.promise;
    drop("/save.wav"); await h.settle(); jobs[0].resolve(result()); await h.settle();
    expect(byLabel(h.handle.current, "Remove save.wav").props.disabled).toBe(true);
    expect(nodes(h.handle.current).find(n => n.props?.["aria-label"]?.startsWith("Re-transcribe save.wav"))?.props.disabled).toBe(true);
    pending.resolve(null); await h.settle(); expect(byLabel(h.handle.current, "Remove save.wav").props.disabled).toBe(false);
});

test("failed and cancelled files do not block the next queued file or write history", async () => {
    const h = fixture(); drop("/failure.wav", "/cancel.wav", "/good.wav"); await h.settle();
    jobs[0].reject(new Error("corrupt audio")); await h.settle();
    jobs[1].reject(new Error("Transcription cancelled")); await h.settle();
    jobs[2].resolve(result()); await h.settle();
    expect(cards(h.handle.current).map(c => c.props.className)).toEqual(["file-card file-card--error", "file-card file-card--cancelled", "file-card file-card--done"]);
    expect(calls.filter(c => c.cmd === "save_transcript_history")).toHaveLength(1);
});

test("empty recognized text does not create a history entry", async () => {
    const h = fixture(); drop("/silence.wav"); await h.settle(); jobs[0].resolve(result("   ")); await h.settle();
    expect(calls.filter(c => c.cmd === "save_transcript_history")).toHaveLength(0);
});

test("changing models preserves queued model identity; explicit Re-run takes the new choice", async () => {
    const h = fixture(); drop("/a.wav", "/b.wav"); await h.settle();
    h.setProps({ activeEngine: "qwen3", currentQwen3Model: "qwen-test", asrBackend: "gpu" });
    jobs[0].resolve(result()); await h.settle();
    expect(jobs[1].args).toMatchObject({ expectedEngine: "granite", expectedModelId: "granite-test" });
    jobs[1].reject(new Error("selected model changed")); await h.settle();
    byLabel(h.handle.current, "Retry transcribing b.wav").props.onClick(); await h.settle();
    expect(jobs[2].args).toMatchObject({ expectedEngine: "qwen3", expectedModelId: "qwen-test", useGpu: true });
});

test("progress and drag subscriptions arriving after unmount are both removed", async () => {
    registrationPending = true; const h = fixture(); h.unmount();
    for (const done of registrations) done(); await h.settle();
    expect(progressListeners.size).toBe(0); expect(dropListeners.size).toBe(0);
});

test("effect replay retains exactly one progress and one drag subscription", async () => {
    const h = fixture(true); await h.settle();
    expect(progressListeners.size).toBe(1); expect(dropListeners.size).toBe(1);
    h.unmount(); await h.settle(); expect(progressListeners.size).toBe(0); expect(dropListeners.size).toBe(0);
});

test("unmount cancels a scheduled progress frame and prevents queued inference after current work", async () => {
    const h = fixture(); drop("/current.wav", "/later.wav"); await h.settle(); progress(0, 60);
    h.unmount(); await h.settle(); expect(frames.size).toBe(0);
    jobs[0].resolve(result()); await h.settle(); expect(jobs).toHaveLength(1);
});


test("voice setup blocks file drops and gives a truthful busy message", async () => {
    const h = fixture(); h.setProps({ isVoiceEnrolling: true });
    drop("/blocked.wav"); await h.settle();
    expect(jobs).toHaveLength(0);
    expect(text(h.handle.current)).toContain("Voice setup in progress");
    expect(text(h.handle.current)).not.toContain("Loading model");
    h.setProps({ isVoiceEnrolling: false }); drop("/ready.wav"); await h.settle();
    expect(jobs).toHaveLength(1);
    jobs[0].resolve(result()); await h.settle();
});

test("streamed steps stay active until the whole file finishes and saving has its own step", async () => {
    const h = fixture(); drop("/steps.wav"); await h.settle();
    const step = (key: string) => nodes(cards(h.handle.current)[0]).find(n => n.props?.["data-step"] === key);
    progress(0, 83, "transcribing", jobs[0].args.jobId, {
        steps_started: ["open", "convert", "speech", "recognize"], steps_completed: ["model", "open"], backend: "CUDA0",
        transcribed_audio_ms: 16000, transcription_time_ms: 1000,
    }); flushFrames(); await h.settle();
    expect(step("open").props["data-state"]).toBe("complete");
    expect(step("convert").props["data-state"]).toBe("active");
    expect(text(step("recognize"))).toContain("CUDA0");
    expect(text(h.handle.current)).toContain("16.0× realtime");
    expect(text(h.handle.current)).not.toContain("83%");
    progress(0, 83, "paused"); flushFrames(); await h.settle();
    expect(text(h.handle.current)).toContain("Paused for recording");
    expect(step("recognize").props["data-state"]).toBe("paused");
    const pending = deferred<unknown>(); historyPending = pending.promise;
    jobs[0].resolve(result()); await h.settle();
    expect(step("convert").props["data-state"]).toBe("complete");
    expect(text(step("recognize"))).toContain("CUDA0");
    expect(step("save").props["data-state"]).toBe("active");
    pending.resolve(null); await h.settle();
    expect(step("save").props["data-state"]).toBe("complete");
});
