// A minimal single-component React hook runtime, so hooks can be tested
// without a DOM or a renderer. Supports the hooks our hooks use: useState,
// useRef, useEffect, useCallback, useMemo. Effects run after each render in
// declaration order, as in React; state set during effects or async work
// triggers another render on the next `settle()`.
import { mock } from "bun:test";

type EffectSlot = { deps?: unknown[]; cleanup?: () => void };

let slots: unknown[] = [];
let cursor = 0;
let pending: (() => void)[] = [];
let dirty = false;

const depsChanged = (prev: unknown[] | undefined, next: unknown[] | undefined) =>
    !prev || !next || prev.length !== next.length || next.some((d, i) => !Object.is(d, prev[i]));

const hooks = {
    useState<T>(init: T | (() => T)) {
        const i = cursor++;
        if (!(i in slots)) slots[i] = typeof init === "function" ? (init as () => T)() : init;
        const set = (v: T | ((p: T) => T)) => {
            const next = typeof v === "function" ? (v as (p: T) => T)(slots[i] as T) : v;
            if (!Object.is(next, slots[i])) {
                slots[i] = next;
                dirty = true;
            }
        };
        return [slots[i] as T, set] as const;
    },
    useRef<T>(init: T) {
        const i = cursor++;
        if (!(i in slots)) slots[i] = { current: init };
        return slots[i] as { current: T };
    },
    useEffect(fn: () => void | (() => void), deps?: unknown[]) {
        const i = cursor++;
        const prev = slots[i] as EffectSlot | undefined;
        if (!depsChanged(prev?.deps, deps)) return;
        slots[i] = { deps, cleanup: prev?.cleanup };
        pending.push(() => {
            (slots[i] as EffectSlot).cleanup?.();
            const cleanup = fn();
            (slots[i] as EffectSlot).cleanup = typeof cleanup === "function" ? cleanup : undefined;
        });
    },
    useCallback<T>(fn: T, deps: unknown[]) {
        return hooks.useMemo(() => fn, deps);
    },
    useMemo<T>(make: () => T, deps: unknown[]) {
        const i = cursor++;
        const prev = slots[i] as { deps: unknown[]; value: T } | undefined;
        if (prev && !depsChanged(prev.deps, deps)) return prev.value;
        const value = make();
        slots[i] = { deps, value };
        return value;
    },
};

const realReact = { ...(await import("react")) };
mock.module("react", () => ({ ...realReact, ...hooks, default: { ...realReact, ...hooks } }));

/** Renders `hook` and returns a handle whose `current` is the latest result. */
export function renderHook<T>(hook: () => T) {
    slots = [];
    pending = [];
    const handle = { current: undefined as unknown as T };
    const render = () => {
        cursor = 0;
        dirty = false;
        handle.current = hook();
        const effects = pending;
        pending = [];
        effects.forEach((run) => run());
    };
    render();
    /** Lets promises resolve and re-renders until nothing changes. */
    const settle = async () => {
        for (let round = 0; round < 50; round++) {
            for (let t = 0; t < 10; t++) await Promise.resolve();
            await new Promise((r) => setTimeout(r, 0));
            if (!dirty) return;
            render();
        }
        throw new Error("hook never settled");
    };
    return { handle, settle, rerender: render };
}
