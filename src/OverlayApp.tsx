import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Store } from "@tauri-apps/plugin-store";
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
    | "cancelled";

interface Payload {
    phase: Phase | "hidden";
    text?: string;
    ms?: number;
    engine?: string | null;
}

const BAR_COUNT = 17;
const ATTACK = 0.35;
const DECAY = 0.12;

function formatEngine(engine: string) {
    if (engine === "parakeet") return "Parakeet";
    if (engine === "granite_speech") return "Granite";
    return "Whisper";
}

function formatElapsed(ms: number) {
    const totalSeconds = Math.max(0, Math.floor(ms / 1000));
    const minutes = Math.floor(totalSeconds / 60);
    const seconds = totalSeconds % 60;
    return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

function formatLatency(ms: number | null) {
    if (ms == null) return null;
    return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`;
}

export function OverlayApp() {
    const [phase, setPhase] = useState<Phase>("recording");
    const [engine, setEngine] = useState("whisper");
    const [transcript, setTranscript] = useState("");
    const [latencyMs, setLatencyMs] = useState<number | null>(null);
    const [elapsedMs, setElapsedMs] = useState(0);
    const [levels, setLevels] = useState<number[]>(() => Array(BAR_COUNT).fill(0));
    const [overlayStyle, setOverlayStyle] = useState<'minimal' | 'full'>('full');

    const smoothedRef = useRef<number[]>(Array(BAR_COUNT).fill(0));
    const sessionStartedAtRef = useRef(0);
    const pauseStartedAtRef = useRef<number | null>(null);
    const pausedTotalMsRef = useRef(0);
    const previousPhaseRef = useRef<Phase | "hidden">("hidden");

    // Load overlay style from store on mount, then listen for live changes
    useEffect(() => {
        Store.load('settings.json').then((store) => {
            store.get<'minimal' | 'full'>('overlay_style').then((val) => {
                if (val) setOverlayStyle(val);
            });
        });

        let unlisten: (() => void) | undefined;
        listen<'minimal' | 'full'>('overlay-style-changed', (event) => {
            setOverlayStyle(event.payload);
        }).then((fn) => { unlisten = fn; });

        return () => { if (unlisten) unlisten(); };
    }, []);

    // Resize the Tauri window to match the overlay style so the transparent
    // window box never extends beyond the visible content.
    useEffect(() => {
        const win = getCurrentWindow();
        if (overlayStyle === 'minimal') {
            // Tight fit around the pill content — content is ~190px at 0.72rem Martian Mono
            // with padding 11+14px + dot 7px + gaps. 230px gives comfortable clearance.
            win.setSize(new LogicalSize(230, 34)).catch(() => {});
        } else {
            win.setSize(new LogicalSize(380, 170)).catch(() => {});
        }
    }, [overlayStyle]);

    useEffect(() => {
        let unlisten: (() => void) | undefined;
        listen<Payload>("overlay-state", (event) => {
            const payload = event.payload;
            if (payload.phase === "hidden") {
                previousPhaseRef.current = "hidden";
                return;
            }

            if (payload.engine) {
                setEngine(payload.engine);
            }
            if (typeof payload.text === "string") {
                setTranscript(payload.text);
            }
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
                    setTranscript(payload.text ?? "");
                    setLatencyMs(null);
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
            const raw = phase === "paused" ? 0 : event.payload;
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

    const statusLabel = phase === "recording"
        ? "Listening"
        : phase === "paused"
            ? "Paused"
            : phase === "transcribing"
                ? "Transcribing"
                : phase === "correcting"
                    ? "Correcting"
                    : phase === "done"
                        ? "Inserted"
                        : phase === "too_short"
                            ? "Too Short"
                            : phase === "paste_failed"
                                ? "Paste Failed"
                                : "Discarded";

    const transcriptBody = transcript.trim()
        || (phase === "recording"
            ? "Listening for your first chunk..."
            : phase === "paused"
                ? "Recording paused. Resume when you're ready."
                : phase === "done"
                    ? "Transcript sent back to the main app."
                    : phase === "cancelled"
                        ? "Recording discarded."
                        : "Working on your transcript...");

    const engineClass = `overlay-engine--${engine}`;

    if (overlayStyle === 'minimal') {
        return (
            <div className={`overlay-pill overlay-pill--${phase} ${engineClass}`}>
                <span className={`overlay-pill-dot overlay-pill-dot--${phase}`} />
                <span className="overlay-pill-engine">{formatEngine(engine)}</span>
                <span className="overlay-pill-sep">·</span>
                <span className="overlay-pill-status">{statusLabel}</span>
                {(phase === "recording" || phase === "paused") && (
                    <span className="overlay-pill-time">{formatElapsed(elapsedMs)}</span>
                )}
                {phase === "done" && formatLatency(latencyMs) && (
                    <span className="overlay-pill-time">{formatLatency(latencyMs)}</span>
                )}
            </div>
        );
    }

    return (
        <div className={`overlay-shell overlay-shell--${phase} ${engineClass}`}>
            <div className="overlay-topline">
                <div className="overlay-phase-group">
                    <span className={`overlay-phase-dot overlay-phase-dot--${phase}`} />
                    <div className="overlay-phase-copy">
                        <span className="overlay-engine">{formatEngine(engine)}</span>
                        <span className="overlay-status">{statusLabel}</span>
                    </div>
                </div>
                <div className="overlay-meta">
                    {(phase === "recording" || phase === "paused") && (
                        <span className="overlay-time">{formatElapsed(elapsedMs)}</span>
                    )}
                    {phase === "done" && formatLatency(latencyMs) && (
                        <span className="overlay-time">{formatLatency(latencyMs)}</span>
                    )}
                </div>
            </div>

            <div className="overlay-transcript">
                {transcriptBody}
            </div>

            <div className="overlay-bottom">
                <div className={`overlay-waveform${phase === "paused" ? " overlay-waveform--paused" : ""}`}>
                    {levels.map((level, index) => (
                        <span
                            key={index}
                            className="overlay-waveform-bar"
                            style={{ height: `${Math.max(4, level * 28)}px` }}
                        />
                    ))}
                </div>
            </div>
        </div>
    );
}
