import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "../frontend/hookHarness";
import { testTrace } from "./trace";

const trace = testTrace("transcript-feed");
function deferred<T>() {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>(r => { resolve = r; });
    return { promise, resolve };
}
let respond: (cmd: string, args: any) => unknown;
const calls: { cmd: string; args: any }[] = [];
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({ ...realCore, invoke: async (cmd: string, args: any) => {
    calls.push({ cmd, args }); trace("invoke.start", { cmd, args });
    const value = await respond(cmd, args); trace("invoke.finish", { cmd, value }); return value;
} }));
const { TranscriptFeed } = await import("../../src/components/TranscriptFeed");
const nodes = (root: any): any[] => root == null || typeof root !== "object" ? []
    : Array.isArray(root) ? root.flatMap(nodes) : [root, ...nodes(root.props?.children)];
const text = (root: any): string => root == null || typeof root === "boolean" ? ""
    : typeof root !== "object" ? String(root) : Array.isArray(root) ? root.map(text).join(" ") : text(root.props?.children);
const ids = (root: any) => nodes(root).filter(n => n.props?.role === "article").map(n => n.props.id);
const row = (id: number, engine = "granite") => ({ id, engine, transcript: `Synthetic transcript ${id}`, created_at: "2026-10-07T12:00:00Z",
    duration_ms: 1000, grammar_llm_used: false, processing_time_ms: 123, audio_source: "microphone", kind: "dictation" });
function fixture() {
    let key = 0;
    const h = renderHook(() => (TranscriptFeed as any).type({ refreshKey: key, isRecording: false, isPaused: false, isProcessingTranscript: false, latestLatency: null }));
    return { ...h, refresh: () => { key++; h.rerender(); } };
}
beforeEach(() => { calls.length = 0; respond = () => []; });

test("an old history response cannot replace a newer refresh", async () => {
    const old = deferred<any[]>(), recent = deferred<any[]>();
    let reads = 0; respond = () => ++reads === 1 ? old.promise : recent.promise;
    const h = fixture(); h.refresh(); await h.settle();
    recent.resolve([row(2)]); await h.settle();
    old.resolve([row(1)]); await h.settle(); trace("final", ids(h.handle.current));
    expect(ids(h.handle.current)).toEqual(["transcript-card-2"]);
});

test("an in-flight history read cannot resurrect a successfully deleted transcript", async () => {
    respond = () => [row(1), row(2)];
    const h = fixture(); await h.settle();
    const pending = deferred<any[]>(); respond = (cmd) => cmd === "list_transcript_history" ? pending.promise : null;
    h.refresh(); await h.settle();
    const button = nodes(h.handle.current).find(n => n.props?.id === "transcript-delete-1");
    await button.props.onClick({ stopPropagation() {} }); await h.settle();
    pending.resolve([row(1), row(2)]); await h.settle(); trace("final.after-delete", ids(h.handle.current));
    expect(ids(h.handle.current)).toEqual(["transcript-card-2"]);
});

test("Qwen3 history displays its actual recognition engine", async () => {
    respond = () => [row(1, "qwen3")];
    const h = fixture(); await h.settle();
    const badge = nodes(h.handle.current).find(n => n.props?.className === "feed-badge feed-badge-engine--qwen3");
    expect(text(badge)).toBe("Qwen3");
});

test("clipboard rejection must not display a successful Copied state", async () => {
    respond = () => [row(1)];
    const previous = Object.getOwnPropertyDescriptor(globalThis, "navigator");
    Object.defineProperty(globalThis, "navigator", { configurable: true, value: { clipboard: { writeText: async () => { throw new Error("clipboard unavailable"); } } } });
    try {
        const h = fixture(); await h.settle();
        await nodes(h.handle.current).find(n => n.props?.id === "transcript-copy-1").props.onClick({ stopPropagation() {} });
        await h.settle();
        const button = nodes(h.handle.current).find(n => n.props?.id === "transcript-copy-1");
        expect(button.props["aria-label"]).toBe("Copy transcript to clipboard");
        expect(nodes(h.handle.current).some(n => n.props?.role === "alert" && text(n).includes("clipboard unavailable"))).toBe(true);
    } finally {
        if (previous) Object.defineProperty(globalThis, "navigator", previous);
        else delete (globalThis as any).navigator;
    }
});
