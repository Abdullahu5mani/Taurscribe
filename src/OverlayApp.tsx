import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import "./OverlayApp.css";

type Phase =
    | "recording"
    | "paused"
    | "transcribing"
    | "correcting"
    | "done"
    | "too_short"
    | "paste_failed"
    | "cancelled"
    | "no_model"
    | "model_loading"
    | "nothing_heard";

interface Payload {
    phase: Phase | "hidden";
    text?: string;
    ms?: number;
    engine?: string | null;
}

const BAR_COUNT = 17;
const ATTACK = 0.35;
const DECAY = 0.12;
const OVERLAY_WIDTH = 228;
const OVERLAY_HEIGHT = 42;

function formatElapsed(ms: number) {
    const totalSeconds = Math.max(0, Math.floor(ms / 1000));
    const minutes = Math.floor(totalSeconds / 60);
    const seconds = totalSeconds % 60;
    return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

function formatLatency(ms: number | null) {
    if (ms == null) return "--";
    return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`;
}

function getStatusLabel(phase: Phase) {
    switch (phase) {
        case "recording":
            return "Listening";
        case "paused":
            return "Paused";
        case "transcribing":
            return "Transcribing";
        case "correcting":
            return "Correcting";
        case "done":
            return "Done";
        case "too_short":
            return "Too short";
        case "paste_failed":
            return "Paste failed";
        case "cancelled":
            return "Discarded";
        case "no_model":
            return "No model";
        case "model_loading":
            return "Loading model";
        case "nothing_heard":
            return "Nothing heard";
    }
}

export function OverlayApp() {
    const [phase, setPhase] = useState<Phase>("recording");
    const [latencyMs, setLatencyMs] = useState<number | null>(null);
    const [elapsedMs, setElapsedMs] = useState(0);
    const [levels, setLevels] = useState<number[]>(() => Array(BAR_COUNT).fill(0));

    const smoothedRef = useRef<number[]>(Array(BAR_COUNT).fill(0));
    const sessionStartedAtRef = useRef(Date.now());
    const pauseStartedAtRef = useRef<number | null>(null);
    const pausedTotalMsRef = useRef(0);
    const previousPhaseRef = useRef<Phase | "hidden">("hidden");
    const isOverlayActiveRef = useRef(false);

    useEffect(() => {
        const win = getCurrentWindow();
        const size = new LogicalSize(OVERLAY_WIDTH, OVERLAY_HEIGHT);

        const applySize = async () => {
            await win.setMinSize(null).catch(() => {});
            await win.setMaxSize(null).catch(() => {});
            await win.setSize(new LogicalSize(1, 1)).catch(() => {});
            await new Promise((r) => setTimeout(r, 16));
            await win.setSize(size).catch(() => {});
            await win.setMinSize(size).catch(() => {});
            await win.setMaxSize(size).catch(() => {});

            if (isOverlayActiveRef.current) {
                await new Promise((r) => setTimeout(r, 30));
                invoke("show_overlay").catch(() => {});
            }
        };

        applySize();
    }, []);

    useEffect(() => {
        let unlisten: (() => void) | undefined;
        listen<Payload>("overlay-state", (event) => {
            const payload = event.payload;
            if (payload.phase === "hidden") {
                isOverlayActiveRef.current = false;
                previousPhaseRef.current = "hidden";
                return;
            }

            isOverlayActiveRef.current = true;

            if (typeof payload.ms === "number") {
                setLatencyMs(payload.ms);
            } else if (payload.phase !== "done") {
                setLatencyMs(null);
            }

            if (payload.phase === "recording") {
                if (previousPhaseRef.current === "paused" && pauseStartedAtRef.current) {
                    pausedTotalMsRef.current += Date.now() - pauseStartedAtRef.current;
                    pauseStartedAtRef.current = null;
                } else if (previousPhaseRef.current !== "recording") {
                    sessionStartedAtRef.current = Date.now();
                    pausedTotalMsRef.current = 0;
                    pauseStartedAtRef.current = null;
                    setElapsedMs(0);
                    smoothedRef.current = Array(BAR_COUNT).fill(0);
                    setLevels(Array(BAR_COUNT).fill(0));
                }
            }

            if (payload.phase === "paused" && previousPhaseRef.current !== "paused") {
                pauseStartedAtRef.current = Date.now();
            }

            if (payload.phase !== "paused" && previousPhaseRef.current === "paused" && pauseStartedAtRef.current) {
                pausedTotalMsRef.current += Date.now() - pauseStartedAtRef.current;
                pauseStartedAtRef.current = null;
            }

            previousPhaseRef.current = payload.phase;
            setPhase(payload.phase);
        }).then((fn) => { unlisten = fn; });

        return () => {
            if (unlisten) unlisten();
        };
    }, []);

    useEffect(() => {
        if (phase !== "recording" && phase !== "paused") return;

        const tick = () => {
            const pauseMs = phase === "paused" && pauseStartedAtRef.current
                ? Date.now() - pauseStartedAtRef.current
                : 0;
            const elapsed = Date.now() - sessionStartedAtRef.current - pausedTotalMsRef.current - pauseMs;
            setElapsedMs(Math.max(0, elapsed));
        };

        tick();
        const interval = setInterval(tick, 250);
        return () => clearInterval(interval);
    }, [phase]);

    useEffect(() => {
        let unlisten: (() => void) | undefined;
        listen<number>("audio-level", (event) => {
            const raw = phase === "recording" ? event.payload : 0;
            const prev = smoothedRef.current;
            const mid = Math.floor(BAR_COUNT / 2);
            const centred = [...prev];

            for (let i = 0; i < mid; i++) {
                centred[i] = prev[i + 1];
            }
            for (let i = mid + 1; i < BAR_COUNT; i++) {
                centred[i] = prev[i - 1];
            }
            centred[mid] = raw;

            const smoothed = centred.map((value, index) => {
                let oldValue;
                if (index === mid) oldValue = prev[mid];
                else if (index < mid) oldValue = prev[index + 1];
                else oldValue = prev[index - 1];
                const alpha = value > oldValue ? ATTACK : DECAY;
                return oldValue + alpha * (value - oldValue);
            });

            smoothedRef.current = smoothed;
            setLevels(smoothed);
        }).then((fn) => { unlisten = fn; });

        return () => {
            if (unlisten) unlisten();
        };
    }, [phase]);

    const isLive = phase === "recording" || phase === "paused";
    const isDone = phase === "done";
    const isProcessing = phase === "transcribing" || phase === "correcting" || phase === "model_loading";
    const isError =
        phase === "no_model" ||
        phase === "too_short" ||
        phase === "nothing_heard" ||
        phase === "paste_failed" ||
        phase === "cancelled";

    return (
        <div className={`overlay-pill overlay-pill--${phase}`}>
            <div className="overlay-pill__left">
                {isDone ? (
                    <span className="overlay-pill__icon overlay-pill__icon--done">✓</span>
                ) : isProcessing ? (
                    <span className="overlay-pill__spinner" />
                ) : isError ? (
                    <span className="overlay-pill__icon overlay-pill__icon--error">!</span>
                ) : (
                    <span className={`overlay-pill__dot${phase === "paused" ? " overlay-pill__dot--paused" : ""}`} />
                )}
                <span className="overlay-pill__time">
                    {isDone ? formatLatency(latencyMs) : isLive ? formatElapsed(elapsedMs) : getStatusLabel(phase)}
                </span>
            </div>

            <div className={`overlay-pill__wave${isLive ? "" : " overlay-pill__wave--inactive"}`}>
                {levels.map((level, index) => (
                    <span
                        key={index}
                        className="overlay-pill__bar"
                        style={{ height: `${Math.max(3, Math.round(level * 24))}px` }}
                    />
                ))}
            </div>
        </div>
    );
}
