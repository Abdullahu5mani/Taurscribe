import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "./hookHarness";

// Settings → About → Factory reset: the confirm step, Cancel, the native
// reset, the dev-build path (store cleared, page reloaded) and failures.
let reset: () => Promise<boolean>;
const calls: string[] = [];
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({
    ...realCore,
    invoke: async (cmd: string) => {
        calls.push(cmd);
        if (cmd === "factory_reset_app_data") return reset();
        if (cmd === "get_platform") return "macos";
        if (cmd === "get_gpu_report" || cmd === "get_hardware_diagnostics") return null;
        throw new Error(`${cmd} not mocked`);
    },
}));
const realApp = { ...(await import("@tauri-apps/api/app")) };
mock.module("@tauri-apps/api/app", () => ({ ...realApp, getVersion: async () => "2.0.0" }));
let storeCleared = false;
const realStore = { ...(await import("@tauri-apps/plugin-store")) };
mock.module("@tauri-apps/plugin-store", () => ({
    ...realStore,
    Store: { load: async () => ({ clear: async () => { storeCleared = true; }, save: async () => {}, close: async () => {} }) },
}));

const timers: (() => void)[] = [];
let reloaded = 0;
(globalThis as any).window = {
    location: { reload: () => { reloaded++; } },
    setTimeout: (fn: () => void) => { timers.push(fn); return timers.length; },
};
const { AboutTab } = await import("../../src/components/settings/AboutTab");

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

beforeEach(() => {
    calls.length = 0;
    timers.length = 0;
    reloaded = 0;
    storeCleared = false;
    reset = async () => true;
});

const open = async () => {
    const view = renderHook(() => AboutTab());
    await view.settle();
    return view;
};

test("the first click only asks for confirmation; Cancel backs out without resetting", async () => {
    const { handle, settle } = await open();
    await byTest(handle.current, "factory-reset-btn").props.onClick();
    await settle();
    expect(calls).not.toContain("factory_reset_app_data");
    expect(textOf(handle.current)).toContain("This action is permanent.");

    byTest(handle.current, "factory-reset-cancel-btn").props.onClick();
    await settle();
    expect(byTest(handle.current, "factory-reset-btn")).toBeDefined();
    expect(byTest(handle.current, "factory-reset-confirm-btn")).toBeUndefined();
    expect(calls).not.toContain("factory_reset_app_data");
});

test("confirming resets once, shows Resetting…, and ignores a second click", async () => {
    let finish!: (v: boolean) => void;
    reset = () => new Promise((r) => { finish = r; });
    const { handle, settle } = await open();
    await byTest(handle.current, "factory-reset-btn").props.onClick();
    await settle();
    const confirm = byTest(handle.current, "factory-reset-confirm-btn");
    void confirm.props.onClick();
    await settle();
    expect(textOf(handle.current)).toContain("Resetting…");
    expect(byTest(handle.current, "factory-reset-confirm-btn").props.disabled).toBe(true);
    void byTest(handle.current, "factory-reset-confirm-btn").props.onClick();
    finish(true);
    await settle();
    expect(calls.filter((c) => c === "factory_reset_app_data")).toHaveLength(1);
});

test("a dev build (no restart) clears the settings store and reloads into setup", async () => {
    reset = async () => false;
    const { handle, settle } = await open();
    await byTest(handle.current, "factory-reset-btn").props.onClick();
    await settle();
    await byTest(handle.current, "factory-reset-confirm-btn").props.onClick();
    await settle();
    expect(storeCleared).toBe(true);
    expect(reloaded).toBe(1);
});

test("a refused reset (recording running) is shown and the button works again", async () => {
    reset = async () => { throw "Stop the current recording before running a factory reset."; };
    const { handle, settle } = await open();
    await byTest(handle.current, "factory-reset-btn").props.onClick();
    await settle();
    await byTest(handle.current, "factory-reset-confirm-btn").props.onClick();
    await settle();
    expect(textOf(handle.current)).toMatch(/Reset failed:\s+Stop the current recording/);
    expect(byTest(handle.current, "factory-reset-confirm-btn").props.disabled).toBe(false);
    expect(storeCleared).toBe(false);
});

test("a restart that never happens tells the user the reset will retry on next launch", async () => {
    const { handle, settle } = await open();
    await byTest(handle.current, "factory-reset-btn").props.onClick();
    await settle();
    await byTest(handle.current, "factory-reset-confirm-btn").props.onClick();
    await settle();
    expect(timers).toHaveLength(1);
    timers[0]();
    await settle();
    expect(textOf(handle.current)).toContain("the pending reset will retry on next launch");
    expect(byTest(handle.current, "factory-reset-btn")).toBeDefined();
});
