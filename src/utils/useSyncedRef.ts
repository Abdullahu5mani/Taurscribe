import { useRef, useEffect } from "react";

/**
 * Returns a ref that stays in sync with `value` after every render.
 * Eliminates the repeated pattern:
 *   const fooRef = useRef(foo);
 *   useEffect(() => { fooRef.current = foo; }, [foo]);
 */
export function useSyncedRef<T>(value: T) {
    const ref = useRef(value);
    // No dep-array: runs after every render so the ref is always current.
    // This is intentional — the ref must reflect the latest closure value
    // available to async handlers that read it after a state update.
    useEffect(() => {
        ref.current = value;
    });
    return ref;
}
