import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "./hookHarness";

// Voice bank: "Find more samples" (play / Add / Not them / Show more), a
// person's samples (remove, never the last), and "Is this Dana?" on a meeting.
const calls: { cmd: string; args: any }[] = [];
let suggestions: any[];
let samples: any[];
let asks: any[];
let failures: Record<string, string>;
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({
    ...realCore,
    convertFileSrc: (p: string) => p,
    invoke: async (cmd: string, args: any) => {
        calls.push({ cmd, args });
        if (failures[cmd]) throw failures[cmd];
        switch (cmd) {
            case "find_voice_samples": return suggestions.slice(0, args.limit);
            case "generate_vault_voice_sample": return "/generated/new.wav";
            case "list_voiceprints": return samples;
            case "suggest_speaker_names": return asks;
            default: return null;
        }
    },
}));
const played: string[] = [];
(globalThis as any).Audio = class { constructor(public src: string) { played.push(src); } pause() {} async play() {} onended = null; onerror = null; };
const { VoiceBankPanel, SUGGESTION_PAGE } = await import("../../src/components/VoiceBankPanel");
const { SpeakerSuggestions } = await import("../../src/components/SpeakerSuggestions");

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
const flat = (root: any) => textOf(root).replace(/\s+/g, " ").replace(/ ([%”?.]|’)/g, "$1").replace(/“ /g, "“").trim();

const suggestion = (meeting: number, speaker: string, similarity: number) => ({
    meeting_id: meeting, meeting_title: `Meeting ${meeting}`, created_at: "2026-10-01T12:00:00Z",
    speaker_id: speaker, speaker_name: "Speaker 2", clip_path: `/clips/${meeting}_${speaker}.wav`,
    speech_ms: 14_000, preview: "The budget is fine", similarity,
});
let changed: number;
beforeEach(() => {
    calls.length = 0;
    played.length = 0;
    failures = {};
    changed = 0;
    suggestions = Array.from({ length: 13 }, (_, i) => suggestion(i + 1, "speaker_remote_2", 0.9 - i * 0.03));
    samples = [
        { id: 2, clip_path: "/clips/b.wav", meeting_id: 7, meeting_title: "Planning", created_at: "2026-10-05T00:00:00Z" },
        { id: 1, clip_path: null, meeting_id: null, meeting_title: null, created_at: "2026-09-01T00:00:00Z" },
    ];
    asks = [];
});
const panel = async (mode: "find" | "samples") => {
    const view = renderHook(() => VoiceBankPanel({ personId: "p_dana", personName: "Dana", mode, onChanged: () => changed++, onClose: () => {} }));
    await view.settle();
    return view;
};

test("Find more lists the closest speakers first with their match, meeting and line", async () => {
    const { handle } = await panel("find");
    expect(calls[0]).toEqual({ cmd: "find_voice_samples", args: { personId: "p_dana", limit: SUGGESTION_PAGE } });
    const first = byTest(handle.current, "voicebank-suggestion-1-speaker_remote_2");
    expect(flat(first)).toContain("Meeting 1");
    expect(flat(first)).toContain("“The budget is fine”");
    expect(flat(first)).toContain("90% match");
    expect(flat(first)).toContain("14s of speech");
    expect(byTest(handle.current, "voicebank-show-more")).toBeDefined();
});

test("Play plays that speaker's clip; Add keeps them and removes the row; Not them is sent too", async () => {
    const { handle, settle } = await panel("find");
    byTest(handle.current, "voicebank-play-1-speaker_remote_2").props.onClick();
    expect(played).toEqual(["/clips/1_speaker_remote_2.wav"]);
    await byTest(handle.current, "voicebank-add-1-speaker_remote_2").props.onClick();
    await settle();
    expect(calls.at(-1)).toEqual({ cmd: "accept_voice_sample", args: { personId: "p_dana", meetingId: 1, speakerId: "speaker_remote_2" } });
    expect(byTest(handle.current, "voicebank-suggestion-1-speaker_remote_2")).toBeUndefined();
    expect(changed).toBe(1);

    await byTest(handle.current, "voicebank-reject-2-speaker_remote_2").props.onClick();
    await settle();
    expect(calls.at(-1)).toEqual({ cmd: "reject_voice_sample", args: { personId: "p_dana", meetingId: 2, speakerId: "speaker_remote_2" } });
    expect(byTest(handle.current, "voicebank-suggestion-2-speaker_remote_2")).toBeUndefined();
    expect(changed).toBe(1);
});

