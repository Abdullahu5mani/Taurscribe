import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "./hookHarness";

const deferred = <T,>() => {
    let resolve!: (value: T) => void, reject!: (error: Error) => void;
    const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
    return { promise, resolve, reject };
};
const detail = (id: number) => ({
    id, title: `Review ${id}`, category: "General", platform: "meet", duration_ms: 10000,
    summary: ["Approved summary"], transcript_raw: "Speech", audio_path: null,
    action_items: [{ id: "stable-action", task: "Send notes", assignee: "Alice", status: "todo" }],
    turns: ["a", "b"].map((speaker_id, index) => ({ speaker_id, speaker_name: index ? "Bob" : "Alice",
        channel: 1, start_ms: index * 1000, end_ms: (index + 1) * 1000, text: "Speech",
        snippet_path: `/${id}-${speaker_id}.wav` })),
});
let invokeOverride: ((cmd: string, args: any) => Promise<any> | undefined) | null = null;
const calls: { cmd: string; args: any }[] = [];
const closed: number[] = [], viewed: number[] = [];
let modelInstalled = true;
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({ ...realCore, convertFileSrc: (path: string) => path,
    invoke: async (cmd: string, args: any) => {
        calls.push({ cmd, args });
        const override = invokeOverride?.(cmd, args);
        if (override) return override;
        if (cmd === "get_meeting_detail") return detail(args.meetingId);
        if (cmd === "get_download_status") return args.modelIds.map((id: string) => ({ id, downloaded: modelInstalled }));
        if (cmd === "save_meeting_review") return null;
        throw new Error(`Unexpected command: ${cmd}`);
    },
}));
const audios: FakeAudio[] = [];
const playPlans: Promise<void>[] = [];
class FakeAudio {
    paused = true;
    onended!: () => void;
    onerror!: () => void;
    constructor(public src: string) { audios.push(this); }
    pause() { this.paused = true; }
    async play() { this.paused = false; await (playPlans.shift() ?? Promise.resolve()); }
}
(globalThis as any).Audio = FakeAudio;
const { MeetingReviewModal } = await import("../../src/components/MeetingReviewModal");
const nodes = (root: any): any[] => root == null || typeof root !== "object" ? []
    : Array.isArray(root) ? root.flatMap(nodes) : [root, ...nodes(root.props?.children)];
const byTest = (root: any, id: string) => {
    const found = nodes(root).find(node => node.props?.["data-testid"] === id);
    expect(found).toBeDefined();
    return found;
};
const render = (initial = 1) => {
    let id: number | null = initial;
    const view = renderHook(() => MeetingReviewModal({ meetingId: id,
        onClose: () => { closed.push(id!); }, onViewInCatalog: meeting => { viewed.push(meeting); } }));
    return { ...view, select(next: number | null) { id = next; view.rerender(); } };
};
const saves = () => calls.filter(call => call.cmd === "save_meeting_review");

beforeEach(() => {
    invokeOverride = null;
    modelInstalled = true;
    calls.length = closed.length = viewed.length = audios.length = playPlans.length = 0;
});

test("review saves category, stable action IDs, notes and only changed trimmed names atomically", async () => {
    const view = render(); await view.settle();
    byTest(view.handle.current, "meeting-review-title-input").props.onChange({ target: { value: "  Approved title  " } });
    byTest(view.handle.current, "meeting-review-category-Planning").props.onClick();
    byTest(view.handle.current, "meeting-review-speaker-name-input-a").props.onChange({ target: { value: "  Abdullah  " } });
    byTest(view.handle.current, "meeting-review-action-done-stable-action").props.onChange();
    await view.settle();
    await byTest(view.handle.current, "meeting-review-save-btn").props.onClick();
    expect(saves()).toHaveLength(1);
    expect(saves()[0].args).toEqual({ meetingId: 1, title: "Approved title", category: "Planning",
        summary: ["Approved summary"], actionItems: [{ ...detail(1).action_items[0], status: "done" }],
        speakerRenames: [{ speakerId: "a", newName: "Abdullah" }] });
    expect(closed).toEqual([1]); expect(viewed).toEqual([1]);
});

test("failed review save keeps edits visible and retries the same payload", async () => {
    const view = render(); await view.settle();
    byTest(view.handle.current, "meeting-review-title-input").props.onChange({ target: { value: "Keep my edits" } });
    await view.settle();
    invokeOverride = cmd => cmd === "save_meeting_review" ? Promise.reject(new Error("Disk full")) : undefined;
    await byTest(view.handle.current, "meeting-review-save-btn").props.onClick(); await view.settle();
    expect(byTest(view.handle.current, "meeting-review-title-input").props.value).toBe("Keep my edits");
    byTest(view.handle.current, "meeting-review-save-error");
    expect(closed).toEqual([]); expect(viewed).toEqual([]);
    invokeOverride = null;
    await byTest(view.handle.current, "meeting-review-save-btn").props.onClick(); await view.settle();
    expect(saves()[1].args).toEqual(saves()[0].args);
    expect(closed).toEqual([1]);
});

