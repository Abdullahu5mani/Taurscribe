import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "./hookHarness";

const listeners = new Map<string, (event: any) => void>();
const calls: { cmd: string; args: any }[] = [];
let detail: any;
let pendingDetail: Promise<any> | null = null;
let invokeOverride: ((cmd: string, args: any) => Promise<any> | undefined) | null = null;
let deferListeners = false;
const registrations: (() => void)[] = [];
const subscriptions = new Map<string, Set<(event: any) => void>>();
const playPlans: Promise<void>[] = [];

const summary = (id: number) => ({
    id, session_id: `call_${id}`, title: `Meeting ${id}`, platform: "meet",
    app_name: "Chrome", url: "", created_at: "2026-09-30T12:00:00Z",
    duration_ms: id === 1 ? 120000 : 60000, category: "General",
    speaker_count: 1, snippet_preview: "", action_item_count: 0, has_audio: true,
});
const makeDetail = (id: number, text = "Original transcript") => ({
    ...summary(id), audio_path: `/meeting${id}.webm`, transcript_raw: text,
    summary: [text], action_items: [],
    turns: [{ speaker_id: "you", speaker_name: "You", start_ms: 0, end_ms: 1000, channel: 0, text }],
});
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({
    ...realCore,
    convertFileSrc: (path: string) => path,
    invoke: async (cmd: string, args: any) => {
        calls.push({ cmd, args });
        const override = invokeOverride?.(cmd, args);
        if (override) return override;
        if (cmd === "list_meetings") return [summary(1), summary(2)];
        if (cmd === "get_meeting_platform_counts") return [];
        if (cmd === "get_meeting_detail") {
            return args.meetingId === 1 ? pendingDetail ?? detail : makeDetail(args.meetingId);
        }
        throw new Error(cmd);
    },
}));
const realEvent = { ...(await import("@tauri-apps/api/event")) };
mock.module("@tauri-apps/api/event", () => ({
    ...realEvent,
    listen: async (name: string, callback: (event: any) => void) => {
        listeners.set(name, callback);
        const active = subscriptions.get(name) ?? new Set();
        active.add(callback);
        subscriptions.set(name, active);
        const unlisten = () => {
            active.delete(callback);
            if (listeners.get(name) === callback) {
                const remaining = [...active];
                if (remaining.length) listeners.set(name, remaining[remaining.length - 1]);
                else listeners.delete(name);
            }
        };
        if (deferListeners) await new Promise<void>(resolve => registrations.push(resolve));
        return unlisten;
    },
}));
let savePath: string | null = null;
const saveRequests: any[] = [];
const realDialog = { ...(await import("@tauri-apps/plugin-dialog")) };
mock.module("@tauri-apps/plugin-dialog", () => ({
    ...realDialog,
    save: async (options: any) => { saveRequests.push(options); return savePath; },
}));
(globalThis as any).window = { innerWidth: 1000 };

const audios: FakeAudio[] = [];
class FakeAudio {
    currentTime = 0;
    duration = 120;
    paused = true;
    playbackRate = 1;
    ontimeupdate!: () => void;
    onended!: () => void;
    onerror!: () => void;
    constructor(public src: string) { audios.push(this); }
    pause() { this.paused = true; }
    async play() { this.paused = false; await (playPlans.shift() ?? Promise.resolve()); }
}
(globalThis as any).Audio = FakeAudio;
const { MeetingsPanel } = await import("../../src/components/MeetingsPanel");
const props = {
    activeMeeting: null, isRecording: false, isDualChannelRecording: false,
    dualLevels: { mic: 0, system: 0 }, onStartDualRecording: () => {}, onStopRecording: () => {},
};
const nodes = (root: any): any[] => {
    if (root == null || typeof root !== "object") return [];
    if (Array.isArray(root)) return root.flatMap(nodes);
    return [root, ...nodes(root.props?.children)];
};
const byTest = (root: any, id: string) => nodes(root).find(node => node.props?.["data-testid"] === id);
const textOf = (root: any): string => {
    if (root == null || typeof root === "boolean") return "";
    if (typeof root !== "object") return String(root);
    if (Array.isArray(root)) return root.map(textOf).join(" ");
    return textOf(root.props?.children);
};
const select = (root: any, id: number) => {
    const card = nodes(root).find(node => node.key === String(id) && String(node.props?.className).includes("meeting-list-card"));
    expect(card).toBeDefined();
    card.props.onClick();
};
const finished = () => listeners.get("meeting-processing-finished")!({ payload: { meeting_id: 1, error: null } });
const render = () => renderHook(() => MeetingsPanel(props));

