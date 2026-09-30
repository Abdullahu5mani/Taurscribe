import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "./hookHarness";
import type { FileHistoryRecord } from "../../src/types/fileHistory";

type Call = { cmd: string; args?: Record<string, unknown> };
let calls: Call[] = [];
let respond: (cmd: string, args?: Record<string, unknown>) => unknown = () => [];
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({
    ...realCore,
    invoke: async (cmd: string, args?: Record<string, unknown>) => {
        calls.push({ cmd, args });
        return await respond(cmd, args);
    },
}));

const { useFileHistory, FILE_HISTORY_PAGE_SIZE } = await import("../../src/hooks/useFileHistory");
const { saveFileHistory, importMethodLabel } = await import("../../src/utils/fileHistory");
const { FileHistory } = await import("../../src/components/FileHistory");
const record = (id: number): FileHistoryRecord => ({
    id, created_at: "2026-09-30T14:01:02.000Z", transcript: `Saved transcript ${id}`,
    engine: "qwen3", model_id: "qwen3-asr-0.6b", audio_source: "notes.wav",
    source_path: "/missing/original/notes.wav", import_method: "drop",
    duration_ms: 1200, processing_time_ms: 200, grammar_llm_used: false, kind: "file",
});

beforeEach(() => { calls = []; respond = () => []; });

test("a fresh mount restores file transcripts without reading original audio", async () => {
    respond = () => [record(1)];
    const { handle, settle } = renderHook(() => useFileHistory(0));
    await settle();
    expect(handle.current.records[0].transcript).toBe("Saved transcript 1");
    expect(calls).toEqual([{ cmd: "list_transcript_history", args: { kind: "file", limit: 25, offset: 0 } }]);
    expect(handle.current.loading).toBe(false);
    expect(handle.current.hasMore).toBe(false);
});

test("history loads bounded pages and deduplicates shifted offset pages", async () => {
    respond = (_cmd, args) => args?.offset === 0
        ? Array.from({ length: FILE_HISTORY_PAGE_SIZE }, (_, index) => record(100 - index))
        : [record(76), record(75), record(74)];
    const { handle, settle } = renderHook(() => useFileHistory(0));
    await settle();
    expect(handle.current.hasMore).toBe(true);
    await handle.current.loadMore();
    await settle();
    expect(handle.current.records).toHaveLength(27);
    expect(calls[1].args?.offset).toBe(25);
    expect(handle.current.hasMore).toBe(false);
});

test("a successful save refresh key reloads the first page", async () => {
    let key = 0;
    respond = () => [record(key + 1)];
    const { handle, settle, rerender } = renderHook(() => useFileHistory(key));
    await settle();
    key = 1;
    rerender();
    await settle();
    expect(handle.current.records.map(row => row.id)).toEqual([2]);
    expect(calls[1].args?.offset).toBe(0);
});

test("a stale load-more response cannot overwrite a newer refresh", async () => {
    let resolveOld!: (rows: FileHistoryRecord[]) => void;
    let refreshed = false;
    respond = (_cmd, args) => args?.offset === 25
        ? new Promise<FileHistoryRecord[]>(resolve => { resolveOld = resolve; })
        : refreshed ? [record(200)] : Array.from({ length: 25 }, (_, index) => record(100 - index));
    const { handle, settle } = renderHook(() => useFileHistory(0));
    await settle();
    const oldRequest = handle.current.loadMore();
    refreshed = true;
    await handle.current.refresh();
    await settle();
    resolveOld([record(70)]);
    await oldRequest;
    await settle();
    expect(handle.current.records.map(row => row.id)).toEqual([200]);
    expect(handle.current.loading).toBe(false);
});

test("failed page loads preserve visible transcripts and retry the same offset", async () => {
    let fail = true;
    respond = (_cmd, args) => {
        if (args?.offset === 0) return Array.from({ length: 25 }, (_, index) => record(index));
        if (fail) throw new Error("database busy");
        return [record(26)];
    };
    const { handle, settle } = renderHook(() => useFileHistory(0));
    await settle();
    await handle.current.loadMore();
    await settle();
    expect(handle.current.error).toContain("database busy");
    expect(handle.current.records).toHaveLength(25);
    fail = false;
    await handle.current.loadMore();
    await settle();
    expect(calls[2].args?.offset).toBe(25);
    expect(handle.current.error).toBeNull();
    expect(handle.current.records).toHaveLength(26);
});

test("repeated load-more clicks make only one request", async () => {
    let finish!: (rows: FileHistoryRecord[]) => void;
    respond = (_cmd, args) => args?.offset === 0
        ? Array.from({ length: 25 }, (_, index) => record(index))
        : new Promise<FileHistoryRecord[]>(resolve => { finish = resolve; });
    const { handle, settle } = renderHook(() => useFileHistory(0));
    await settle();
    const loading = handle.current.loadMore();
    await handle.current.loadMore();
    expect(calls).toHaveLength(2);
    finish([]);
    await loading;
    await settle();
});

