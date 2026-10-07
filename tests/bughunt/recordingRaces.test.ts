import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "../frontend/hookHarness";

const listeners = new Map<string, (event: { payload: any }) => void>();
let calls: string[];
let argumentsByCommand: Map<string, any>;
let responses: Map<string, () => unknown>;
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({ ...realCore, invoke: async (cmd: string, args?: unknown) => {
    calls.push(cmd);
    argumentsByCommand.set(cmd, args);
    if (responses.has(cmd)) return responses.get(cmd)!();
    switch (cmd) {
        case "get_granite_status": return { loaded: true, model_id: "granite-test" };
        case "get_audio_source_mode": return "mic";
        case "start_recording": case "pause_recording": case "resume_recording": case "type_text":
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
function fixture(options: { grammar?: boolean; mute?: boolean } = {}) {
    const phases: string[] = [], notices: any[] = [], transcripts: string[] = [], headers: string[] = [];
    const noop = () => {};
    const hook = renderHook(() => useRecording({
        activeEngineRef: { current: "granite" }, models: [], graniteModels: [{ id: "granite-test" }], qwen3Models: [],
        currentModel: null, currentGraniteModel: "granite-test", currentQwen3Model: null, asrBackend: "cpu",
        setCurrentModel: noop, setLoadedEngine: noop, enableGrammarLMRef: { current: options.grammar ?? false },
        enableDenoiseRef: { current: true }, enableOverlayRef: { current: false },
        muteBackgroundAudioRef: { current: options.mute ?? false }, transcriptionStyleRef: { current: "Clean" },
        setHeaderStatus: (s: string) => headers.push(s), setTrayState: async () => {}, setIsSettingsOpen: noop,
        dictionaryRef: { current: [] }, snippetsRef: { current: [] },
        setSessionPhase: (s: string) => phases.push(s), setSessionNotice: (n: any) => notices.push(n),
        setSessionTranscript: (s: string) => transcripts.push(s),
    } as any));
    return { ...hook, phases, notices, transcripts, headers };
}
const fire = (event: string, payload: unknown) => listeners.get(event)!({ payload });
beforeEach(() => { calls = []; argumentsByCommand = new Map(); responses = new Map(); listeners.clear(); });

// Discovery assertions express the required behavior. Run explicitly with
// bun test tests/bughunt; failures identify product bugs, not accepted outcomes.

test("a completed Stop cannot be undone by a delayed Start response", async () => {
    const pending = deferred<any>(); responses.set("start_recording", () => pending.promise);
    const h = fixture(); await h.settle();
    const start = h.handle.current.handleStartRecording(); await h.settle();
    fire("recording-state", { is_recording: true, is_dual_channel: false }); await h.settle();
    await h.handle.current.handleStopRecording();
    fire("recording-state", { is_recording: false, is_dual_channel: false });
    pending.resolve({ ok: true, data: "started" }); await start; await h.settle();
    expect(calls.filter(c => c === "stop_recording")).toHaveLength(1);
    expect(h.handle.current.isRecording).toBe(false);
    expect(h.phases.at(-1)).not.toBe("recording");
});

test("a rejected Stop keeps the still-running capture visible and retryable", async () => {
    const h = fixture(); await h.handle.current.handleStartRecording();
    responses.set("stop_recording", () => { throw "A recording is starting or finishing; try again in a moment"; });
    await h.handle.current.handleStopRecording(); await h.settle();
    expect(h.handle.current.isProcessingTranscript).toBe(false);
    expect(h.handle.current.isRecording).toBe(true);
    expect(h.handle.current.isRecordingRef.current).toBe(true);
});

test("a failed old Start cannot clear a newer externally started session", async () => {
    const pending = deferred<any>(); responses.set("start_recording", () => pending.promise);
    const h = fixture(); await h.settle();
    const start = h.handle.current.handleStartRecording(); await h.settle();
    fire("recording-state", { is_recording: true, is_dual_channel: true });
    pending.reject("A recording is starting or finishing; try again in a moment");
    await start; await h.settle();
    expect(h.handle.current.isRecording).toBe(true);
    expect(h.handle.current.isDualChannelRecording).toBe(true);
});

test("backend mode reconciliation wins over the delayed UI Start mode", async () => {
    const pending = deferred<any>(); responses.set("start_recording", () => pending.promise);
    const h = fixture(); await h.settle();
    const start = h.handle.current.handleStartRecording(false, "mic"); await h.settle();
    // The backend accepted a meeting start while the UI command was pending.
    fire("recording-state", { is_recording: true, is_dual_channel: true });
    pending.resolve({ ok: false, error: { code: "already_recording", message: "Already recording" } });
    await start; await h.settle();
    expect(h.handle.current.isRecording).toBe(true);
    expect(h.handle.current.isDualChannelRecording).toBe(true);
});

test("duplicate voice busy events do not prevent recording after enrollment finishes", async () => {
    const h = fixture(); await h.settle();
    fire("voice-enrollment-state", { active: true });
    fire("voice-enrollment-state", { active: true });
    await h.handle.current.handleStartRecording();
    fire("voice-enrollment-state", { active: false });
    await h.handle.current.handleStartRecording(); await h.settle();
    expect(calls.filter(c => c === "start_recording")).toHaveLength(1);
    expect(h.handle.current.isVoiceEnrolling).toBe(false);
});