beforeEach(() => {
    calls.length = 0;
    listeners.clear();
    audios.length = 0;
    pendingDetail = null;
    detail = makeDetail(1);
    invokeOverride = null;
    deferListeners = false;
    registrations.length = 0;
    subscriptions.clear();
    playPlans.length = 0;
});

test("completion refreshes the selected meeting without opening a review popup", async () => {
    const { handle, settle } = render();
    await settle();
    expect(textOf(handle.current)).toContain("Original transcript");
    detail = makeDetail(1, "Continued transcript plus new speech");
    finished();
    listeners.get("meeting-processing-complete")!({ payload: { meeting_id: 1, imported: true } });
    await settle();
    const review = nodes(handle.current).find(node => typeof node.props?.onViewInCatalog === "function");
    expect(review.props.meetingId).toBeNull();
    await settle();
    expect(textOf(handle.current)).toContain("Continued transcript plus new speech");
});

test("the completion event alone refreshes existing detail", async () => {
    const { handle, settle } = render();
    await settle();
    detail = makeDetail(1, "Newly completed transcript");
    listeners.get("meeting-processing-complete")!({ payload: { meeting_id: 1 } });
    await settle();
    expect(textOf(handle.current)).toContain("Newly completed transcript");
});

test("switching recordings resets the playback position and duration", async () => {
    const { handle, settle } = render();
    await settle();
    byTest(handle.current, "meetings-audio-play-btn").props.onClick();
    await settle();
    audios[0].currentTime = 32;
    audios[0].ontimeupdate();
    await settle();
    select(handle.current, 2);
    await settle();
    const scrubber = byTest(handle.current, "meetings-audio-scrubber");
    byTest(handle.current, "meetings-audio-play-btn").props.onClick();
    await settle();
    expect(audios[1].src).toBe("/meeting2.webm");
    expect(audios[1].currentTime).toBe(0);
    expect(scrubber.props.value).toBe(0);
    expect(scrubber.props.max).toBe(60);
});

test("refreshing the same recording retains its playback position", async () => {
    const { handle, settle } = render();
    await settle();
    byTest(handle.current, "meetings-audio-play-btn").props.onClick();
    await settle();
    audios[0].currentTime = 32;
    audios[0].ontimeupdate();
    await settle();
    finished();
    await settle();
    byTest(handle.current, "meetings-audio-play-btn").props.onClick();
    await settle();
    expect(audios[1].currentTime).toBe(32);
});

test("a continued audio file resets the player even with the same meeting ID", async () => {
    const { handle, settle } = render();
    await settle();
    byTest(handle.current, "meetings-audio-play-btn").props.onClick();
    await settle();
    audios[0].currentTime = 32;
    audios[0].ontimeupdate();
    await settle();
    detail = { ...makeDetail(1), audio_path: "/continued.webm", duration_ms: 180000 };
    finished();
    await settle();
    byTest(handle.current, "meetings-audio-play-btn").props.onClick();
    await settle();
    expect(audios[1].src).toBe("/continued.webm");
    expect(audios[1].currentTime).toBe(0);
    expect(byTest(handle.current, "meetings-audio-scrubber").props.max).toBe(180);
});

test("switching away and back loads the changed transcript", async () => {
    const { handle, settle } = render();
    await settle();
    detail = makeDetail(1, "New confirmed transcript");
    select(handle.current, 2);
    await settle();
    select(handle.current, 1);
    await settle();
    expect(textOf(handle.current)).toContain("New confirmed transcript");
    expect(calls.filter(call => call.cmd === "get_meeting_detail")).toHaveLength(3);
});

