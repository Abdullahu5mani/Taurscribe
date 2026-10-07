import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "./hookHarness";

const listeners = new Map<string, (event: any) => void>();
let voice = { enrolled: false, speech_seconds: null as number | null, created_at: null as string | null, speaker_model_installed: true };
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({
    ...realCore,
    invoke: async (cmd: string) => {
        if (cmd === "get_my_voice") return voice;
        throw new Error(cmd);
    },
}));
const realEvent = { ...(await import("@tauri-apps/api/event")) };
mock.module("@tauri-apps/api/event", () => ({
    ...realEvent,
    listen: async (name: string, callback: (event: any) => void) => {
        listeners.set(name, callback);
        return () => { if (listeners.get(name) === callback) listeners.delete(name); };
    },
}));

const { InPersonPanel, VOICE_PROMPTED_KEY } = await import("../../src/components/InPersonPanel");

const nodes = (root: any): any[] => {
    if (root == null || typeof root !== "object") return [];
    if (Array.isArray(root)) return root.flatMap(nodes);
    return [root, ...nodes(root.props?.children)];
};
const byTest = (root: any, id: string) => nodes(root).find((n) => n.props?.["data-testid"] === id);
const textOf = (root: any): string => {
    if (root == null || typeof root === "boolean") return "";
    if (typeof root !== "object") return String(root);
    if (Array.isArray(root)) return root.map(textOf).join(" ");
    return textOf(root.props?.children);
};
const voiceSetup = (root: any) => nodes(root).find((n) => typeof n.type === "function" && n.type.name === "VoiceSetup");

let settings: Record<string, unknown>;
let events: string[];
let props: any;
beforeEach(() => {
    listeners.clear();
    voice = { enrolled: false, speech_seconds: null, created_at: null, speaker_model_installed: true };
    settings = {};
    events = [];
    props = {
        isRecording: false, isInPersonRecording: false, isPaused: false, isProcessing: false, separatorInstalled: true,
        onStart: () => events.push("start"),
        onStop: () => events.push("stop"),
        onPause: () => events.push("pause"),
        onResume: () => events.push("resume"),
        onOpenModelSettings: () => events.push("models"),
        onClose: () => events.push("close"),
        getRecordingMs: () => 65_000,
        getLiveTranscript: () => "",
        getSetting: async (key: string) => settings[key] ?? null,
        setSetting: async (key: string, value: unknown) => { settings[key] = value; },
    };
});
const render = () => renderHook(() => InPersonPanel(props));

test("the first recording offers the voice setup once; skipping it records as Speaker 1, 2", async () => {
    const { handle, settle } = render();
    await settle();
    expect(textOf(handle.current)).toContain("everyone is labelled Speaker 1, 2");
    byTest(handle.current, "inperson-record-btn").props.onClick();
    await settle();
    const setup = voiceSetup(handle.current);
    expect(setup).toBeDefined();
    expect(events).toEqual([]);

    setup.props.onDone(false); // Skip
    await settle();
    expect(settings[VOICE_PROMPTED_KEY]).toBe(true);
    expect(events).toEqual([]);
    expect(voiceSetup(handle.current)).toBeUndefined();

    // Offered once: the next Start records straight away.
    byTest(handle.current, "inperson-record-btn").props.onClick();
    await settle();
    expect(events).toEqual(["start"]);
});

test("after setting up the voice, Start recording in the setup starts the meeting", async () => {
    const { handle, settle } = render();
    await settle();
    byTest(handle.current, "inperson-record-btn").props.onClick();
    await settle();
    voiceSetup(handle.current).props.onDone(true);
    await settle();
    expect(events).toEqual(["start"]);
});

test("with a voice set up, Start records at once", async () => {
    voice = { ...voice, enrolled: true, speech_seconds: 14 };
    const { handle, settle } = render();
    await settle();
    expect(textOf(handle.current)).toContain("labelled “You”");
    byTest(handle.current, "inperson-record-btn").props.onClick();
    await settle();
    expect(events).toEqual(["start"]);
});

test("while recording it shows the time and live transcript; Stop stops and closes the section", async () => {
    voice = { ...voice, enrolled: true };
    const { handle, settle, rerender } = render();
    await settle();
    props = { ...props, isRecording: true, isInPersonRecording: true };
    rerender();
    await settle();
    expect(textOf(byTest(handle.current, "inperson-clock"))).toBe("01:05");
    listeners.get("transcription-chunk")!({ payload: { text: "Let's start with the budget." } });
    listeners.get("transcription-chunk")!({ payload: { text: "  " } });
    listeners.get("transcription-chunk")!({ payload: { text: "I have the numbers here." } });
    await settle();
    const feed = textOf(byTest(handle.current, "inperson-feed"));
    expect(feed).toContain("Let's start with the budget.");
    expect(feed).toContain("I have the numbers here.");
    // No close button while recording.
    expect(byTest(handle.current, "inperson-close-btn")).toBeUndefined();

    byTest(handle.current, "inperson-pause-btn").props.onClick();
    byTest(handle.current, "inperson-stop-btn").props.onClick();
    expect(events).toEqual(["pause", "stop", "close"]);
});

test("reopened mid-recording, it shows the transcript so far", async () => {
    props = { ...props, isRecording: true, isInPersonRecording: true, getLiveTranscript: () => "Earlier we agreed on the twelfth." };
    const { handle, settle } = render();
    await settle();
    expect(textOf(byTest(handle.current, "inperson-feed"))).toContain("Earlier we agreed on the twelfth.");
});

test("when idle it can be closed", async () => {
    const { handle, settle } = render();
    await settle();
    byTest(handle.current, "inperson-close-btn").props.onClick();
    expect(events).toEqual(["close"]);
});

test("a call recording elsewhere keeps Start disabled, and a missing separator model is pointed out", async () => {
    props = { ...props, isRecording: true, isInPersonRecording: false, separatorInstalled: false };
    const { handle, settle } = render();
    await settle();
    expect(byTest(handle.current, "inperson-record-btn").props.disabled).toBe(true);
    expect(byTest(handle.current, "inperson-separator-missing")).toBeDefined();
});
