import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "./hookHarness";

// The app-wide event listeners (useHotkeyListeners): the global hotkey's
// start / stop / toggle rules, and what happens when the microphone
// disconnects, falls back to another device, or the recording writer fails.
const listeners = new Map<string, (event: any) => unknown>();
const invoked: string[] = [];
const realEvent = { ...(await import("@tauri-apps/api/event")) };
mock.module("@tauri-apps/api/event", () => ({
    ...realEvent,
    listen: async (name: string, cb: (event: any) => unknown) => {
        listeners.set(name, cb);
        return () => { if (listeners.get(name) === cb) listeners.delete(name); };
    },
}));
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({
    ...realCore,
    invoke: async (cmd: string) => { invoked.push(cmd); return null; },
}));
const { useHotkeyListeners } = await import("../../src/hooks/useHotkeyListeners");

let events: string[];
let statuses: string[];
let refs: any;
let stopGate: Promise<void> | null;
let startGate: Promise<void> | null;
beforeEach(() => {
    listeners.clear();
    invoked.length = 0;
    events = [];
    statuses = [];
    stopGate = null;
    startGate = null;
    refs = {
        isRecordingRef: { current: false },
        isLoadingRef: { current: false },
        activeEngineRef: { current: "whisper" },
        isFileTranscribingRef: { current: false },
        isMeetingImportingRef: { current: false },
        enableOverlayRef: { current: true },
        asrModelCountsRef: { current: { whisper: 1, granite: 0, qwen3: 0 } },
        handleStartRecordingRef: { current: async () => { events.push("start"); if (startGate) await startGate; refs.isRecordingRef.current = true; } },
        handleStopRecordingRef: { current: async () => { events.push("stop"); if (stopGate) await stopGate; refs.isRecordingRef.current = false; } },
        handleTranscriptionChunkRef: { current: () => {} },
        playErrorRef: { current: () => events.push("error-sound") },
        setHeaderStatusRef: { current: (msg: string) => statuses.push(msg) },
        triggerNoModelAttentionRef: { current: () => events.push("no-model") },
        setLoadedEngine: () => {},
        silenceTimerRef: { current: null },
        setShowSilenceWarning: () => {},
        refreshMacPermissions: async () => {},
    };
});
const mount = async () => {
    const view = renderHook(() => useHotkeyListeners(refs));
    await view.settle();
    return view;
};
const fire = async (name: string, payload?: unknown) => { await listeners.get(name)!({ payload }); };
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

test("the hotkey starts, then stops after the debounce window", async () => {
    await mount();
    await fire("hotkey-start-recording");
    expect(events).toEqual(["start"]);
    await fire("hotkey-stop-recording");
    expect(events).toEqual(["start", "stop"]);
    expect(invoked).toContain("cli_ui_ready");
});

test("key chatter: a second start within 350 ms is ignored", async () => {
    await mount();
    await fire("hotkey-start-recording");
    refs.isRecordingRef.current = false; // even if the first start failed
    await fire("hotkey-start-recording");
    expect(events).toEqual(["start"]);
    await wait(360);
    await fire("hotkey-start-recording");
    expect(events).toEqual(["start", "start"]);
});

test("toggle: press to start, press to stop", async () => {
    await mount();
    await fire("hotkey-toggle-recording");
    await fire("hotkey-toggle-recording");
    expect(events).toEqual(["start", "stop"]);
});

test("a stop pressed while the recording is still starting stops it once it has started", async () => {
    let release!: () => void;
    startGate = new Promise((r) => { release = r; });
    await mount();
    const starting = fire("hotkey-start-recording");
    await wait(0);
    await fire("hotkey-stop-recording"); // key released before start finished
    expect(events).toEqual(["start"]);
    release();
    await starting;
    await wait(300);
    expect(events).toEqual(["start", "stop"]);
});

test("the hotkey refuses while a model loads, a file transcribes, or no model is installed", async () => {
    await mount();
    refs.isLoadingRef.current = true;
    await fire("hotkey-start-recording");
    expect(events).toEqual(["error-sound"]);
    expect(invoked).toContain("show_overlay");

    refs.isLoadingRef.current = false;
    refs.isFileTranscribingRef.current = true;
    await wait(360);
    await fire("hotkey-start-recording");
    expect(statuses).toContain("Cannot record while a file is being transcribed");

    refs.isFileTranscribingRef.current = false;
    refs.activeEngineRef.current = "granite";
    await wait(360);
    await fire("hotkey-start-recording");
    expect(events).toEqual(["error-sound", "error-sound", "error-sound", "no-model"]);
});

test("a disconnected microphone stops the recording once and says why", async () => {
    let release!: () => void;
    stopGate = new Promise((r) => { release = r; });
    refs.isRecordingRef.current = true;
    await mount();
    await fire("audio-disconnected", { code: "audio_device_disconnected", message: "The requested device is no longer available." });
    await fire("audio-disconnected", { code: "audio_device_disconnected", message: "The requested device is no longer available." });
    await fire("hotkey-stop-recording"); // the user also presses stop
    release();
    await wait(0);
    expect(events).toEqual(["stop"]);
    expect(statuses[0]).toBe("Microphone disconnected: The requested device is no longer available.");
});

test("a disconnect when not recording only shows the message; older string payloads still work", async () => {
    await mount();
    await fire("audio-disconnected", "device removed");
    expect(events).toEqual([]);
    expect(statuses).toEqual(["Microphone disconnected: device removed"]);
});

test("after a disconnect stops the recording, the next start works (retry)", async () => {
    refs.isRecordingRef.current = true;
    await mount();
    await fire("audio-disconnected", { message: "gone" });
    await wait(0);
    expect(refs.isRecordingRef.current).toBe(false);
    await fire("hotkey-start-recording");
    expect(events).toEqual(["stop", "start"]);
});

test("a missing chosen mic falls back to another device with a notice", async () => {
    await mount();
    await fire("audio-fallback", "MacBook Air Microphone");
    expect(statuses).toEqual(["Mic lost, using fallback: MacBook Air Microphone"]);
});

test("a failing recording writer stops the recording and keeps the message up", async () => {
    refs.isRecordingRef.current = true;
    await mount();
    await fire("recording-write-error", "Recording queue overflowed; audio is incomplete and will be retained.");
    await wait(0);
    expect(events).toEqual(["stop"]);
    expect(statuses[0]).toContain("audio is incomplete and will be retained");
});

test("unmounting removes every listener", async () => {
    const view = await mount();
    expect(listeners.size).toBeGreaterThanOrEqual(10);
    view.unmount();
    expect(listeners.size).toBe(0);
});