test("a late detail response cannot overwrite a newer selection", async () => {
    let resolveOld!: (value: any) => void;
    pendingDetail = new Promise(resolve => { resolveOld = resolve; });
    const { handle, settle } = render();
    await settle();
    select(handle.current, 2);
    await settle();
    resolveOld(makeDetail(1, "Stale first meeting"));
    await settle();
    expect(textOf(handle.current)).not.toContain("Stale first meeting");
    expect(byTest(handle.current, "meetings-detail-title-input").props.value).toBe("Meeting 2");
});

test("processing errors preserve the saved transcript", async () => {
    const { handle, settle } = render();
    await settle();
    listeners.get("meeting-processing-finished")!({ payload: { meeting_id: null, error: "Transcription failed; recording retained" } });
    await settle();
    expect(textOf(handle.current)).toContain("Transcription failed; recording retained");
    expect(textOf(handle.current)).toContain("Original transcript");
});

test("late callbacks from an old audio player cannot affect its replacement", async () => {
    const { handle, settle } = render();
    await settle();
    byTest(handle.current, "meetings-audio-play-btn").props.onClick();
    await settle();
    select(handle.current, 2);
    await settle();
    byTest(handle.current, "meetings-audio-play-btn").props.onClick();
    await settle();
    audios[0].currentTime = 99;
    audios[0].ontimeupdate();
    audios[0].onerror();
    await settle();
    expect(byTest(handle.current, "meetings-audio-scrubber").props.value).toBe(0);
    expect(byTest(handle.current, "meetings-audio-play-btn").props.title).toBe("Pause Audio");
});

test("late listener registrations are removed after unmount", async () => {
    deferListeners = true;
    const view = render();
    await view.settle();
    view.unmount();
    registrations.forEach(resolve => resolve());
    await view.settle();
    expect([...subscriptions.values()].every(active => active.size === 0)).toBe(true);
    expect(listeners.size).toBe(0);
});

test("strict effect replay leaves exactly one subscription per event", async () => {
    const view = renderHook(() => MeetingsPanel(props), { strictEffects: true });
    await view.settle();
    expect([...subscriptions.values()].map(active => active.size)).toEqual([1, 1, 1, 1]);
    detail = makeDetail(1, "After strict effect replay");
    listeners.get("meeting-processing-complete")!({ payload: { meeting_id: 1 } });
    await view.settle();
    expect(textOf(view.handle.current)).toContain("After strict effect replay");
    view.unmount();
    await view.settle();
    expect([...subscriptions.values()].every(active => active.size === 0)).toBe(true);
});

test("unmount pauses the recording and isolated speaker preview", async () => {
    detail.turns = [{ ...detail.turns[0], channel: 1, speaker_id: "guest", speaker_name: "Guest", snippet_path: "/guest.wav" }];
    const view = render();
    await view.settle();
    byTest(view.handle.current, "meetings-audio-play-btn").props.onClick();
    byTest(view.handle.current, "meetings-play-snippet-btn-0").props.onClick();
    await view.settle();
    expect(audios).toHaveLength(2);
    view.unmount();
    expect(audios.every(audio => audio.paused)).toBe(true);
});

test("a rejected old play promise cannot stop a newer recording", async () => {
    let rejectOld!: (error: Error) => void;
    playPlans.push(new Promise((_resolve, reject) => { rejectOld = reject; }));
    const view = render();
    await view.settle();
    byTest(view.handle.current, "meetings-audio-play-btn").props.onClick();
    await view.settle();
    select(view.handle.current, 2);
    await view.settle();
    byTest(view.handle.current, "meetings-audio-play-btn").props.onClick();
    await view.settle();
    rejectOld(new Error("Playback superseded"));
    await view.settle();
    expect(byTest(view.handle.current, "meetings-audio-play-btn").props.title).toBe("Pause Audio");
    expect(byTest(view.handle.current, "meeting-audio-missing")).toBeUndefined();
});

test("playback failure leaves the transcript visible and reports missing audio", async () => {
    const view = render();
    await view.settle();
    playPlans.push(Promise.reject(new Error("Audio file unavailable")));
    byTest(view.handle.current, "meetings-audio-play-btn").props.onClick();
    await view.settle();
    expect(byTest(view.handle.current, "meeting-audio-missing")).toBeDefined();
    expect(textOf(view.handle.current)).toContain("Original transcript");
    expect(byTest(view.handle.current, "meetings-audio-play-btn")).toBeUndefined();
});

