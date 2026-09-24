import { useLayoutEffect, useState, type RefObject } from "react";

export interface IndicatorBox {
    left: number;
    width: number;
    /** False until the first measurement, so the bar doesn't slide in from 0. */
    ready: boolean;
}

/**
 * Tracks the position of the active item (matched by `activeSelector`) inside
 * `groupRef`, so a single indicator can slide between items instead of each
 * item drawing its own. Re-measures when the group or its items resize.
 */
export function useSlidingIndicator(
    groupRef: RefObject<HTMLElement | null>,
    activeSelector: string,
    activeKey: unknown,
): IndicatorBox {
    const [box, setBox] = useState<IndicatorBox>({ left: 0, width: 0, ready: false });

    useLayoutEffect(() => {
        const group = groupRef.current;
        if (!group) return;
        const measure = () => {
            const active = group.querySelector<HTMLElement>(activeSelector);
            if (!active) return;
            setBox((prev) => {
                const next = { left: active.offsetLeft, width: active.offsetWidth, ready: true };
                return prev.left === next.left && prev.width === next.width && prev.ready ? prev : next;
            });
        };
        measure();
        const ro = new ResizeObserver(measure);
        ro.observe(group);
        group.querySelectorAll<HTMLElement>(":scope > *").forEach((el) => ro.observe(el));
        return () => ro.disconnect();
    }, [groupRef, activeSelector, activeKey]);

    return box;
}
