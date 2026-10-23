import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import { renderHook } from "./hookHarness";
import type { MeetingInfo } from "../../src/components/MeetingBanner";
import { MEETING_KEYS } from "../../src/components/settings/types";

let enabled = true, delay = 3, showBanner = true, starts = 0, recording = false;
let currentMeeting: MeetingInfo | null;
let registration: Promise<void> | undefined;
const listeners = new Map<string, Set<(event: any) => void>>();
const storeListeners = new Set<(key: string, value: unknown) => void>();
const timers = new Map<number, () => void>();
let nextTimer = 0;
const originalInterval = globalThis.setInterval, originalClear = globalThis.clearInterval;
const core = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({ ...core, invoke: async (cmd: string) => {
  expect(cmd).toBe("get_auto_record_meetings");
  return enabled;
} }));
const events = { ...(await import("@tauri-apps/api/event")) };
mock.module("@tauri-apps/api/event", () => ({ ...events, listen: async (name: string, fn: (event: any) => void) => {
  if (registration) await registration;
  const set = listeners.get(name) ?? new Set(); listeners.set(name, set); set.add(fn);
  return () => set.delete(fn);
} }));
const store = { ...(await import("@tauri-apps/plugin-store")) };
mock.module("@tauri-apps/plugin-store", () => ({ ...store, Store: { load: async () => ({
  get: async (key: string) => key === MEETING_KEYS.showBanner ? showBanner : delay,
  onChange: async (fn: (key: string, value: unknown) => void) => { storeListeners.add(fn); return () => storeListeners.delete(fn); },
}) } }));
const { MeetingBanner } = await import("../../src/components/MeetingBanner");
const nodes = (root: any): any[] => root == null || typeof root !== "object" ? [] : Array.isArray(root) ? root.flatMap(nodes) : [root, ...nodes(root.props?.children)];
const button = (root: any, id: string) => nodes(root).find(n => n.props?.id === id);
const tick = (seconds: number) => { for (let i = 0; i < seconds; i++) [...timers.values()].forEach(fn => fn()); };