test("failed refresh retains saved detail and the next completion can recover", async () => {
    const view = render();
    await view.settle();
    pendingDetail = Promise.reject(new Error("Database busy"));
    finished();
    await view.settle();
    expect(textOf(view.handle.current)).toContain("Original transcript");
    pendingDetail = null;
    detail = makeDetail(1, "Recovered transcript");
    finished();
    await view.settle();
    expect(textOf(view.handle.current)).toContain("Recovered transcript");
});

test("newer same-ID detail refresh wins when responses arrive out of order", async () => {
    const view = render();
    await view.settle();
    let resolveOld!: (value: any) => void;
    pendingDetail = new Promise(resolve => { resolveOld = resolve; });
    finished();
    await view.settle();
    pendingDetail = null;
    detail = makeDetail(1, "Newest continuation");
    finished();
    await view.settle();
    resolveOld(makeDetail(1, "Outdated continuation"));
    await view.settle();
    expect(textOf(view.handle.current)).toContain("Newest continuation");
    expect(textOf(view.handle.current)).not.toContain("Outdated continuation");
});

test("an older search response cannot replace a newer filtered catalog", async () => {
    const view = render();
    await view.settle();
    let resolveOld!: (value: any) => void;
    const old = new Promise(resolve => { resolveOld = resolve; });
    invokeOverride = (cmd, args) => cmd === "list_meetings"
        ? args.search === "old" ? old : Promise.resolve([summary(2)]) : undefined;
    byTest(view.handle.current, "meetings-search-input").props.onChange({ target: { value: "old" } });
    await view.settle();
    byTest(view.handle.current, "meetings-search-input").props.onChange({ target: { value: "new" } });
    await view.settle();
    resolveOld([summary(1)]);
    await view.settle();
    expect(byTest(view.handle.current, "meetings-detail-title-input").props.value).toBe("Meeting 2");
    const cards = nodes(view.handle.current).filter(node => String(node.props?.className).includes("meeting-list-card"));
    expect(cards.map(card => card.key)).toEqual(["2"]);
});

test("no-speech completion explains that nothing was saved", async () => {
    const view = render();
    await view.settle();
    listeners.get("meeting-processing-started")!({ payload: { title: "Silent call", platform: "meet" } });
    await view.settle();
    expect(byTest(view.handle.current, "meetings-processing-status")).toBeDefined();
    listeners.get("meeting-processing-finished")!({ payload: { meeting_id: null, error: null } });
    await view.settle();
    expect(byTest(view.handle.current, "meetings-processing-status")).toBeUndefined();
    expect(textOf(view.handle.current)).toContain("Nothing was saved: no speech");
});

test("Record in person opens the in-person section, which replaces the button", async () => {
    let opened = 0;
    const withButton = renderHook(() => MeetingsPanel({ ...props, onRecordInPerson: () => { opened += 1; } }));
    await withButton.settle();
    byTest(withButton.handle.current, "meetings-record-in-person-btn").props.onClick();
    expect(opened).toBe(1);
    const section = { type: "div", props: { "data-testid": "in-person-section" }, key: null };
    const withSection = renderHook(() => MeetingsPanel({ ...props, onRecordInPerson: () => {}, inPerson: section as any }));
    await withSection.settle();
    expect(byTest(withSection.handle.current, "in-person-section")).toBeDefined();
    expect(byTest(withSection.handle.current, "meetings-record-in-person-btn")).toBeUndefined();
});

// ── Toolbar: delete needs confirming, copy / export report failures ──────────
const { exportFileName } = await import("../../src/components/MeetingsPanel");
let clipboardFails = false;
const copied: string[] = [];
Object.defineProperty(globalThis.navigator, "clipboard", {
    configurable: true,
    value: { writeText: async (t: string) => { if (clipboardFails) throw new Error("Document is not focused"); copied.push(t); } },
});

