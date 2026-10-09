import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "./hookHarness";
import type { RecordingSource } from "../../src/utils/meetingImport";

const calls: { cmd: string; args: any }[] = [];
let picker: string[] | null = null;
let inspection: { recordings: RecordingSource[]; errors: string[] };
let importResult: Promise<number>;
const drops = new Set<(event: any) => void>();
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({ ...realCore, invoke: async (cmd: string, args: any) => {
  calls.push({ cmd, args });
  if (cmd === "inspect_meeting_recordings") return inspection;
  if (cmd === "import_meeting_recordings") return importResult;
  throw new Error(cmd);
} }));
const realDialog = { ...(await import("@tauri-apps/plugin-dialog")) };
mock.module("@tauri-apps/plugin-dialog", () => ({ ...realDialog, open: async () => picker }));
const realWebview = { ...(await import("@tauri-apps/api/webview")) };
mock.module("@tauri-apps/api/webview", () => ({ ...realWebview, getCurrentWebview: () => ({ onDragDropEvent: async (fn: any) => { drops.add(fn); return () => drops.delete(fn); } }) }));
const { MeetingRecordingImport } = await import("../../src/components/MeetingRecordingImport");
const nodes = (root: any): any[] => root == null || typeof root !== "object" ? [] : Array.isArray(root) ? root.flatMap(nodes) : [root, ...nodes(root.props?.children)];
const byTest = (root: any, id: string) => nodes(root).find(n => n.props?.["data-testid"] === id);
const byLabel = (root: any, label: string) => nodes(root).find(n => n.props?.["aria-label"] === label);
const content = (root: any): string => root == null || typeof root === "boolean" ? "" : typeof root !== "object" ? String(root) : Array.isArray(root) ? root.map(content).join(" ") : content(root.props?.children);
const source = (name: string, start: string): RecordingSource => ({ path: `/audio/${name}`, name, duration_ms: 60000,
  recorded_start: start, recorded_end: new Date(Date.parse(start) + 60000).toISOString(), timestamp_source: "Embedded timestamp",
  file_created_at: null, file_modified_at: null, offset_ms: 0 });
const fixture = () => {
  const busy: boolean[] = [], completed: number[] = [];
  let blocked = false;
  const view = renderHook(() => MeetingRecordingImport({ blocked, onProcessingChange: b => busy.push(b), onComplete: id => completed.push(id) }));
  return { ...view, busy, completed, block: () => { blocked = true; view.rerender(); } };
};
beforeEach(() => {
  calls.length = 0; drops.clear(); picker = ["/audio/later.wav", "/audio/earlier.wav"];
  inspection = { recordings: [source("later.wav", "2026-10-10T12:00:00Z"), source("earlier.wav", "2026-10-09T12:00:00Z")], errors: [] };
  importResult = Promise.resolve(47);
  (globalThis as any).window = { devicePixelRatio: 2 };
});

test("metadata sorts the initial selection; warnings allow import and manual order is sent exactly", async () => {
  const view = fixture(); await view.settle();
  await byTest(view.handle.current, "meeting-import-browse").props.onClick(); await view.settle();
  expect(content(view.handle.current)).toContain("may be different meetings");
  expect(byTest(view.handle.current, "meeting-import-submit").props.disabled).toBe(false);
  byLabel(view.handle.current, "Move later.wav earlier").props.onClick(); await view.settle();
  await byTest(view.handle.current, "meeting-import-submit").props.onClick(); await view.settle();
  expect(calls.find(c => c.cmd === "import_meeting_recordings")?.args.paths).toEqual(["/audio/later.wav", "/audio/earlier.wav"]);
  expect(view.completed).toEqual([47]); expect(view.busy).toEqual([true, false]);
});

test("failed processing preserves the draft and two immediate clicks start only one job", async () => {
  let reject!: (error: Error) => void;
  importResult = new Promise((_, r) => { reject = r; });
  const view = fixture(); await view.settle();
  await byTest(view.handle.current, "meeting-import-browse").props.onClick(); await view.settle();
  const submit = byTest(view.handle.current, "meeting-import-submit").props.onClick;
  const pending = submit(); void submit(); await view.settle();
  expect(calls.filter(c => c.cmd === "import_meeting_recordings")).toHaveLength(1);
  reject(new Error("Recording retained after ASR failure")); await pending; await view.settle();
  expect(content(view.handle.current)).toContain("Recording retained after ASR failure");
  expect(content(view.handle.current)).toContain("earlier.wav"); expect(view.completed).toEqual([]);
  expect(view.busy).toEqual([true, false]);
});

test("native drops only import inside the meeting zone, with physical pixel scaling", async () => {
  const view = fixture(); await view.settle();
  byTest(view.handle.current, "meeting-import-drop-zone").props.ref.current = { getBoundingClientRect: () => ({ left: 10, right: 100, top: 10, bottom: 100, width: 90 }) };
  const drop = (x: number) => { for (const fn of drops) fn({ payload: { type: "drop", paths: ["/audio/meeting.wav"], position: { x, y: 40 } } }); };
  drop(400); await view.settle(); expect(calls).toHaveLength(0);
  drop(40); await view.settle(); expect(calls[0].cmd).toBe("inspect_meeting_recordings");
  view.block(); drop(40); await view.settle(); expect(calls).toHaveLength(1);
});

test("unsupported selections report errors while valid recordings remain available", async () => {
  inspection.errors = ["Unsupported recording: picture.png"];
  const view = fixture(); await view.settle();
  await byTest(view.handle.current, "meeting-import-browse").props.onClick(); await view.settle();
  expect(content(view.handle.current)).toContain("picture.png");
  expect(byTest(view.handle.current, "meeting-import-submit").props.disabled).toBe(false);
});

test("unmount keeps the parent busy until backend completion and removes drop listeners", async () => {
  let resolve!: (id: number) => void;
  importResult = new Promise(r => { resolve = r; });
  const view = fixture(); await view.settle();
  await byTest(view.handle.current, "meeting-import-browse").props.onClick(); await view.settle();
  const pending = byTest(view.handle.current, "meeting-import-submit").props.onClick();
  view.unmount(); expect(view.busy).toEqual([true]); expect(drops.size).toBe(0);
  resolve(47); await pending; expect(view.busy).toEqual([true, false]); expect(view.completed).toEqual([]);
});