// Honour App's actual JSX key when mounting the component: testing the banner
// alone missed a parent key that discarded dismissal state on title updates.
const app = ts.createSourceFile("App.tsx", readFileSync(new URL("../../src/App.tsx", import.meta.url), "utf8"), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
let keyExpression: string | undefined;
function visit(node: ts.Node) {
  if (ts.isJsxSelfClosingElement(node) && node.tagName.getText(app) === "MeetingBanner") {
    const key = node.attributes.properties.find(a => ts.isJsxAttribute(a) && a.name.getText(app) === "key") as ts.JsxAttribute | undefined;
    if (key?.initializer && ts.isJsxExpression(key.initializer)) keyExpression = key.initializer.expression?.getText(app);
  }
  ts.forEachChild(node, visit);
}
visit(app);
function mount() {
  const key = () => keyExpression ? runInNewContext(keyExpression, { activeMeeting: currentMeeting }) : "unkeyed";
  const render = () => renderHook(() => MeetingBanner({ meeting: currentMeeting, isRecording: recording, onStartDualRecording: () => starts++ }));
  let lastKey = key(), view = render();
  return {
    get current() { return view.handle.current; },
    settle: () => view.settle(),
    rerender() {
      const next = key();
      if (next !== lastKey) { view.unmount(); view = render(); lastKey = next; }
      else view.rerender();
    },
    unmount: () => view.unmount(),
  };
}

beforeEach(() => {
  enabled = showBanner = true; delay = 3; starts = 0; recording = false;
  registration = undefined;
  currentMeeting = { pid: 123, app_name: "Zoom", title: "Team call", url: "", platform: "zoom", confidence: 100, should_record: true, is_using_mic: true, is_playing_audio: true };
  timers.clear(); listeners.clear(); storeListeners.clear();
  globalThis.setInterval = ((fn: () => void) => { timers.set(++nextTimer, fn); return nextTimer; }) as any;
  globalThis.clearInterval = ((id: number) => { timers.delete(id); }) as any;
});
afterEach(() => { globalThis.setInterval = originalInterval; globalThis.clearInterval = originalClear; });

test("dismiss hides the notification and cancels recording", async () => {
  const view = mount(); await view.settle();
  button(view.current, "meeting-banner-dismiss-btn").props.onClick(); await view.settle();
  expect(view.current).toBeNull();
  tick(5); await view.settle(); expect(starts).toBe(0);
});

test("a title update preserves dismissal through App's parent key", async () => {
  const view = mount(); await view.settle();
  button(view.current, "meeting-banner-dismiss-btn").props.onClick(); await view.settle();
  currentMeeting = { ...currentMeeting!, title: "(1) Team call" }; view.rerender(); await view.settle();
  tick(5); await view.settle(); expect(starts).toBe(0);
  expect(view.current).toBeNull();
  currentMeeting = null; view.rerender(); await view.settle();
  currentMeeting = { pid: 123, app_name: "Zoom", title: "Next call", url: "", platform: "zoom", confidence: 100, should_record: true, is_using_mic: true, is_playing_audio: true };
  view.rerender(); await view.settle(); tick(3); await view.settle(); expect(starts).toBe(1);
});

test("turning auto-record off cancels a pending countdown immediately", async () => {
  const view = mount(); await view.settle(); expect(timers.size).toBe(1);
  enabled = false;
  listeners.get("auto-record-meetings-changed")?.forEach(fn => fn({ payload: false }));
  await view.settle(); expect(timers.size).toBe(0);
  tick(5); await view.settle(); expect(starts).toBe(0);
});

test("the final backend check blocks recording even if a settings event is missed", async () => {
  const view = mount(); await view.settle(); enabled = false;
  tick(3); await view.settle(); expect(starts).toBe(0);
});

test("starting and stopping a recording cannot revive an earlier countdown", async () => {
  const view = mount(); await view.settle(); tick(1);
  recording = true; view.rerender(); await view.settle();
  recording = false; view.rerender(); await view.settle();
  tick(5); await view.settle(); expect(starts).toBe(0);
});

test("banner visibility changes apply during the current call", async () => {
  enabled = false;
  const view = mount(); await view.settle(); expect(view.current).not.toBeNull();
  showBanner = false; storeListeners.forEach(fn => fn(MEETING_KEYS.showBanner, false)); await view.settle();
  expect(view.current).toBeNull();
  showBanner = true; storeListeners.forEach(fn => fn(MEETING_KEYS.showBanner, true)); await view.settle();
  expect(view.current).not.toBeNull();
});

test("unmount cancels pending work and removes subscriptions", async () => {
  const view = mount(); await view.settle(); view.unmount(); await Promise.resolve();
  tick(5); expect(starts).toBe(0); expect(timers.size).toBe(0);
  expect(storeListeners.size).toBe(0);
  expect([...listeners.values()].every(set => set.size === 0)).toBe(true);
});

test("startup settings hydration before listener registration is still observed", async () => {
  let register!: () => void;
  registration = new Promise(resolve => { register = resolve; });
  enabled = false;
  const view = mount(); await view.settle(); expect(timers.size).toBe(0);
  enabled = true; // Startup hydrates saved settings before the listener exists.
  register(); await view.settle(); tick(3); await view.settle(); expect(starts).toBe(1);
});

test("late listener registration is cleaned up after unmount", async () => {
  let register!: () => void;
  registration = new Promise(resolve => { register = resolve; });
  const view = mount(); await view.settle(); view.unmount(); register();
  for (let i = 0; i < 5; i++) await Promise.resolve();
  expect([...listeners.values()].every(set => set.size === 0)).toBe(true);
  tick(5); expect(starts).toBe(0);
});

test("manual recording before settings subscription is ready consumes auto-record", async () => {
  let register!: () => void;
  registration = new Promise(resolve => { register = resolve; });
  const view = mount(); await view.settle();
  button(view.current, "meeting-banner-record-btn").props.onClick(); expect(starts).toBe(1);
  register(); await view.settle(); tick(5); await view.settle(); expect(starts).toBe(1);
});