test("Delete asks first; only the confirming click deletes the meeting", async () => {
    const { handle, settle } = render();
    await settle();
    await byTest(handle.current, "meetings-delete-btn").props.onClick();
    await settle();
    expect(calls.filter(c => c.cmd === "delete_meeting")).toHaveLength(0);
    expect(textOf(byTest(handle.current, "meetings-delete-confirm-btn"))).toContain("Delete meeting?");
    byTest(handle.current, "meetings-delete-cancel-btn").props.onClick();
    await settle();
    expect(byTest(handle.current, "meetings-delete-confirm-btn")).toBeUndefined();

    invokeOverride = (cmd) => cmd === "delete_meeting" ? Promise.resolve(null) : undefined;
    await byTest(handle.current, "meetings-delete-btn").props.onClick();
    await settle();
    await byTest(handle.current, "meetings-delete-confirm-btn").props.onClick();
    await settle();
    expect(calls.filter(c => c.cmd === "delete_meeting").map(c => c.args)).toEqual([{ meetingId: 1 }]);
});

test("a pending delete question does not carry over to another meeting", async () => {
    const { handle, settle } = render();
    await settle();
    await byTest(handle.current, "meetings-delete-btn").props.onClick();
    await settle();
    select(handle.current, 2);
    await settle();
    select(handle.current, 1);
    await settle();
    expect(byTest(handle.current, "meetings-delete-confirm-btn")).toBeUndefined();
});

test("a failed delete is shown and the meeting stays", async () => {
    invokeOverride = (cmd) => cmd === "delete_meeting" ? Promise.reject("database is locked") : undefined;
    const { handle, settle } = render();
    await settle();
    await byTest(handle.current, "meetings-delete-btn").props.onClick();
    await settle();
    await byTest(handle.current, "meetings-delete-confirm-btn").props.onClick();
    await settle();
    expect(textOf(byTest(handle.current, "meetings-toolbar-error"))).toContain("Couldn't delete this meeting: database is locked");
    expect(textOf(handle.current)).toContain("Original transcript");
});

test("Copy Transcript copies speaker-labelled turns; a refused clipboard says so instead of 'Copied'", async () => {
    copied.length = 0;
    clipboardFails = false;
    const { handle, settle } = render();
    await settle();
    await byTest(handle.current, "meetings-copy-transcript-btn").props.onClick();
    await settle();
    expect(copied).toEqual(["[0:00] You:\nOriginal transcript"]);
    expect(textOf(byTest(handle.current, "meetings-copy-transcript-btn"))).toContain("Copied");

    clipboardFails = true;
    const second = render();
    await second.settle();
    await byTest(second.handle.current, "meetings-copy-transcript-btn").props.onClick();
    await second.settle();
    expect(textOf(byTest(second.handle.current, "meetings-copy-transcript-btn"))).toContain("Copy Transcript");
    expect(textOf(byTest(second.handle.current, "meetings-toolbar-error"))).toContain("Couldn't copy the transcript");
    clipboardFails = false;
});

test("Export saves the Markdown notes where the user chooses; Cancel writes nothing; a failure is shown", async () => {
    saveRequests.length = 0;
    invokeOverride = (cmd) => cmd === "save_meeting_notes" ? Promise.resolve(null) : undefined;
    const { handle, settle } = render();
    await settle();

    savePath = null; // the user cancels the dialog
    await byTest(handle.current, "meetings-export-markdown-btn").props.onClick();
    await settle();
    expect(saveRequests[0]).toEqual({ defaultPath: "Meeting_1_transcript.md", filters: [{ name: "Markdown", extensions: ["md"] }] });
    expect(calls.filter(c => c.cmd === "save_meeting_notes")).toHaveLength(0);

    savePath = "/Users/me/Documents/Meeting_1_transcript.md";
    await byTest(handle.current, "meetings-export-markdown-btn").props.onClick();
    await settle();
    expect(calls.find(c => c.cmd === "save_meeting_notes")!.args).toEqual({ meetingId: 1, format: "markdown", path: savePath });
    expect(textOf(byTest(handle.current, "meetings-export-markdown-btn"))).toContain("Exported");

    invokeOverride = (cmd) => cmd === "save_meeting_notes" ? Promise.reject("Couldn't write /Volumes/USB/x.md: Read-only file system") : undefined;
    await byTest(handle.current, "meetings-export-markdown-btn").props.onClick();
    await settle();
    expect(textOf(byTest(handle.current, "meetings-toolbar-error"))).toContain("Couldn't export the notes: Couldn't write /Volumes/USB/x.md: Read-only file system");
});

