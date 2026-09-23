import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import "./OverlayApp.css";

export type Phase =
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

const BAR_COUNT = 21;
const ATTACK = 0.35;
const DECAY = 0.12;
const OVERLAY_WIDTH = 236;
const OVERLAY_HEIGHT = 44;

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
            return "Transcribing…";
        case "correcting":
            return "Polishing…";
        case "done":
            return "Pasted";
        case "too_short":
            return "Too short";
        case "paste_failed":
            return "Couldn't paste";
        case "cancelled":
            return "Discarded";
        case "no_model":
            return "No model loaded";
        case "model_loading":
            return "Loading model…";
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

    return <OverlayPill phase={phase} elapsedMs={elapsedMs} latencyMs={latencyMs} levels={levels} />;
}

const PROCESSING = new Set<Phase>(["transcribing", "correcting", "model_loading"]);
const WARNING = new Set<Phase>(["no_model", "too_short", "nothing_heard", "paste_failed"]);

function PhaseGlyph({ phase }: { phase: Phase }) {
    if (phase === "recording") return <span className="ov-dot" aria-hidden="true" />;
    if (phase === "paused") {
        return (
            <span className="ov-glyph ov-glyph--pause" aria-hidden="true">
                <svg viewBox="0 0 16 16"><rect x="3.5" y="3" width="3" height="10" rx="1" /><rect x="9.5" y="3" width="3" height="10" rx="1" /></svg>
            </span>
        );
    }
    if (PROCESSING.has(phase)) return <span className="ov-spinner" aria-hidden="true" />;
    if (phase === "done") {
        return (
            <span className="ov-glyph ov-glyph--done" aria-hidden="true">
                <svg viewBox="0 0 16 16"><circle cx="8" cy="8" r="7" /><path d="M4.8 8.2l2.1 2.1 4.3-4.5" /></svg>
            </span>
        );
    }
    if (phase === "cancelled") {
        return (
            <span className="ov-glyph ov-glyph--muted" aria-hidden="true">
                <svg viewBox="0 0 16 16"><circle cx="8" cy="8" r="7" /><path d="M5.6 5.6l4.8 4.8M10.4 5.6l-4.8 4.8" /></svg>
            </span>
        );
    }
    return (
        <span className="ov-glyph ov-glyph--warn" aria-hidden="true">
            <svg viewBox="0 0 16 16"><circle cx="8" cy="8" r="7" /><path d="M8 4.6v4.2" /><circle className="ov-glyph-dotfill" cx="8" cy="11.3" r="0.9" /></svg>
        </span>
    );
}

/** Pure view of the overlay capsule; also rendered by the dev preview. */
export function OverlayPill({ phase, elapsedMs, latencyMs, levels }: {
    phase: Phase;
    elapsedMs: number;
    latencyMs: number | null;
    levels: number[];
}) {
    const isLive = phase === "recording" || phase === "paused";
    const isDone = phase === "done";
    const tone = isLive ? "live" : PROCESSING.has(phase) ? "busy" : isDone ? "ok" : WARNING.has(phase) ? "warn" : "muted";
    const status = isDone
        ? `Transcription finished in ${formatLatency(latencyMs)}`
        : isLive ? `Elapsed time ${formatElapsed(elapsedMs)}` : getStatusLabel(phase);

    return (
        <div
            id="overlay-pill"
            data-testid="overlay-pill"
            className={`overlay-pill ov ov--${tone} ov--${phase}`}
            role="status"
            aria-live="polite"
            aria-label={`Recording Overlay: ${getStatusLabel(phase)}`}
        >
            <PhaseGlyph phase={phase} />

            {isLive ? (
                <div className="ov-wave" aria-hidden="true">
                    {levels.map((level, index) => (
                        <span
                            key={index}
                            className="ov-bar"
                            style={{ transform: `scaleY(${Math.max(0.12, Math.min(1, level * 1.15))})` }}
                        />
                    ))}
                </div>
            ) : (
                <span className="ov-label" key={phase}>{getStatusLabel(phase)}</span>
            )}

            <span
                id="overlay-time-label"
                data-testid="overlay-time-label"
                className={`ov-meta${isLive || isDone ? "" : " ov-meta--empty"}`}
                aria-label={`Overlay status: ${status}`}
            >
                {isDone ? formatLatency(latencyMs) : isLive ? formatElapsed(elapsedMs) : ""}
            </span>
        </div>
    );
}
