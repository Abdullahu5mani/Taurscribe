import { useEffect, type RefObject } from "react";

/**
 * Calls `onDismiss` when the user presses anywhere outside `ref` (or the
 * window loses focus). Backdrops alone miss areas like the title bar, whose
 * mousedown handler starts a window drag. Presses inside `ignoreSelector`
 * (the button that toggles the popover) are left to that button.
 */
export function useDismissOnOutside(
    ref: RefObject<HTMLElement | null>,
    onDismiss: () => void,
    { enabled = true, ignoreSelector }: { enabled?: boolean; ignoreSelector?: string } = {},
) {
    useEffect(() => {
        if (!enabled) return;
        const onPointerDown = (e: PointerEvent) => {
            const target = e.target as Element | null;
            if (!target || ref.current?.contains(target)) return;
            if (ignoreSelector && target.closest?.(ignoreSelector)) return;
            onDismiss();
        };
        const onBlur = () => onDismiss();
        document.addEventListener("pointerdown", onPointerDown, true);
        window.addEventListener("blur", onBlur);
        return () => {
            document.removeEventListener("pointerdown", onPointerDown, true);
            window.removeEventListener("blur", onBlur);
        };
    }, [ref, onDismiss, enabled, ignoreSelector]);
}
