import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Store } from "@tauri-apps/plugin-store";
import type { ASREngine } from "./useEngineSwitch";

export interface AutoUnloadStatus {
    timeout_seconds: number;
    remaining_seconds: number | null;
    is_loaded: boolean;
    last_activity_epoch: number;
}

export interface AutoUnloadOption {
    label: string;
    shortLabel: string;
    value: number;
    description: string;
}

export const AUTO_UNLOAD_OPTIONS: AutoUnloadOption[] = [
    { label: "Immediately", shortLabel: "⚡ Instant", value: 1, description: "Unload right after each transcription (frees VRAM instantly)" },
    { label: "5 minutes", shortLabel: "5m", value: 300, description: "Unload after 5 minutes of inactivity" },
    { label: "15 minutes", shortLabel: "15m", value: 900, description: "Unload after 15 minutes of inactivity" },
    { label: "30 minutes", shortLabel: "30m", value: 1800, description: "Unload after 30 minutes of inactivity (Recommended)" },
    { label: "1 hour", shortLabel: "1h", value: 3600, description: "Unload after 1 hour of inactivity" },
    { label: "Never", shortLabel: "Never", value: 0, description: "Keep model in memory indefinitely until manually unloaded" },
];

export function formatTimeoutLabel(seconds: number): string {
    switch (seconds) {
        case 0: return "Never";
        case 1: return "Instant";
        case 300: return "5m";
        case 900: return "15m";
        case 1800: return "30m";
        case 3600: return "1h";
        default:
            if (seconds < 60) return `${seconds}s`;
            if (seconds < 3600) return `${Math.round(seconds / 60)}m`;
            return `${Math.round(seconds / 3600)}h`;
    }
}

export function formatRemaining(seconds: number): string {
    if (seconds <= 0) return "0s";
    const mins = Math.floor(seconds / 60);
    const secs = seconds % 60;
    if (mins > 0) {
        return `${mins}m ${secs.toString().padStart(2, "0")}s`;
    }
    return `${secs}s`;
}

interface UseAutoUnloadParams {
    loadedEngine: ASREngine | null;
    setLoadedEngine: (engine: ASREngine | null) => void;
    setHeaderStatus: (msg: string, dur?: number) => void;
}

export function useAutoUnload({
    loadedEngine,
    setLoadedEngine,
    setHeaderStatus,
}: UseAutoUnloadParams) {
    const [timeoutSeconds, setTimeoutSeconds] = useState<number>(1800); // 30 min default
    const [remainingSeconds, setRemainingSeconds] = useState<number | null>(null);
    const [isMenuOpen, setIsMenuOpen] = useState(false);
    const storeRef = useRef<Store | null>(null);

    // ── Load saved preference on mount ────────────────────────────────────────
    useEffect(() => {
        let isMounted = true;
        const init = async () => {
            try {
                const store = await Store.load("settings.json");
                storeRef.current = store;
                const saved = await store.get<number>("auto_unload_timeout_secs");
                if (saved !== null && saved !== undefined && typeof saved === "number") {
                    if (isMounted) setTimeoutSeconds(saved);
                    await invoke("set_auto_unload_timeout", { seconds: saved }).catch(() => {});
                } else {
                    // Default to 1800 (30 minutes)
                    await invoke("set_auto_unload_timeout", { seconds: 1800 }).catch(() => {});
                }
            } catch (e) {
                console.warn("[AUTO-UNLOAD] Failed to load settings.json:", e);
            }
        };
        init();
        return () => {
            isMounted = false;
        };
    }, []);

    // ── Polling remaining time when model is loaded & timeout > 1 ──────────────
    useEffect(() => {
        if (!loadedEngine || timeoutSeconds <= 1) {
            setRemainingSeconds(null);
            return;
        }

        let isMounted = true;
        const checkStatus = async () => {
            try {
                const status = await invoke<AutoUnloadStatus>("get_auto_unload_status");
                if (isMounted) {
                    setRemainingSeconds(status.remaining_seconds);
                }
            } catch {
                // Ignore query errors
            }
        };

        checkStatus();
        const interval = setInterval(checkStatus, 1000);
        return () => {
            isMounted = false;
            clearInterval(interval);
        };
    }, [loadedEngine, timeoutSeconds]);

    // ── Listen for auto-unload event from Rust background watchdog ─────────────
    useEffect(() => {
        let unlisten: (() => void) | undefined;
        listen<{ timeout_seconds: number; unloaded_engines: string[] }>(
            "model-auto-unloaded",
            (event) => {
                setLoadedEngine(null);
                setRemainingSeconds(null);
                const timeoutLabel = formatTimeoutLabel(event.payload.timeout_seconds);
                const msg = event.payload.timeout_seconds === 1
                    ? "Model unloaded immediately after transcription — VRAM freed"
                    : `Model auto-unloaded after ${timeoutLabel} of inactivity — VRAM freed`;
                setHeaderStatus(msg, 6000);
            },
        ).then((fn) => {
            unlisten = fn;
        });

        return () => {
            unlisten?.();
        };
    }, [setLoadedEngine, setHeaderStatus]);

    // ── Change timeout setting ────────────────────────────────────────────────
    const updateTimeout = useCallback(
        async (newSeconds: number) => {
            setTimeoutSeconds(newSeconds);
            try {
                await invoke("set_auto_unload_timeout", { seconds: newSeconds });
                const store = storeRef.current ?? (await Store.load("settings.json"));
                storeRef.current = store;
                await store.set("auto_unload_timeout_secs", newSeconds);
                await store.save();
            } catch (e) {
                console.error("[AUTO-UNLOAD] Failed to save timeout:", e);
            }
        },
        [],
    );

    return {
        timeoutSeconds,
        remainingSeconds,
        isMenuOpen,
        setIsMenuOpen,
        updateTimeout,
    };
}
