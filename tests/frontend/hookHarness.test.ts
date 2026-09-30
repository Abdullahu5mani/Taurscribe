import { expect, test } from "bun:test";
import { useEffect, useState } from "react";
import { renderHook } from "./hookHarness";

test("late state updates belong to their instance and cannot corrupt another test", async () => {
    const first = renderHook(() => useState(1));
    const lateSetter = first.handle.current[1];
    first.unmount();
    const second = renderHook(() => useState(20));
    lateSetter(999);
    await second.settle();
    expect(second.handle.current[0]).toBe(20);
});

test("simultaneously mounted hooks have independent state and stable setters", async () => {
    const first = renderHook(() => useState(1));
    const second = renderHook(() => useState(2));
    const setter = first.handle.current[1];
    setter(3);
    await first.settle();
    await second.settle();
    expect(first.handle.current[0]).toBe(3);
    expect(second.handle.current[0]).toBe(2);
    expect(first.handle.current[1]).toBe(setter);
});

test("effect cleanup runs before replacement and exactly once on unmount", () => {
    const events: string[] = [];
    let dep = 1;
    const view = renderHook(() => useEffect(() => {
        const captured = dep;
        events.push(`setup:${captured}`);
        return () => { events.push(`cleanup:${captured}`); };
    }, [dep]));
    dep = 2;
    view.rerender();
    view.unmount();
    view.unmount();
    expect(events).toEqual(["setup:1", "cleanup:1", "setup:2", "cleanup:2"]);
});

test("strict effect replay exercises setup cleanup setup", () => {
    const events: string[] = [];
    const view = renderHook(() => useEffect(() => {
        events.push("setup");
        return () => { events.push("cleanup"); };
    }, []), { strictEffects: true });
    expect(events).toEqual(["setup", "cleanup", "setup"]);
    view.unmount();
    expect(events).toEqual(["setup", "cleanup", "setup", "cleanup"]);
});