test("two save clicks in the same render issue exactly one write", async () => {
    const save = deferred<void>();
    invokeOverride = cmd => cmd === "save_meeting_review" ? save.promise : undefined;
    const view = render(); await view.settle();
    const click = byTest(view.handle.current, "meeting-review-save-btn").props.onClick;
    const first = click(), second = click();
    expect(saves()).toHaveLength(1);
    save.resolve(); await Promise.all([first, second]);
});

test("saving blocks button and backdrop dismissal until the write finishes", async () => {
    const save = deferred<void>();
    invokeOverride = cmd => cmd === "save_meeting_review" ? save.promise : undefined;
    const view = render(); await view.settle();
    const pending = byTest(view.handle.current, "meeting-review-save-btn").props.onClick(); await view.settle();
    expect(byTest(view.handle.current, "meeting-review-dismiss-btn").props.disabled).toBe(true);
    view.handle.current!.props.onClick();
    byTest(view.handle.current, "meeting-review-close-modal-btn").props.onClick();
    expect(closed).toEqual([]);
    save.resolve(); await pending;
});

test("a stale detail response cannot populate a different meeting review", async () => {
    const old = deferred<any>();
    invokeOverride = (cmd, args) => cmd === "get_meeting_detail" && args.meetingId === 1 ? old.promise : undefined;
    const view = render(); await view.settle();
    view.select(2); await view.settle();
    old.resolve(detail(1)); await view.settle();
    expect(byTest(view.handle.current, "meeting-review-title-input").props.value).toBe("Review 2");
});

test("an old save finishing cannot dismiss or navigate a newer review", async () => {
    const save = deferred<void>();
    invokeOverride = cmd => cmd === "save_meeting_review" ? save.promise : undefined;
    const view = render(); await view.settle();
    const pending = byTest(view.handle.current, "meeting-review-save-btn").props.onClick();
    view.select(2); await view.settle();
    save.resolve(); await pending; await view.settle();
    expect(closed).toEqual([]); expect(viewed).toEqual([]);
    expect(byTest(view.handle.current, "meeting-review-title-input").props.value).toBe("Review 2");
});

test("an old save failure cannot add its error to a newer review", async () => {
    const save = deferred<void>();
    invokeOverride = cmd => cmd === "save_meeting_review" ? save.promise : undefined;
    const view = render(); await view.settle();
    const pending = byTest(view.handle.current, "meeting-review-save-btn").props.onClick();
    view.select(2); await view.settle();
    save.reject(new Error("Old meeting failed")); await pending; await view.settle();
    expect(nodes(view.handle.current).some(node => node.props?.["data-testid"] === "meeting-review-save-error")).toBe(false);
});

test("a stale speaker preview callback cannot clear the new speaker's playback", async () => {
    const view = render(); await view.settle();
    byTest(view.handle.current, "meeting-review-play-snippet-btn-a").props.onClick(); await view.settle();
    byTest(view.handle.current, "meeting-review-play-snippet-btn-b").props.onClick(); await view.settle();
    audios[0].onended(); await view.settle();
    expect(byTest(view.handle.current, "meeting-review-play-snippet-btn-b").props.className).toContain("playing");
    expect(audios[0].paused).toBe(true);
    view.unmount(); expect(audios[1].paused).toBe(true);
});

test("a rejected old preview promise cannot stop the current speaker", async () => {
    const play = deferred<void>(); playPlans.push(play.promise);
    const view = render(); await view.settle();
    byTest(view.handle.current, "meeting-review-play-snippet-btn-a").props.onClick(); await view.settle();
    byTest(view.handle.current, "meeting-review-play-snippet-btn-b").props.onClick(); await view.settle();
    play.reject(new Error("Superseded")); await view.settle();
    expect(byTest(view.handle.current, "meeting-review-play-snippet-btn-b").props.className).toContain("playing");
});

test("switching reviews pauses the previous preview and resets its UI", async () => {
    const view = render(); await view.settle();
    byTest(view.handle.current, "meeting-review-play-snippet-btn-a").props.onClick(); await view.settle();
    view.select(2); await view.settle();
    expect(audios[0].paused).toBe(true);
    expect(byTest(view.handle.current, "meeting-review-play-snippet-btn-a").props.className).not.toContain("playing");
});

test("blank speaker edits stay blank while unchanged and empty renames are omitted", async () => {
    const view = render(); await view.settle();
    byTest(view.handle.current, "meeting-review-speaker-name-input-a").props.onChange({ target: { value: "" } });
    await view.settle();
    expect(byTest(view.handle.current, "meeting-review-speaker-name-input-a").props.value).toBe("");
    await byTest(view.handle.current, "meeting-review-save-btn").props.onClick();
    expect(saves()[0].args.speakerRenames).toEqual([]);
});

test("uninstalled recognition model is explained without blocking per-meeting review", async () => {
    modelInstalled = false;
    const view = render(); await view.settle();
    byTest(view.handle.current, "meeting-review-speaker-model-note");
    await byTest(view.handle.current, "meeting-review-save-btn").props.onClick();
    expect(saves()).toHaveLength(1);
});
