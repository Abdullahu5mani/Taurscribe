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
        case "check_meeting_permissions": return { microphone: "granted", system_audio: "granted" };
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
    const promise = new Promise<T>((done) => { resolve = done; });
    return { promise, resolve };
}
function fixture(options: { grammar?: boolean; mute?: boolean; file?: boolean } = {}) {
    const phases: string[] = [], notices: any[] = [], transcripts: string[] = [], headers: string[] = [];
    const noop = () => {};
    const hook = renderHook(() => useRecording({
        activeEngineRef: { current: "granite" }, models: [], graniteModels: [{ id: "granite-test" }], qwen3Models: [],
        isFileTranscribingRef: { current: options.file ?? false },
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

test("denied system audio never loads a model or starts a mic-only meeting", async () => {
    responses.set("check_meeting_permissions", () => ({ microphone: "granted", system_audio: "denied" }));
    const h = fixture(); await h.handle.current.handleStartRecording(false, "dual_channel"); await h.settle();
    expect(calls).not.toContain("start_recording");
    expect(calls).not.toContain("get_granite_status");
    expect(h.handle.current.isRecording).toBe(false);
    const notice = h.notices.at(-1);
    expect(notice.code).toBe("system_audio_permission_denied");
    notice.actions[0].onClick(); await h.settle();
    expect(calls).toContain("open_system_audio_settings");
});

test("an undecided audio grant is requested and verified before opening tracks", async () => {
    let granted = false;
    responses.set("check_meeting_permissions", () => ({ microphone: "granted", system_audio: granted ? "granted" : "undetermined" }));
    responses.set("request_system_audio_permission", () => { granted = true; return "granted"; });
    const h = fixture(); await h.handle.current.handleStartRecording(false, "dual_channel"); await h.settle();
    expect(calls.indexOf("request_system_audio_permission")).toBeLessThan(calls.indexOf("start_recording"));
    expect(h.handle.current.isDualChannelRecording).toBe(true);
});

test("an unreadable permission status fails closed while dictation remains independent", async () => {
    responses.set("check_meeting_permissions", () => null);
    const h = fixture(); await h.handle.current.handleStartRecording(false, "dual_channel");
    expect(calls).not.toContain("start_recording");
    expect(h.notices.at(-1).code).toBe("meeting_permission_check_failed");
    calls.length = 0;
    await h.handle.current.handleStartRecording(false, "mic");
    expect(calls).toContain("start_recording");
    expect(calls).not.toContain("check_meeting_permissions");
});

test("a permission reply after an external Stop cannot open a new capture", async () => {
    const pending = deferred<any>();
    responses.set("check_meeting_permissions", () => pending.promise);
    const h = fixture(); const start = h.handle.current.handleStartRecording(false, "dual_channel");
    await h.settle();
    fire("recording-state", { is_recording: false, is_dual_channel: false });
    pending.resolve({ microphone: "granted", system_audio: "granted" });
    await start; await h.settle();
    expect(calls).not.toContain("start_recording");
});

test("silent caller warning has a retry action and clears when sound returns", async () => {
    const h = fixture(); await h.handle.current.handleStartRecording(false, "dual_channel"); await h.settle();
    fire("meeting-audio-status", { status: "silent", message: "If someone is speaking, check call audio" });
    const notice = h.notices.at(-1);
    expect(notice.code).toBe("meeting_audio_unverified");
    expect(notice.sticky).toBe(true);
    notice.actions[0].onClick(); await h.settle();
    expect(calls).toContain("retry_meeting_audio_capture");
    fire("meeting-audio-status", { status: "restored", message: "Call audio received" });
    expect(h.notices.at(-1)).toBe(null);
    fire("recording-state", { is_recording: false, is_dual_channel: false });
    const count = h.notices.length;
    fire("meeting-audio-status", { status: "silent", message: "Late warning" });
    expect(h.notices).toHaveLength(count);
});

test("system-audio fallback stays visible during capture and clears on external Stop", async () => {
    const h = fixture(); await h.handle.current.handleStartRecording(false, "dual_channel"); await h.settle();
    fire("meeting-audio-status", { status: "fallback", message: "Recording all system audio" });
    fire("meeting-audio-status", { status: "restored", message: "Sound received" });
    expect(h.notices.at(-1).code).toBe("meeting_audio_fallback");
    fire("recording-state", { is_recording: false, is_dual_channel: false });
    expect(h.notices.at(-1)).toBe(null);
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
    h.handle.current.handleTranscriptionChunk("First sentence."); h.handle.current.handleTranscriptionChunk("Second sentence.");
    expect(h.transcripts.at(-1)).toBe("First sentence. Second sentence.");
    const pending = deferred<any>(); responses.set("stop_recording", () => pending.promise);
    const stop = h.handle.current.handleStopRecording(); await h.settle();
    h.handle.current.handleTranscriptionChunk(" tail"); expect(h.transcripts.at(-1)).toBe("First sentence. Second sentence. tail");
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


test("external starts initialize the timer and reset prior pause and transcript state", async () => {
    const originalNow = Date.now;
    let now = 100_000;
    Date.now = () => now;
    try {
        const h = fixture(); await h.settle();
        fire("recording-state", { is_recording: true, is_dual_channel: false });
        expect(h.handle.current.getRecordingMs()).toBe(0);
        now += 2000;
        await h.handle.current.handlePauseRecording();
        now += 5000;
        fire("recording-state", { is_recording: false, is_dual_channel: false });
        now += 3000;
        fire("recording-state", { is_recording: true, is_dual_channel: false, is_in_person: true });
        await h.settle();
        expect(h.handle.current.isPaused).toBe(false);
        expect(h.handle.current.getRecordingMs()).toBe(0);
        expect(h.handle.current.getLiveTranscript()).toBe("");
        now += 1500;
        expect(h.handle.current.getRecordingMs()).toBe(1500);
        fire("recording-state", { is_recording: true, is_dual_channel: false, is_in_person: true });
        expect(h.handle.current.getRecordingMs()).toBe(1500); // Duplicate event must not reset it.
    } finally { Date.now = originalNow; }
});

test("voice setup blocks button and hotkey starts before loading or opening audio", async () => {
    const h = fixture(); await h.settle();
    fire("voice-enrollment-state", { active: true }); await h.settle();
    expect(h.handle.current.isVoiceEnrolling).toBe(true);
    await h.handle.current.handleStartRecording();
    await h.handle.current.handleStartRecording(true);
    expect(calls).not.toContain("get_granite_status");
    expect(calls).not.toContain("start_recording");
    fire("voice-enrollment-state", { active: false }); await h.settle();
    await h.handle.current.handleStartRecording();
    expect(calls).toContain("start_recording");
});

test("an old voice status response cannot overwrite a newer enrollment event", async () => {
    const pending = deferred<any>(); responses.set("get_my_voice", () => pending.promise);
    const h = fixture(); await h.settle();
    fire("voice-enrollment-state", { active: true });
    pending.resolve({ active: false }); await h.settle();
    expect(h.handle.current.isVoiceEnrolling).toBe(true);
});

test("mounting during voice enrollment hydrates the busy state", async () => {
    responses.set("get_my_voice", () => ({ active: true }));
    const h = fixture(); await h.settle();
    expect(h.handle.current.isVoiceEnrolling).toBe(true);
    await h.handle.current.handleStartRecording();
    expect(calls).not.toContain("start_recording");
});

test("recording during a file skips the conflicting UI model load and supplies the model to the priority worker", async () => {
    responses.set("get_granite_status", () => ({ loaded: false, model_id: null }));
    const h = fixture({ file: true }); await h.settle();
    await h.handle.current.handleStartRecording(true);
    expect(calls).toContain("start_recording"); expect(calls).not.toContain("load_granite_model");
    expect(argumentsByCommand.get("start_recording")).toMatchObject({ modelId: "granite-test", useGpu: false });
    expect(h.headers).toContain("Pausing file transcription at the next chunk…");
    expect(h.handle.current.isRecordingRef.current).toBe(true);
});

test("an unloaded model does not delay Start: recording begins at once and the backend loads it", async () => {
    responses.set("get_granite_status", () => ({ loaded: false, model_id: null }));
    const h = fixture();
    await h.handle.current.handleStartRecording();
    await h.settle();
    expect(calls).not.toContain("init_granite");
    expect(calls).toContain("start_recording");
    expect(argumentsByCommand.get("start_recording")).toMatchObject({
        modelId: "granite-test", engine: "granite", backgroundLoad: true,
    });
    expect(h.handle.current.isRecording).toBe(true);
    // The backend reports the model ready: no failure, recording continues.
    fire("recording-model-loading", { loading: false, model_id: "granite-test" });
    await h.settle();
    expect(h.handle.current.isRecording).toBe(true);
    expect(calls).not.toContain("cancel_recording");
});

test("a model that fails to load during a recording discards it and says why", async () => {
    responses.set("get_granite_status", () => ({ loaded: false, model_id: null }));
    const h = fixture();
    await h.handle.current.handleStartRecording();
    await h.settle();
    fire("recording-model-loading", { loading: false, model_id: "granite-test", error: "model file is corrupt" });
    await h.settle();
    expect(calls).toContain("cancel_recording");
    expect(h.headers.some(m => m.includes("model file is corrupt"))).toBe(true);
});

test("a loaded model keeps the normal start: no background load requested", async () => {
    const h = fixture();
    await h.handle.current.handleStartRecording();
    await h.settle();
    expect(argumentsByCommand.get("start_recording")).toMatchObject({ backgroundLoad: false });
});
