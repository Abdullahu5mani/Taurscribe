import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "./hookHarness";

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
    const promise = new Promise<T>((done) => { resolve = done; });
    return { promise, resolve };
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

test("double Stop sends one command and stays processing until it completes", async () => {
    const h = fixture();
    await h.handle.current.handleStartRecording();
    const pending = deferred<any>(); responses.set("stop_recording", () => pending.promise);
    const first = h.handle.current.handleStopRecording();
    const second = h.handle.current.handleStopRecording();
    await h.settle();
    const count = calls.filter(c => c === "stop_recording").length;
    const processing = h.handle.current.isProcessingTranscript;
    pending.resolve(stopped()); await Promise.all([first, second]); await h.settle();
    expect(count).toBe(1); expect(processing).toBe(true);
    expect(calls.filter(c => c === "type_text")).toHaveLength(1);
});

test("double Start while a model is loading opens devices only once", async () => {
    const pending = deferred<any>(); responses.set("get_granite_status", () => pending.promise);
    const h = fixture();
    const first = h.handle.current.handleStartRecording();
    const second = h.handle.current.handleStartRecording();
    await h.settle(); expect(calls).not.toContain("start_recording");
    pending.resolve({ loaded: true, model_id: "granite-test" });
    await Promise.all([first, second]); await h.settle();
    expect(calls.filter(c => c === "start_recording")).toHaveLength(1);
});

test("a start during pending transcription cannot replace the active session", async () => {
    const h = fixture(); await h.handle.current.handleStartRecording();
    const pending = deferred<any>(); responses.set("stop_recording", () => pending.promise);
    const stop = h.handle.current.handleStopRecording(); await h.settle();
    await h.handle.current.handleStartRecording();
    expect(calls.filter(c => c === "start_recording")).toHaveLength(1);
    pending.resolve(stopped()); await stop;
});

test("a manually started meeting shows both channels before any level event", async () => {
    const h = fixture({ mute: true }); await h.handle.current.handleStartRecording(false, "dual_channel"); await h.settle();
    expect(h.handle.current.isDualChannelRecording).toBe(true);
    expect(calls).not.toContain("mute_system_audio");
});

test("late meeting levels cannot restore meeting UI after backend Stop", async () => {
    const h = fixture();
    fire("recording-state", { is_recording: true, is_dual_channel: true });
    fire("dual-audio-levels", { mic: 0.3, system: 0.4 }); await h.settle();
    expect(h.handle.current.dualLevels.system).toBe(0.4);
    fire("recording-state", { is_recording: false, is_dual_channel: false });
    fire("dual-audio-levels", { mic: 0.8, system: 0.9 }); await h.settle();
    expect(h.handle.current.isRecording).toBe(false);
    expect(h.handle.current.isDualChannelRecording).toBe(false);
    expect(h.handle.current.dualLevels).toEqual({ mic: 0, system: 0 });
});

test("a dictation ignores delayed levels from an earlier meeting", async () => {
    const h = fixture(); await h.handle.current.handleStartRecording();
    fire("dual-audio-levels", { mic: 0.5, system: 0.7 }); await h.settle();
    expect(h.handle.current.isDualChannelRecording).toBe(false);
    expect(h.handle.current.dualLevels.system).toBe(0);
});

test("failed pause leaves capture live and accepts subsequent chunks", async () => {
    responses.set("pause_recording", () => ({ ok: false, error: { message: "device failed" } }));
    const h = fixture(); await h.handle.current.handleStartRecording();
    await h.handle.current.handlePauseRecording(); h.handle.current.handleTranscriptionChunk(" hello"); await h.settle();
    expect(h.handle.current.isPaused).toBe(false); expect(h.handle.current.isRecording).toBe(true);
    expect(h.transcripts.at(-1)).toBe("hello");
});

test("pause rejects chunks; failed resume remains paused; successful resume accepts them", async () => {
    const h = fixture(); await h.handle.current.handleStartRecording();
    await h.handle.current.handlePauseRecording(); h.handle.current.handleTranscriptionChunk(" stale");
    responses.set("resume_recording", () => ({ ok: false, error: { message: "device failed" } }));
    await h.handle.current.handleResumeRecording(); await h.settle();
    expect(h.handle.current.isPaused).toBe(true); expect(h.transcripts.at(-1)).toBe("");
    responses.delete("resume_recording"); await h.handle.current.handleResumeRecording();
    h.handle.current.handleTranscriptionChunk(" fresh"); expect(h.transcripts.at(-1)).toBe("fresh");
});

test("tail chunks are accepted during Stop and stale chunks rejected after completion", async () => {
    const h = fixture(); await h.handle.current.handleStartRecording();
    h.handle.current.handleTranscriptionChunk(" nest"); h.handle.current.handleTranscriptionChunk("ed");
    expect(h.transcripts.at(-1)).toBe("nested");
    const pending = deferred<any>(); responses.set("stop_recording", () => pending.promise);
    const stop = h.handle.current.handleStopRecording(); await h.settle();
    h.handle.current.handleTranscriptionChunk(" tail"); expect(h.transcripts.at(-1)).toBe("nested tail");
    pending.resolve(stopped()); await stop;
    h.handle.current.handleTranscriptionChunk(" stale"); expect(h.transcripts.at(-1)).toBe("final text");
});