test("export file names drop characters no file system accepts", () => {
    expect(exportFileName("In-person meeting on Oct 7, 2026", "markdown")).toBe("In-person_meeting_on_Oct_7,_2026_transcript.md");
    expect(exportFileName("Budget: Q3/Q4 <draft>?", "text")).toBe("Budget_Q3_Q4_draft_transcript.txt");
    expect(exportFileName("  ", "json")).toBe("meeting_transcript.json");
});

test("merging is offered on turns and persisted undo reloads the selected meeting", async () => {
    detail.turns = [
        { speaker_id: "a", speaker_name: "Dana", start_ms: 0, end_ms: 1000, channel: 1, text: "First" },
        { speaker_id: "b", speaker_name: "Speaker 2", start_ms: 1000, end_ms: 2000, channel: 1, text: "Second" },
    ];
    detail.merges = [{ id: 7, from_speaker_name: "Speaker 3", into_speaker_name: "Dana" }];
    const view = renderHook(() => MeetingsPanel(props)); await view.settle();
    invokeOverride = (cmd) => ["merge_meeting_speakers", "undo_meeting_speaker_merge"].includes(cmd) ? Promise.resolve() : undefined;
    const select = nodes(view.handle.current).find(n => n.props?.["aria-label"] === "Merge Speaker 2 into");
    await select.props.onChange({ target: { value: "a" } }); await view.settle();
    expect(calls.some(c => c.cmd === "merge_meeting_speakers" && c.args.meetingId === 1 && c.args.fromSpeakerId === "b" && c.args.intoSpeakerId === "a")).toBe(true);
    await byTest(view.handle.current,"meetings-undo-merge-btn").props.onClick(); await view.settle();
    expect(calls.some(c => c.cmd === "undo_meeting_speaker_merge" && c.args.mergeId === 7 && c.args.meetingId === 1)).toBe(true);
    expect(byTest(view.handle.current,"meetings-cycle-sample-btn-0")).toBeUndefined();
});
test("clicking transcript seeks and highlights all speakers active at that position", async () => {
    detail.turns = [
        { speaker_id: "a", speaker_name: "Dana", start_ms: 5000, end_ms: 8000, channel: 1, text: "First" },
        { speaker_id: "b", speaker_name: "Sam", start_ms: 6000, end_ms: 9000, channel: 1, text: "Second" },
    ];
    const view = renderHook(() => MeetingsPanel(props)); await view.settle();
    const seek = nodes(view.handle.current).find(n=>n.props?.["aria-label"]?.startsWith("Jump to 0:06:"));
    expect(seek).toBeDefined(); seek.props.onClick(); await view.settle();
    expect(byTest(view.handle.current,"meetings-turn-0").props["aria-current"]).toBe("true");
    expect(byTest(view.handle.current,"meetings-turn-1").props["aria-current"]).toBe("true");
});

test("re-separation requires confirmation and passes the selected model and count", async () => {
  const busy:boolean[]=[];invokeOverride=(cmd)=>cmd === "reseparate_meeting_speakers" ? Promise.resolve() : undefined;
  const view=renderHook(()=>MeetingsPanel({...props,modelId:"tiny",useGpu:false,onImportProcessingChange:v=>busy.push(v)}));await view.settle();
  const hint=nodes(view.handle.current).find(n=>n.props?.["aria-label"] === "Speaker count hint");
  hint.props.onChange({target:{value:"3"}});await view.settle();
  byTest(view.handle.current,"meetings-reseparate-btn").props.onClick();await view.settle();
  expect(calls.some(c=>c.cmd === "reseparate_meeting_speakers")).toBe(false);
  byTest(view.handle.current,"meetings-reseparate-btn").props.onClick();await view.settle();
  expect(calls.find(c=>c.cmd === "reseparate_meeting_speakers")?.args).toEqual({meetingId:1,expectedSpeakers:3,modelId:"tiny",useGpu:false});expect(busy).toEqual([true,false]);
});