test("Show more asks for the next page", async () => {
    const { handle, settle } = await panel("find");
    byTest(handle.current, "voicebank-show-more").props.onClick();
    await settle();
    expect(calls.at(-1)!.args.limit).toBe(SUGGESTION_PAGE * 2);
    expect(nodes(handle.current).filter((n) => String(n.props?.["data-testid"]).startsWith("voicebank-suggestion-"))).toHaveLength(13);
    expect(byTest(handle.current, "voicebank-show-more")).toBeUndefined();
});

test("nothing similar and errors are explained", async () => {
    suggestions = [];
    let view = await panel("find");
    expect(flat(byTest(view.handle.current, "voicebank-no-suggestions"))).toContain("No other speakers in your meetings sound like Dana");
    failures.find_voice_samples = "The voice bank needs the Meeting speakers download (Settings → Models).";
    view = await panel("find");
    expect(flat(byTest(view.handle.current, "voicebank-error"))).toContain("Meeting speakers download");
    failures = { accept_voice_sample: "That speaker is no longer in the meeting." };
    suggestions = [suggestion(1, "speaker_remote_2", 0.8)];
    view = await panel("find");
    await byTest(view.handle.current, "voicebank-add-1-speaker_remote_2").props.onClick();
    await view.settle();
    expect(flat(byTest(view.handle.current, "voicebank-error"))).toBe("That speaker is no longer in the meeting.");
    expect(byTest(view.handle.current, "voicebank-suggestion-1-speaker_remote_2")).toBeDefined();
});

test("samples: remove asks first, and the last sample cannot be removed", async () => {
    const { handle, settle } = await panel("samples");
    expect(flat(byTest(handle.current, "voicebank-sample-2"))).toContain("Planning");
    expect(flat(byTest(handle.current, "voicebank-sample-1"))).toContain("clip missing");
    await byTest(handle.current, "voicebank-remove-2").props.onClick();
    await settle();
    expect(calls.some((c) => c.cmd === "delete_voiceprint")).toBe(false);
    expect(flat(byTest(handle.current, "voicebank-remove-2"))).toContain("Remove?");
    await byTest(handle.current, "voicebank-remove-2").props.onClick();
    await settle();
    expect(calls.at(-1)).toEqual({ cmd: "delete_voiceprint", args: { personId: "p_dana", voiceprintId: 2 } });
    expect(byTest(handle.current, "voicebank-remove-1").props.disabled).toBe(true);
    expect(changed).toBe(1);
});

test("on a meeting: 'Speaker 2 sounds like Dana' — yes labels and reloads, no is remembered", async () => {
    asks = [
        { speaker_id: "speaker_remote_2", speaker_name: "Speaker 2", person_id: "p_dana", person_name: "Dana", similarity: 0.54, clip_path: "/c.wav" },
        { speaker_id: "speaker_remote_3", speaker_name: "Speaker 3", person_id: "p_sam", person_name: "Sam", similarity: 0.49, clip_path: null },
    ];
    let labelled = 0;
    const { handle, settle } = renderHook(() => SpeakerSuggestions({ meetingId: 9, onLabelled: () => labelled++ }));
    await settle();
    expect(flat(byTest(handle.current, "speaker-suggestion-speaker_remote_2"))).toContain("Speaker 2 sounds like Dana · 54% match");
    await byTest(handle.current, "speaker-suggestion-yes-speaker_remote_2").props.onClick();
    await settle();
    expect(calls.at(-1)).toEqual({ cmd: "accept_voice_sample", args: { personId: "p_dana", meetingId: 9, speakerId: "speaker_remote_2" } });
    expect(labelled).toBe(1);
    await byTest(handle.current, "speaker-suggestion-no-speaker_remote_3").props.onClick();
    await settle();
    expect(calls.at(-1)).toEqual({ cmd: "reject_voice_sample", args: { personId: "p_sam", meetingId: 9, speakerId: "speaker_remote_3" } });
    expect(handle.current).toBeNull();
});

test("generated sample is previewed and only enrolled after explicit identity confirmation", async () => {
    const { handle, settle } = await panel("samples");
    await byTest(handle.current,"voicebank-generate-sample").props.onClick(); await settle();
    expect(calls.some(c=>c.cmd === "generate_vault_voice_sample" && c.args.personId === "p_dana")).toBe(true);
    expect(calls.some(c=>c.cmd === "add_vault_voice_sample")).toBe(false);
    byTest(handle.current,"voicebank-preview-sample").props.onClick();
    expect(played).toContain("/generated/new.wav");
    await byTest(handle.current,"voicebank-confirm-sample").props.onClick(); await settle();
    expect(calls.some(c=>c.cmd === "add_vault_voice_sample" && c.args.personId === "p_dana" && c.args.path === "/generated/new.wav")).toBe(true);
    expect(changed).toBe(1); expect(byTest(handle.current,"voicebank-confirm-sample")).toBeUndefined();
});
