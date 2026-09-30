// A minimal single-component React hook runtime, so hooks can be tested
// without a DOM or a renderer. Supports the hooks our hooks use: useState,
// useRef, useEffect, useCallback, useMemo. Effects run after each render in
// declaration order, as in React; state set during effects or async work
// triggers another render on the next `settle()`.
import { afterEach, mock } from "bun:test";

type EffectSlot = { kind: "effect"; deps?: unknown[]; cleanup?: () => void; setup: () => void | (() => void) };
type Runtime = { slots: any[]; cursor: number; pending: (() => void)[]; dirty: boolean; mounted: boolean };
let active: Runtime | undefined;
const mounted = new Set<() => void>();
afterEach(() => { for (const unmount of mounted) unmount(); });
const runtime = () => {
    if (!active) throw new Error("Hook called outside renderHook");
    return active;
};

const depsChanged = (prev: unknown[] | undefined, next: unknown[] | undefined) =>
    !prev || !next || prev.length !== next.length || next.some((d, i) => !Object.is(d, prev[i]));

const hooks = {
    useState<T>(init: T | (() => T)) {
        const r = runtime(), i = r.cursor++;
        if (!(i in r.slots)) {
            const slot = { value: typeof init === "function" ? (init as () => T)() : init, set: undefined as any };
            slot.set = (v: T | ((p: T) => T)) => {
                if (!r.mounted) return;
                const next = typeof v === "function" ? (v as (p: T) => T)(slot.value) : v;
                if (!Object.is(next, slot.value)) { slot.value = next; r.dirty = true; }
            };
            r.slots[i] = slot;
        }
        return [r.slots[i].value as T, r.slots[i].set as (v: T | ((p: T) => T)) => void] as const;
    },
    useRef<T>(init: T) {
        const r = runtime(), i = r.cursor++;
        if (!(i in r.slots)) r.slots[i] = { current: init };
        return r.slots[i] as { current: T };
    },
    useEffect(fn: () => void | (() => void), deps?: unknown[]) {
        const r = runtime(), i = r.cursor++;
        const prev = r.slots[i] as EffectSlot | undefined;
        if (!depsChanged(prev?.deps, deps)) return;
        const slot: EffectSlot = { kind: "effect", deps, cleanup: prev?.cleanup, setup: fn };
        r.slots[i] = slot;
        r.pending.push(() => {
            slot.cleanup?.();
            const cleanup = fn();
            slot.cleanup = typeof cleanup === "function" ? cleanup : undefined;
        });
    },
    useCallback<T>(fn: T, deps: unknown[]) {
        return hooks.useMemo(() => fn, deps);
    },
    useMemo<T>(make: () => T, deps: unknown[]) {
        const r = runtime(), i = r.cursor++;
        const prev = r.slots[i] as { deps: unknown[]; value: T } | undefined;
        if (prev && !depsChanged(prev.deps, deps)) return prev.value;
        const value = make();
        r.slots[i] = { deps, value };
        return value;
    },
};

const realReact = { ...(await import("react")) };
mock.module("react", () => ({ ...realReact, ...hooks, default: { ...realReact, ...hooks } }));

/** Renders `hook` and returns a handle whose `current` is the latest result. */
export function renderHook<T>(hook: () => T, options: { strictEffects?: boolean } = {}) {
    const r: Runtime = { slots: [], cursor: 0, pending: [], dirty: false, mounted: true };
    const handle = { current: undefined as unknown as T };
    const render = () => {
        if (!r.mounted) throw new Error("Cannot render an unmounted hook");
        r.cursor = 0;
        r.dirty = false;
        const previous = active;
        active = r;
        try { handle.current = hook(); } finally { active = previous; }
        const effects = r.pending;
        r.pending = [];
        effects.forEach((run) => run());
    };
    const unmount = () => {
        if (!r.mounted) return;
        r.mounted = false;
        for (const slot of r.slots) if (slot?.kind === "effect") slot.cleanup?.();
        mounted.delete(unmount);
    };
    mounted.add(unmount);
    render();
    if (options.strictEffects) {
        for (const slot of r.slots) if (slot?.kind === "effect") slot.cleanup?.();
        for (const slot of r.slots) if (slot?.kind === "effect") {
            const cleanup = slot.setup();
            slot.cleanup = typeof cleanup === "function" ? cleanup : undefined;
        }
    }
    /** Lets promises resolve and re-renders until nothing changes. */
    const settle = async () => {
        for (let round = 0; round < 50; round++) {
            for (let t = 0; t < 10; t++) await Promise.resolve();
            await new Promise((r) => setTimeout(r, 0));
            if (!r.mounted || !r.dirty) return;
            render();
        }
        throw new Error("hook never settled");
    };
    return { handle, settle, rerender: render, unmount };
}
