import { useState, useRef } from "react";

/**
 * Manages the header status message that temporarily shows a message
 * and then reverts to the scrolling ticker.
 */
export function useHeaderStatus() {
    const [headerStatusMessage, setHeaderStatusMessage] = useState<string | null>(null);
    const [headerStatusIsProcessing, setHeaderStatusIsProcessing] = useState(false);
    const headerStatusTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    const setHeaderStatus = (message: string, durationMs = 3200, isProcessing = false) => {
        if (headerStatusTimeoutRef.current) clearTimeout(headerStatusTimeoutRef.current);
        setHeaderStatusMessage(message);
        setHeaderStatusIsProcessing(isProcessing);
        headerStatusTimeoutRef.current = setTimeout(() => {
            setHeaderStatusMessage(null);
            setHeaderStatusIsProcessing(false);
            headerStatusTimeoutRef.current = null;
        }, durationMs);
    };

    return { headerStatusMessage, headerStatusIsProcessing, setHeaderStatus };
}