test("save uses actual source filename and preserves model and completion time on retry", async () => {
    respond = () => null;
    const completed = {
        path: "/Users/test/Documents/Project notes 音声.m4a", name: "Project notes 音声.m4a (1)",
        transcript: "Saved words.\nSecond line.", engine: "granite", modelId: "granite-speech-5-nc",
        audioDurationMs: 5400, processingTimeMs: 450, importMethod: "drop" as const,
        completedAt: "2026-09-30T14:01:02.000Z",
    };
    await saveFileHistory(completed);
    await saveFileHistory(completed);
    expect(calls[0]).toEqual({ cmd: "save_transcript_history", args: {
        transcript: completed.transcript, engine: "granite", durationMs: 5400,
        processingTimeMs: 450, grammarLlmUsed: false, modelId: "granite-speech-5-nc",
        audioSource: "Project notes 音声.m4a", sourcePath: completed.path,
        importMethod: "drop", completedAt: completed.completedAt,
    } });
    expect(calls[1]).toEqual(calls[0]);
});

test("picker sources preserve the original Windows path", async () => {
    await saveFileHistory({ path: "C:\\Users\\test\\Voice memo.wav", transcript: "Hello", engine: "whisper", importMethod: "picker" });
    expect(calls[0].args?.audioSource).toBe("Voice memo.wav");
    expect(calls[0].args?.sourcePath).toBe("C:\\Users\\test\\Voice memo.wav");
    expect(calls[0].args?.importMethod).toBe("picker");
});

test("failed saves remain retryable and empty transcripts create no history", async () => {
    respond = () => { throw new Error("disk full"); };
    await expect(saveFileHistory({ path: "/test.wav", transcript: "Hello", engine: "whisper", importMethod: "drop" })).rejects.toThrow("disk full");
    calls = [];
    await saveFileHistory({ path: "/silence.wav", transcript: "  \n", engine: "whisper", importMethod: "picker" });
    expect(calls).toHaveLength(0);
});

test("legacy entries show unknown import method instead of guessing", () => {
    expect(importMethodLabel(null)).toBe("Unknown (older entry)");
    expect(importMethodLabel("future_method")).toBe("Unknown (older entry)");
    expect(importMethodLabel("drop")).toBe("Dragged and dropped");
    expect(importMethodLabel("picker")).toBe("Selected with file picker");
});

// Inspect the rendered element tree through the same small hook runtime used
// above. Hidden long transcripts should not become DOM children until opened.
function elements(tree: any): any[] {
    if (tree == null || typeof tree !== "object") return [];
    if (Array.isArray(tree)) return tree.flatMap(elements);
    return [tree, ...elements(tree.props?.children)];
}
function textIn(tree: any): string {
    if (tree == null || typeof tree === "boolean") return "";
    if (typeof tree === "string" || typeof tree === "number") return String(tree);
    if (Array.isArray(tree)) return tree.map(textIn).join(" ");
    return textIn(tree.props?.children);
}

test("history renders transcript contents only when the row is expanded", async () => {
    const saved = { ...record(1), transcript: "A long transcript should load only on demand." };
    respond = () => [saved];
    const { handle, settle } = renderHook(() => FileHistory({ refreshKey: 0 }));
    await settle();
    expect(textIn(handle.current)).not.toContain(saved.transcript);
    expect(textIn(handle.current)).toContain("notes.wav");
    let row = elements(handle.current).find(element => element.type === "details");
    row.props.onToggle({ currentTarget: { open: true } });
    await settle();
    expect(textIn(handle.current)).toContain(saved.transcript);
    expect(textIn(handle.current)).toContain(saved.source_path!);
    expect(textIn(handle.current)).toContain("Dragged and dropped");
    row = elements(handle.current).find(element => element.type === "details");
    row.props.onToggle({ currentTarget: { open: false } });
    await settle();
    expect(textIn(handle.current)).not.toContain(saved.transcript);
});

test("expanded legacy history remains readable and labels absent metadata honestly", async () => {
    respond = () => [{ ...record(2), source_path: null, import_method: null, model_id: null }];
    const { handle, settle } = renderHook(() => FileHistory({ refreshKey: 0 }));
    await settle();
    elements(handle.current).find(element => element.type === "details").props.onToggle({ currentTarget: { open: true } });
    await settle();
    expect(textIn(handle.current)).toContain("Saved transcript 2");
    expect(textIn(handle.current)).toContain("Not recorded for this older entry");
    expect(textIn(handle.current)).toContain("Unknown (older entry)");
});