test("processing stage updates and completion clears the status", async () => {
    const view = render(); await view.settle();
    listeners.get("meeting-processing-started")!({ payload: { title: "Import", platform: "Imported recording" } }); await view.settle();
    listeners.get("meeting-processing-progress")!({ payload: { stage: "Converting and joining recordings" } }); await view.settle();
    expect(textOf(byTest(view.handle.current, "meetings-processing-status"))).toContain("Converting and joining recordings");
    listeners.get("meeting-processing-finished")!({ payload: { meeting_id: 1, error: null } }); await view.settle();
    expect(byTest(view.handle.current, "meetings-processing-status")).toBeUndefined();
});

test("meeting tabs separate reading from speaker tools and reset for another meeting", async () => {
    const view = render();
    await view.settle();
    const panel = (name: string) => nodes(view.handle.current).find(node => node.props?.id === `meetings-panel-${name}`);
    expect(panel("transcript").props.hidden).toBe(false);
    expect(panel("speakers").props.hidden).toBe(true);
    byTest(view.handle.current, "meetings-tab-speakers").props.onClick();
    await view.settle();
    expect(panel("speakers").props.hidden).toBe(false);
    expect(panel("transcript").props.hidden).toBe(true);
    byTest(view.handle.current, "meetings-tab-tools").props.onClick();
    await view.settle();
    expect(panel("tools").props.hidden).toBe(false);
    expect(nodes(panel("tools")).some(node => node.props?.["data-testid"] === "meetings-delete-btn")).toBe(true);
    select(view.handle.current, 2);
    await view.settle();
    expect(panel("transcript").props.hidden).toBe(false);
    view.unmount();
});

test("repeated transcript turns produce one speaker management card per person", async () => {
    detail.turns = [
        { speaker_id:"alex", speaker_name:"Alex", channel:1, start_ms:0, end_ms:1000, text:"One", snippet_path:"/alex.wav" },
        { speaker_id:"alex", speaker_name:"Alex", channel:1, start_ms:2000, end_ms:3000, text:"Two", snippet_path:"/alex.wav" },
        { speaker_id:"sam", speaker_name:"Sam", channel:1, start_ms:4000, end_ms:5000, text:"Three", snippet_path:"/sam.wav" },
    ];
    const view = render();
    await view.settle();
    expect(nodes(view.handle.current).filter(node => node.props?.className === "meeting-speaker-card").length).toBe(2);
    const transcript = nodes(view.handle.current).find(node => node.props?.id === "meetings-panel-transcript");
    expect(nodes(transcript).filter(node => String(node.props?.className).includes("turn-bubble")).length).toBe(2);
    expect(textOf(transcript)).toContain("One");
    expect(textOf(transcript)).toContain("Two");
    expect(nodes(transcript).some(node => node.props?.["data-testid"]?.startsWith("meetings-rename-speaker-btn"))).toBe(false);
    view.unmount();
});

test("leaving Meetings stops its audio and revokes stale media-button playback", async () => {
    let visible = true;
    const view = renderHook(() => MeetingsPanel({ ...props, isVisible:visible }));
    await view.settle();
    byTest(view.handle.current,"meetings-audio-play-btn").props.onClick();
    await view.settle();
    const audio = audios[audios.length-1];
    expect(audio.paused).toBe(false);
    visible=false;view.rerender();await view.settle();
    expect(audio.paused).toBe(true);
    audio.paused=false;(audio as any).onplay({});
    expect(audio.paused).toBe(true);
    view.unmount();
});

test("closing during a pending play request does not report the recording as missing", async () => {
    let reject!: (error:Error)=>void;
    playPlans.push(new Promise<void>((_,failure)=>{reject=failure;}));
    let visible=true;
    const view=renderHook(()=>MeetingsPanel({...props,isVisible:visible}));
    await view.settle();
    byTest(view.handle.current,"meetings-audio-play-btn").props.onClick();
    visible=false;view.rerender();await view.settle();
    reject(Object.assign(new Error("The play request was interrupted by pause"),{name:"AbortError"}));
    await view.settle();
    visible=true;view.rerender();await view.settle();
    expect(byTest(view.handle.current,"meetings-audio-play-btn")).toBeDefined();
    expect(textOf(view.handle.current)).not.toContain("Audio file missing");
    view.unmount();
});
