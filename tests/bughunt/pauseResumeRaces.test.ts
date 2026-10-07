import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "../frontend/hookHarness";
import { testTrace } from "./trace";
const trace = testTrace("pause-resume");

const listeners = new Map<string, (event: { payload: any }) => void>();
let calls: string[];
let argumentsByCommand: Map<string, any>;
let responses: Map<string, () => unknown>;
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({ ...realCore, invoke: async (cmd: string, args?: unknown) => {
    calls.push(cmd); trace("invoke", { cmd, args });
    argumentsByCommand.set(cmd, args);
    if (responses.has(cmd)) return responses.get(cmd)!();
    switch (cmd) {
        case "get_granite_status": return { loaded: true, model_id: "granite-test" };
        case "get_audio_source_mode": return "mic";
        case "start_recording": case "pause_recording": case "resume_recording": case "type_text": case "cancel_recording":
            return { ok: true, data: "ok" };
        case "stop_recording": return stopped();
        default: return null;
    }
} }));
const realEvent = { ...(await import("@tauri-apps/api/event")) };
mock.module("@tauri-apps/api/event", () => ({ ...realEvent,
    listen: async (name: string, callback: any) => {
        listeners.set(name, callback);
        return () => { if (listeners.get(name) === callback) listeners.delete(name); };
    }, emit: async () => {},
}));
const { useRecording } = await import("../../src/hooks/useRecording");
const stopped = () => ({ ok: true, data: {
    kind: "dictation", transcript: "final text", warnings: [], recording_path: null,
    meeting_id: null, duration_ms: 2000, too_short: false, history: { status: "saved", id: 1 },
} });
function deferred<T>() {
    let resolve!: (value: T) => void;
    let reject!: (error: unknown) => void;
    const promise = new Promise<T>((done, failed) => { resolve = done; reject = failed; });
    return { promise, resolve, reject };
}
function fixture(options: { grammar?: boolean; mute?: boolean; tray?: (s: string) => Promise<void> } = {}) {
    const phases: string[] = [], notices: any[] = [], transcripts: string[] = [], headers: string[] = [];
    const noop = () => {};
    const hook = renderHook(() => useRecording({
        activeEngineRef: { current: "granite" }, models: [], graniteModels: [{ id: "granite-test" }], qwen3Models: [],
        currentModel: null, currentGraniteModel: "granite-test", currentQwen3Model: null, asrBackend: "cpu",
        setCurrentModel: noop, setLoadedEngine: noop, enableGrammarLMRef: { current: options.grammar ?? false },
        enableDenoiseRef: { current: true }, enableOverlayRef: { current: false },
        muteBackgroundAudioRef: { current: options.mute ?? false }, transcriptionStyleRef: { current: "Clean" },
        setHeaderStatus: (s: string) => headers.push(s), setTrayState: options.tray ?? (async () => {}), setIsSettingsOpen: noop,
        dictionaryRef: { current: [] }, snippetsRef: { current: [] },
        setSessionPhase: (s: string) => phases.push(s), setSessionNotice: (n: any) => notices.push(n),
        setSessionTranscript: (s: string) => transcripts.push(s),
    } as any));
    return { ...hook, phases, notices, transcripts, headers };
}
const fire = (event: string, payload: unknown) => listeners.get(event)!({ payload });
beforeEach(() => { calls = []; argumentsByCommand = new Map(); responses = new Map(); listeners.clear(); });


test("duplicate Pause responses cannot undo a later Resume", async () => {
    const h = fixture(); await h.handle.current.handleStartRecording();
    const first = deferred<any>(), duplicate = deferred<any>(); let requested = 0;
    responses.set("pause_recording", () => ++requested === 1 ? first.promise : duplicate.promise);
    const pause = h.handle.current.handlePauseRecording();
    const repeated = h.handle.current.handlePauseRecording(); await h.settle();
    first.resolve({ ok: true, data: "paused" }); await pause; await h.settle();
    await h.handle.current.handleResumeRecording(); await h.settle();
    duplicate.resolve({ ok: true, data: "paused" }); await repeated; await h.settle();
    trace("final.duplicate-pause", { paused: h.handle.current.isPaused, requested });
    expect(requested).toBe(1);
    expect(h.handle.current.isPaused).toBe(false);
});

test("duplicate Resume responses cannot undo a later Pause", async () => {
    const h = fixture(); await h.handle.current.handleStartRecording(); await h.handle.current.handlePauseRecording();
    const first = deferred<any>(), duplicate = deferred<any>(); let requested = 0;
    responses.set("resume_recording", () => ++requested === 1 ? first.promise : duplicate.promise);
    const resume = h.handle.current.handleResumeRecording();
    const repeated = h.handle.current.handleResumeRecording(); await h.settle();
    first.resolve({ ok: true, data: "resumed" }); await resume; await h.settle();
    await h.handle.current.handlePauseRecording(); await h.settle();
    duplicate.resolve({ ok: true, data: "resumed" }); await repeated; await h.settle();
    trace("final.duplicate-resume", { paused: h.handle.current.isPaused, requested });
    expect(requested).toBe(1);
    expect(h.handle.current.isPaused).toBe(true);
});


