import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "./hookHarness";
import type { RecordingOutcome, SessionNotice } from "../../src/types/session";

type Call = { cmd: string; args?: Record<string, unknown> };
let calls: Call[] = [];
let stopOutcome: RecordingOutcome;
let failSave = false;

const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({
    ...realCore,
    invoke: async (cmd: string, args?: Record<string, unknown>) => {
        calls.push({ cmd, args });
        switch (cmd) {
            case "stop_recording": return { ok: true, data: stopOutcome, error: null };
            case "type_text": return { ok: true, data: null, error: null };
            case "save_transcript_history":
                if (failSave) throw new Error("database or disk is full");
                return null;
            default: return null;
        }
    },
}));
const realEvent = { ...(await import("@tauri-apps/api/event")) };
mock.module("@tauri-apps/api/event", () => ({ ...realEvent, listen: async () => () => {}, emit: async () => {} }));

const { useRecording } = await import("../../src/hooks/useRecording");

let notices: (SessionNotice | null)[] = [];
let phases: string[] = [];

function stop(outcome: Partial<RecordingOutcome>) {
    stopOutcome = {
        transcript: "hello world",
        kind: "dictation",
        meeting_id: null,
        duration_ms: 1_000,
        too_short: false,
        history: { status: "saved", id: 5 },
        ...outcome,
    };
    const noop = () => {};
    const { handle } = renderHook(() =>
        useRecording({
            activeEngineRef: { current: "granite" },
            models: [], graniteModels: [], qwen3Models: [],
            currentModel: null, currentGraniteModel: "granite-speech-5-nc", currentQwen3Model: null,
            asrBackend: "gpu",
            setCurrentModel: noop, setLoadedEngine: noop,
            enableGrammarLMRef: { current: false },
            enableDenoiseRef: { current: true },
            enableOverlayRef: { current: false },
            muteBackgroundAudioRef: { current: false },
            transcriptionStyleRef: { current: "Clean" },
            setHeaderStatus: noop,
            setTrayState: async () => {},
            setIsSettingsOpen: noop,
            dictionaryRef: { current: [] },
            snippetsRef: { current: [] },
            setSessionPhase: (p: string) => { phases.push(p); },
            setSessionNotice: (n: SessionNotice | null) => { notices.push(n); },
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        } as any),
    );
    return handle.current.handleStopRecording();
}

const cmds = () => calls.map((c) => c.cmd).filter((c) => c !== "set_tray_state");

beforeEach(() => {
    calls = [];
    notices = [];
    phases = [];
    failSave = false;
});

test("a dictation's history entry, saved by the backend, gets the final text", async () => {
    await stop({});
    expect(cmds()).toContain("type_text");
    const update = calls.find((c) => c.cmd === "update_transcript_history");
    expect(update?.args).toMatchObject({ id: 5, transcript: "hello world", grammarLlmUsed: false });
    // Not saved a second time.
    expect(cmds()).not.toContain("save_transcript_history");
    expect(phases.at(-1)).toBe("success");
});

test("the backend decides a recording was too short, whatever Stop cost", async () => {
    await stop({ too_short: true, duration_ms: 120, history: { status: "skipped" } });
    expect(cmds()).not.toContain("type_text");
    expect(cmds()).not.toContain("update_transcript_history");
    expect(notices.at(-1)?.code).toBe("recording_too_short");
});

test("a meeting is never pasted or added to dictation history", async () => {
    await stop({ kind: "meeting", meeting_id: 9, history: { status: "skipped" } });
    expect(cmds()).not.toContain("type_text");
    expect(cmds()).not.toContain("update_transcript_history");
    expect(cmds()).not.toContain("save_transcript_history");
});

test("a transcript that could not be saved is retried, then reported with the kept recording", async () => {
    failSave = true;
    await stop({ history: { status: "not_saved", error: "disk full", recording_path: "/tmp/rec.wav" } });
    expect(cmds()).toContain("type_text");
    expect(calls.find((c) => c.cmd === "save_transcript_history")?.args).toMatchObject({ transcript: "hello world", durationMs: 1_000 });
    expect(phases.at(-1)).toBe("warning");
    expect(notices.at(-1)?.code).toBe("history_not_saved");
    expect(notices.at(-1)?.message).toContain("/tmp/rec.wav");
});

test("a retried save that works clears the problem", async () => {
    await stop({ history: { status: "not_saved", error: "database is locked", recording_path: null } });
    expect(cmds()).toContain("save_transcript_history");
    expect(phases.at(-1)).toBe("success");
    expect(notices.at(-1)).toBeNull();
});