test("paste failure preserves history and restores muted audio", async () => {
    responses.set("type_text", () => ({ ok: false, error: { code: "secure_input" } }));
    const h = fixture({ mute: true }); await h.handle.current.handleStartRecording();
    await h.handle.current.handleStopRecording(); await h.settle();
    expect(calls).toContain("update_transcript_history"); expect(calls).toContain("unmute_system_audio");
    expect(h.notices.at(-1)?.code).toBe("paste_blocked_secure_input"); expect(h.phases.at(-1)).toBe("warning");
    expect(h.transcripts.at(-1)).toBe("final text"); expect(h.handle.current.isProcessingTranscript).toBe(false);
});

test("grammar failure keeps the original transcript and its warning after paste succeeds", async () => {
    responses.set("correct_text", () => { throw new Error("model unavailable"); });
    const h = fixture({ grammar: true }); await h.handle.current.handleStartRecording();
    await h.handle.current.handleStopRecording(); await h.settle();
    expect(h.transcripts.at(-1)).toBe("final text");
    expect(h.notices.at(-1)?.title).toBe("Grammar correction failed"); expect(h.phases.at(-1)).toBe("warning");
    expect(argumentsByCommand.get("update_transcript_history").grammarLlmUsed).toBe(false);
});

test("successful grammar correction is pasted and saved with truthful history metadata", async () => {
    responses.set("correct_text", () => "Final corrected text.");
    const h = fixture({ grammar: true }); await h.handle.current.handleStartRecording();
    await h.handle.current.handleStopRecording(); await h.settle();
    expect(argumentsByCommand.get("type_text")).toEqual({ text: "Final corrected text." });
    expect(argumentsByCommand.get("update_transcript_history")).toMatchObject({ transcript: "Final corrected text.", grammarLlmUsed: true });
    expect(h.phases.at(-1)).toBe("success"); expect(h.notices.at(-1)).toBeNull();
});

test("paste failure does not hide capture, history or grammar failure warnings", async () => {
    responses.set("correct_text", () => { throw new Error("model unavailable"); });
    responses.set("type_text", () => ({ ok: false, error: { code: "secure_input" } }));
    responses.set("stop_recording", () => ({ ok: true, data: { ...stopped().data,
        warnings: ["Capture lost 960 frames"], history: { status: "not_saved", error: "disk full", recording_path: "/tmp/recovery.wav" },
    } }));
    responses.set("save_transcript_history", () => { throw new Error("disk full"); });
    const h = fixture({ grammar: true }); await h.handle.current.handleStartRecording();
    await h.handle.current.handleStopRecording(); await h.settle();
    const notice = h.notices.at(-1);
    expect(notice.code).toBe("paste_blocked_secure_input");
    for (const message of ["960 frames", "disk full", "recovery.wav", "grammar correction step failed"]) expect(notice.message).toContain(message);
    expect(h.transcripts.at(-1)).toBe("final text");
});

test("cancel failure retains active recording and a successful retry clears it", async () => {
    responses.set("cancel_recording", () => { throw new Error("device busy"); });
    const h = fixture(); await h.handle.current.handleStartRecording(false, "dual_channel");
    await h.handle.current.handleCancelRecording(); await h.settle(); expect(h.handle.current.isRecording).toBe(true);
    responses.delete("cancel_recording"); await h.handle.current.handleCancelRecording(); await h.settle();
    expect(h.handle.current.isRecording).toBe(false); expect(h.handle.current.isDualChannelRecording).toBe(false);
    expect(h.notices.at(-1)?.title).toBe("Recording discarded");
});

test("a pending pause cannot put a stopped recording back into the paused state", async () => {
    const h = fixture(); await h.handle.current.handleStartRecording();
    const pending = deferred<any>(); responses.set("pause_recording", () => pending.promise);
    const pause = h.handle.current.handlePauseRecording();
    await h.handle.current.handleStopRecording();
    pending.resolve({ ok: true, data: "paused" }); await pause; await h.settle();
    expect(h.handle.current.isPaused).toBe(false); expect(h.phases.at(-1)).toBe("success");
});

test("a pending resume cannot change the phase of a newer recording", async () => {
    const h = fixture(); await h.handle.current.handleStartRecording(); await h.handle.current.handlePauseRecording();
    const pending = deferred<any>(); responses.set("resume_recording", () => pending.promise);
    const resume = h.handle.current.handleResumeRecording();
    await h.handle.current.handleStopRecording(); await h.handle.current.handleStartRecording();
    await h.handle.current.handlePauseRecording();
    pending.resolve({ ok: true, data: "resumed" }); await resume; await h.settle();
    expect(h.handle.current.isPaused).toBe(true); expect(h.phases.at(-1)).toBe("paused");
});
