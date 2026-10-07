import { beforeEach, expect, mock, test } from "bun:test";
import { renderHook } from "../frontend/hookHarness";
function deferred<T>() {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>(r => { resolve = r; });
    return { promise, resolve };
}
const calls: { cmd: string; args: any }[] = [];
const listeners = new Set<Function>();
let savedGate: ReturnType<typeof deferred<number | null>>;
let registrationGate: ReturnType<typeof deferred<void>> | null;
let persisted: number | null;
const store = {
    get: async (_key: string) => savedGate.promise,
    set: async (_key: string, value: number) => { persisted = value; },
    save: async () => {},
};
const realCore = { ...(await import("@tauri-apps/api/core")) };
mock.module("@tauri-apps/api/core", () => ({ ...realCore, invoke: async (cmd: string, args: any) => {
    calls.push({ cmd, args });
    return null;
} }));
const realStore = { ...(await import("@tauri-apps/plugin-store")) };
mock.module("@tauri-apps/plugin-store", () => ({ ...realStore, Store: { load: async () => store } }));
const realEvent = { ...(await import("@tauri-apps/api/event")) };
mock.module("@tauri-apps/api/event", () => ({ ...realEvent, listen: async (_name: string, fn: Function) => {
    listeners.add(fn);
    if (registrationGate) await registrationGate.promise;
    return () => { listeners.delete(fn); };
} }));
const { useAutoUnload } = await import("../../src/hooks/useAutoUnload");
const setLoadedEngine = () => {};
const setHeaderStatus = () => {};
const fixture = () => renderHook(() => useAutoUnload({ loadedEngine: null, setLoadedEngine, setHeaderStatus }));
beforeEach(() => {
    calls.length = 0; listeners.clear(); savedGate = deferred<number | null>();
    registrationGate = null; persisted = null;
});

test("saved auto-unload preference initializes the UI and native timeout", async () => {
    const h = fixture(); savedGate.resolve(300); await h.settle();
    expect(h.handle.current.timeoutSeconds).toBe(300);
    expect(calls.map(c => c.args.seconds)).toEqual([300]);
});

test("a slow settings read cannot overwrite a newer Never selection", async () => {
    const h = fixture(); await h.settle();
    await h.handle.current.updateTimeout(0); await h.settle();
    expect(h.handle.current.timeoutSeconds).toBe(0);
    expect(persisted).toBe(0);
    savedGate.resolve(300); await h.settle();
    expect(persisted).toBe(0);
    expect(h.handle.current.timeoutSeconds).toBe(0);
    expect(calls.at(-1)?.args.seconds).toBe(0);
});

test("unmounting before listen resolves releases the auto-unload subscription", async () => {
    registrationGate = deferred<void>();
    const h = fixture(); savedGate.resolve(1800); await h.settle();
    expect(listeners.size).toBe(1);
    h.unmount(); registrationGate.resolve(); await h.settle();
    expect(listeners.size).toBe(0);
});

test("an ordinary resolved subscription is removed on unmount", async () => {
    const h = fixture(); savedGate.resolve(0); await h.settle();
    expect(listeners.size).toBe(1); h.unmount();
    expect(listeners.size).toBe(0);
});