test("an old Pause tray response cannot change the phase after Stop", async () => {
    const tray = deferred<void>();
    const h = fixture({ tray: async (s) => { if (s === "paused") await tray.promise; } });
    await h.handle.current.handleStartRecording();
    const request = h.handle.current.handlePauseRecording(); await h.settle();
    await h.handle.current.handleStopRecording(); await h.settle();
    tray.resolve(); await request; await h.settle();
    trace("final.late-tray", h.phases);
    expect(h.phases.at(-1)).not.toBe("paused");
});

test("a Stop waiting on tray state cannot stop a newer externally started capture", async () => {
    const tray = deferred<void>();
    const h = fixture({ tray: async (s) => { if (s === "processing_speech") await tray.promise; } });
    await h.handle.current.handleStartRecording();
    const stopping = h.handle.current.handleStopRecording(); await h.settle();
    fire("recording-state", { is_recording: false, is_dual_channel: false });
    fire("recording-state", { is_recording: true, is_dual_channel: true }); await h.settle();
    tray.resolve(); await stopping; await h.settle();
    trace("final.superseded-stop", { commands: calls, recording: h.handle.current.isRecording });
    expect(calls.filter(c => c === "stop_recording")).toHaveLength(0);
    expect(h.handle.current.isRecording).toBe(true);
});

test("a failed Start waiting on cleanup cannot clear a newer external recording", async () => {
    const tray = deferred<void>();
    const h = fixture({ tray: async (s) => { if (s === "ready") await tray.promise; } });
    responses.set("start_recording", () => { throw new Error("synthetic device failure"); });
    const starting = h.handle.current.handleStartRecording(); await h.settle();
    fire("recording-state", { is_recording: true, is_dual_channel: true }); await h.settle();
    tray.resolve(); await starting; await h.settle();
    trace("final.superseded-start-cleanup", { recording: h.handle.current.isRecording, phases: h.phases });
    expect(h.handle.current.isRecording).toBe(true);
    expect(h.handle.current.isDualChannelRecording).toBe(true);
    expect(h.phases.at(-1)).toBe("recording");
});

for (const ending of ["meeting", "too-short", "silence", "dictation", "paste-failed"] as const) {
    test(`a delayed ${ending} tray response cannot overwrite a newer recording`, async () => {
        const tray = deferred<void>();
        const blockedState = ending === "too-short" || ending === "silence" ? "nothing_heard" : ending === "paste-failed" ? "paste_failed" : "done";
        const h = fixture({ tray: async (s) => { if (s === blockedState) await tray.promise; } });
        await h.handle.current.handleStartRecording();
        const outcome = stopped();
        if (ending === "meeting") outcome.data.kind = "meeting";
        if (ending === "too-short") outcome.data.too_short = true;
        if (ending === "silence") outcome.data.transcript = "";
        responses.set("stop_recording", () => outcome);
        if (ending === "paste-failed") responses.set("type_text", () => ({ ok: false, error: { code: "secure_input", message: "blocked" } }));
        const stop = h.handle.current.handleStopRecording(); await h.settle();
        fire("recording-state", { is_recording: false, is_dual_channel: false });
        fire("recording-state", { is_recording: true, is_dual_channel: true });
        h.handle.current.handleTranscriptionChunk("Synthetic new meeting speech"); await h.settle();
        tray.resolve(); await stop; await h.settle();
        trace("final.terminal-tray", { ending, phases: h.phases, transcripts: h.transcripts, busy: h.handle.current.isProcessingTranscript });
        expect(h.handle.current.isRecording).toBe(true);
        expect(h.phases.at(-1)).toBe("recording");
        expect(h.transcripts.at(-1)).toBe("Synthetic new meeting speech");
    });
}

for (const ending of ["meeting", "too-short", "silence"] as const) {
    test(`a delayed ${ending} unmute cannot reset a newer recording's transcript`, async () => {
        const h = fixture({ mute: true }); await h.handle.current.handleStartRecording();
        const unmute = deferred<null>(); responses.set("unmute_system_audio", () => unmute.promise);
        const outcome = stopped();
        if (ending === "meeting") outcome.data.kind = "meeting";
        if (ending === "too-short") outcome.data.too_short = true;
        if (ending === "silence") outcome.data.transcript = "";
        responses.set("stop_recording", () => outcome);
        const stop = h.handle.current.handleStopRecording(); await h.settle();
        fire("recording-state", { is_recording: false, is_dual_channel: false });
        fire("recording-state", { is_recording: true, is_dual_channel: true });
        h.handle.current.handleTranscriptionChunk("Synthetic new capture"); await h.settle();
        unmute.resolve(null); await stop; await h.settle();
        trace("final.unmute", { ending, phases: h.phases, transcripts: h.transcripts });
        expect(h.handle.current.isRecording).toBe(true);
        expect(h.transcripts.at(-1)).toBe("Synthetic new capture");
        expect(h.phases.at(-1)).toBe("recording");
    });
}

for (const action of ["pause", "resume"] as const) {
    test(`a failed ${action} request releases its guard and allows a retry`, async () => {
        const h = fixture(); await h.handle.current.handleStartRecording();
        if (action === "resume") await h.handle.current.handlePauseRecording();
        responses.set(`${action}_recording`, () => { throw new Error("temporary capture failure"); });
        await h.handle.current[action === "pause" ? "handlePauseRecording" : "handleResumeRecording"]();
        responses.delete(`${action}_recording`);
        await h.handle.current[action === "pause" ? "handlePauseRecording" : "handleResumeRecording"](); await h.settle();
        expect(h.handle.current.isPaused).toBe(action === "pause");
    });
}
