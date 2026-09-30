import { afterAll, beforeAll, expect, test } from "bun:test";

// Just enough DOM for trackScrollState: a scroller that holds attributes and
// fires scroll listeners, frames as timers, and observers we can trigger.
class FakeScroller {
    attrs = new Set<string>();
    children: unknown[] = [];
    scrollTop = 0;
    scrollHeight = 400;
    clientHeight = 100;
    private onScroll?: () => void;
    toggleAttribute(name: string, on: boolean) { if (on) this.attrs.add(name); else this.attrs.delete(name); }
    hasAttribute(name: string) { return this.attrs.has(name); }
    setAttribute(name: string) { this.attrs.add(name); }
    removeAttribute(name: string) { this.attrs.delete(name); }
    addEventListener(_: string, fn: () => void) { this.onScroll = fn; }
    removeEventListener() { this.onScroll = undefined; }
    scrollTo(top: number) { this.scrollTop = top; this.onScroll?.(); }
    get listening() { return this.onScroll !== undefined; }
}

let resized: (() => void) | undefined;
const saved: Record<string, unknown> = {};
beforeAll(() => {
    for (const k of ["requestAnimationFrame", "cancelAnimationFrame", "ResizeObserver", "MutationObserver"]) saved[k] = (globalThis as Record<string, unknown>)[k];
    Object.assign(globalThis, {
        requestAnimationFrame: (cb: () => void) => setTimeout(cb, 0),
        cancelAnimationFrame: (id: number) => clearTimeout(id),
        ResizeObserver: class { constructor(cb: () => void) { resized = cb; } observe() {} disconnect() {} },
        MutationObserver: class { observe() {} disconnect() {} },
    });
});
afterAll(() => { Object.assign(globalThis, saved); });

const { trackScrollState } = await import("../../src/utils/scrollState");
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
const state = (el: FakeScroller) => [...el.attrs].sort().join(",");

test("edges and scrolling are mirrored as attributes", async () => {
    const el = new FakeScroller();
    const stop = trackScrollState(el as unknown as HTMLElement);
    expect(state(el)).toBe("data-more");

    el.scrollTo(150);
    await wait(5);
    expect(state(el)).toBe("data-more,data-scrolled,data-scrolling");
    await wait(200);
    expect(state(el)).toBe("data-more,data-scrolled");

    el.scrollTo(300);
    await wait(200);
    expect(state(el)).toBe("data-scrolled");

    stop();
    expect(state(el)).toBe("");
    expect(el.listening).toBe(false);
});

test("content growing without a scroll shows there is more below", async () => {
    const el = new FakeScroller();
    el.scrollHeight = 100;
    const stop = trackScrollState(el as unknown as HTMLElement);
    expect(state(el)).toBe("");
    el.scrollHeight = 300;
    resized?.();
    await wait(5);
    expect(state(el)).toBe("data-more");
    stop();
});
