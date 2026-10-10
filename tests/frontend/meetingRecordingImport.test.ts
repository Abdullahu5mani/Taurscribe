import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "./hookHarness";
import type { RecordingSource } from "../../src/utils/meetingImport";

const calls: { cmd: string; args: any }[] = [];
let picker: string[] | null = null;
let inspection: { recordings: RecordingSource[]; errors: string[] };
let pendingInspection: Promise<typeof inspection> | null = null;
let importResult: Promise<number>;
const drops = new Set<(event: any) => void>();
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({ ...realCore, invoke: async (cmd: string, args: any) => {
  calls.push({ cmd, args });
  if (cmd === "inspect_meeting_recordings") return pendingInspection ?? inspection;
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
const fixture = (dropTargetRef?: any) => {
  const busy: boolean[] = [], completed: number[] = [];
  let blocked = false;
  const view = renderHook(() => MeetingRecordingImport({ dropTargetRef, blocked, onProcessingChange: b => busy.push(b), onComplete: id => completed.push(id) }));
  return { ...view, busy, completed, block: () => { blocked = true; view.rerender(); } };
};
beforeEach(() => {
  calls.length = 0; drops.clear(); picker = ["/audio/later.wav", "/audio/earlier.wav"];
  inspection = { recordings: [source("later.wav", "2026-10-10T12:00:00Z"), source("earlier.wav", "2026-10-09T12:00:00Z")], errors: [] };
  importResult = Promise.resolve(47);
  pendingInspection = null;
  (globalThis as any).window = { devicePixelRatio: 2 };
});

test("metadata sorts the initial selection; warnings allow import and manual order is sent exactly", async () => {
  const view = fixture(); await view.settle();
  byTest(view.handle.current, "meeting-import-open").props.onClick(); await view.settle();
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
  byTest(view.handle.current, "meeting-import-open").props.onClick(); await view.settle();
  await byTest(view.handle.current, "meeting-import-browse").props.onClick(); await view.settle();
  const submit = byTest(view.handle.current, "meeting-import-submit").props.onClick;
  const pending = submit(); void submit(); await view.settle();
  expect(calls.filter(c => c.cmd === "import_meeting_recordings")).toHaveLength(1);
  reject(new Error("Recording retained after ASR failure")); await pending; await view.settle();
  expect(content(view.handle.current)).toContain("Recording retained after ASR failure");
  byTest(view.handle.current, "meeting-import-open").props.onClick(); await view.settle();
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
  byTest(view.handle.current, "meeting-import-open").props.onClick(); await view.settle();
  await byTest(view.handle.current, "meeting-import-browse").props.onClick(); await view.settle();
  expect(content(view.handle.current)).toContain("picture.png");
  expect(byTest(view.handle.current, "meeting-import-submit").props.disabled).toBe(false);
});

test("unmount keeps the parent busy until backend completion and removes drop listeners", async () => {
  let resolve!: (id: number) => void;
  importResult = new Promise(r => { resolve = r; });
  const view = fixture(); await view.settle();
  byTest(view.handle.current, "meeting-import-open").props.onClick(); await view.settle();
  await byTest(view.handle.current, "meeting-import-browse").props.onClick(); await view.settle();
  const pending = byTest(view.handle.current, "meeting-import-submit").props.onClick();
  view.unmount(); expect(view.busy).toEqual([true]); expect(drops.size).toBe(0);
  resolve(47); await pending; expect(view.busy).toEqual([true, false]); expect(view.completed).toEqual([]);
});

test("dropping anywhere in Meetings opens the modal while metadata is still loading", async () => {
  let resolve!: (result: typeof inspection) => void;
  pendingInspection = new Promise(r => { resolve = r; });
  const target = { current: { getBoundingClientRect: () => ({ left: 0, right: 600, top: 0, bottom: 600, width: 600 }) } };
  const view = fixture(target); await view.settle();
  expect(byTest(view.handle.current, "meeting-import-modal")).toBeUndefined();
  for (const fn of drops) fn({ payload: { type: "drop", paths: ["/audio/meeting.wav"], position: { x: 800, y: 400 } } });
  await view.settle();
  expect(byTest(view.handle.current, "meeting-import-modal").props.role).toBe("dialog");
  expect(content(view.handle.current)).toContain("Reading recording metadata");
  expect(calls.filter(c => c.cmd === "import_meeting_recordings")).toHaveLength(0);
  resolve(inspection); await view.settle();
  expect(content(view.handle.current)).toContain("earlier.wav");
});

test("closing the importer preserves the draft and reopening restores its selected order", async () => {
  const view = fixture(); await view.settle();
  byTest(view.handle.current, "meeting-import-open").props.onClick(); await view.settle();
  await byTest(view.handle.current, "meeting-import-browse").props.onClick(); await view.settle();
  byLabel(view.handle.current, "Move later.wav earlier").props.onClick(); await view.settle();
  byTest(view.handle.current, "meeting-import-close").props.onClick(); await view.settle();
  expect(byTest(view.handle.current, "meeting-import-modal")).toBeUndefined();
  expect(content(view.handle.current)).toContain("2 recordings ready");
  byTest(view.handle.current, "meeting-import-open").props.onClick(); await view.settle();
  expect(byLabel(view.handle.current, "Move later.wav earlier").props.disabled).toBe(true);
  expect(calls.filter(c => c.cmd === "inspect_meeting_recordings")).toHaveLength(1);
});

test("processing dismisses the modal immediately and does not reopen it at completion", async () => {
  let resolve!: (id: number) => void; importResult = new Promise(r => { resolve = r; });
  const view = fixture(); await view.settle();
  byTest(view.handle.current, "meeting-import-open").props.onClick(); await view.settle();
  await byTest(view.handle.current, "meeting-import-browse").props.onClick(); await view.settle();
  const job = byTest(view.handle.current, "meeting-import-submit").props.onClick(); await view.settle();
  expect(byTest(view.handle.current, "meeting-import-modal")).toBeUndefined();
  expect(content(view.handle.current)).toContain("background");
  expect(view.busy).toEqual([true]);
  byTest(view.handle.current, "meeting-import-open").props.onClick(); await view.settle();
  expect(byTest(view.handle.current, "meeting-import-modal")).toBeUndefined();
  resolve(47); await job; await view.settle();
  expect(byTest(view.handle.current, "meeting-import-modal")).toBeUndefined();
  expect(view.completed).toEqual([47]);
  expect(view.busy).toEqual([true, false]);
});

test("import sends optional people hint and invalid counts preserve the draft", async () => {
  const view=fixture(); await view.settle(); byTest(view.handle.current,"meeting-import-open").props.onClick(); await view.settle();
  await byTest(view.handle.current,"meeting-import-browse").props.onClick();await view.settle();
  byLabel(view.handle.current,"About how many people?").props.onChange({target:{value:"2.5"}});await view.settle();
  await byTest(view.handle.current,"meeting-import-submit").props.onClick();await view.settle();
  expect(calls.some(c=>c.cmd === "import_meeting_recordings")).toBe(false);expect(content(view.handle.current)).toContain("1–32");
  byLabel(view.handle.current,"About how many people?").props.onChange({target:{value:"4"}});await view.settle();
  await byTest(view.handle.current,"meeting-import-submit").props.onClick();await view.settle();
  expect(calls.find(c=>c.cmd === "import_meeting_recordings")?.args.expectedSpeakers).toBe(4);
});

test("hidden Meetings does not capture native file drops", async () => {
  const target = { current: { getBoundingClientRect: () => ({ left: 0, right: 600, top: 0, bottom: 600, width: 600 }) } };
  const view = renderHook(() => MeetingRecordingImport({ isVisible: false, dropTargetRef: target, onComplete: () => {} }));
  await view.settle();
  for (const fn of drops) fn({ payload: { type: "drop", paths: ["/audio/a.wav"], position: { x: 100, y: 100 } } });
  await view.settle(); expect(calls).toHaveLength(0);
  expect(byTest(view.handle.current, "meeting-import-modal")).toBeUndefined();
  view.unmount();
});
