import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import "./OverlayApp.css";

type Phase = "recording" | "transcribing" | "correcting" | "done" | "too_short" | "paste_failed";
interface Payload { phase: Phase | "hidden"; text?: string; ms?: number; }

const BAR_COUNT = 17;
const ATTACK = 0.35;  // rise speed  (0 = frozen, 1 = instant)
const DECAY  = 0.12;  // fall speed  (lower = slower fade-out)

export function OverlayApp() {
    const [phase, setPhase] = useState<Phase>("recording");
    const [levels, setLevels] = useState<number[]>(() => Array(BAR_COUNT).fill(0));
    const [doneMs, setDoneMs] = useState<number | null>(null);
    const [elapsed, setElapsed] = useState(0);
    const processingStartRef = useRef<number>(0);
    const recordingStartRef = useRef<number>(0);
    const smoothedRef = useRef<number[]>(Array(BAR_COUNT).fill(0));

    useEffect(() => {
        let unlisten: (() => void) | undefined;
        listen<Payload>("overlay-state", (e) => {
            const p = e.payload.phase;
            if (p === "recording") {
                smoothedRef.current = Array(BAR_COUNT).fill(0);
                setLevels(Array(BAR_COUNT).fill(0));
                setDoneMs(null);
                setElapsed(0);
                recordingStartRef.current = Date.now();
            }
            if (p === "transcribing") {
                processingStartRef.current = Date.now();
            }
            if (p === "done") {
                setDoneMs(Date.now() - processingStartRef.current);
            }
            if (p !== "hidden") setPhase(p);
        }).then(fn => { unlisten = fn; });
        return () => { if (unlisten) unlisten(); };
    }, []);

    useEffect(() => {
        if (phase !== "recording") return;
        const interval = setInterval(() => {
            setElapsed(Math.floor((Date.now() - recordingStartRef.current) / 1000));
        }, 1000);
        return () => clearInterval(interval);
    }, [phase]);

    useEffect(() => {
        let unlisten: (() => void) | undefined;
        listen<number>("audio-level", (e) => {
            const raw = e.payload;
            const prev = smoothedRef.current;
            // Put newest value in the centre bar and push older samples
            // outward to the left and right.  This keeps the “current” level
            // visually centred instead of continually drifting right.
            const mid = Math.floor(BAR_COUNT / 2);
            const centred = [...prev];
            // shift left half one step toward the left edge
            for (let i = 0; i < mid; i++) {
                centred[i] = prev[i + 1];
            }
            // shift right half one step toward the right edge
            for (let i = mid + 1; i < BAR_COUNT; i++) {
                centred[i] = prev[i - 1];
            }
            centred[mid] = raw;

            // apply the same asymmetric EMA smoothing used previously
            const smoothed = centred.map((val, i) => {
                // determine the corresponding “old” value in the prior buffer
                let old;
                if (i === mid) {
                    old = prev[mid];
                } else if (i < mid) {
                    old = prev[i + 1];
                } else {
                    old = prev[i - 1];
                }
                const alpha = val > old ? ATTACK : DECAY;
                return old + alpha * (val - old);
            });

            smoothedRef.current = smoothed;
            setLevels(smoothed);
        }).then(fn => { unlisten = fn; });
        return () => { if (unlisten) unlisten(); };
    }, []);

    return (
        <div className={`overlay-box overlay-box--${phase}`}>
            {phase === "recording" && (
                <div className="waveform-container">
                    <div className="waveform-header">
                        <div className="waveform-dot" />
                        <span className="waveform-timer">
                            {Math.floor(elapsed / 60)}:{String(elapsed % 60).padStart(2, "0")}
                        </span>
                    </div>
                    <div className="waveform-bars">
                        {levels.map((lvl, i) => (
                            <div
                                key={i}
                                className="waveform-bar"
                                style={{ height: `${Math.max(3, lvl * 40)}px` }}
                            />
                        ))}
                    </div>
                </div>
            )}
            {(phase === "transcribing" || phase === "correcting") && (
                <div className={`anim-dots ${phase === "correcting" ? "anim-dots--amber" : ""}`}>
                    <div className="dot dot-1" />
                    <div className="dot dot-2" />
                    <div className="dot dot-3" />
                </div>
            )}
            {phase === "done" && (
                <div className="anim-done">
                    {doneMs != null && (
                        <span className="done-latency">
                            {doneMs >= 1000 ? `${(doneMs / 1000).toFixed(1)}s` : `${doneMs}ms`}
                        </span>
                    )}
                    <svg viewBox="0 0 40 40" className="check-svg">
                        <circle cx="20" cy="20" r="16" className="check-circle" />
                        <polyline points="12,20 18,26 28,14" className="check-mark" />
                    </svg>
                </div>
            )}
            {phase === "too_short" && (
                <div className="anim-too-short">
                    <svg viewBox="0 0 40 40" width="36" height="36">
                        <circle className="x-circle" cx="20" cy="20" r="16"
                            fill="none" stroke="#dc2626" strokeWidth="2.5"
                            strokeDasharray="100" strokeDashoffset="100" />
                        <line className="x-line-1" x1="13" y1="13" x2="27" y2="27"
                            stroke="#dc2626" strokeWidth="2.5" strokeLinecap="round"
                            strokeDasharray="20" strokeDashoffset="20" />
                        <line className="x-line-2" x1="27" y1="13" x2="13" y2="27"
                            stroke="#dc2626" strokeWidth="2.5" strokeLinecap="round"
                            strokeDasharray="20" strokeDashoffset="20" />
                    </svg>
                    <span className="too-short-label">Too short</span>
                </div>
            )}
            {phase === "paste_failed" && (
                <div className="anim-paste-failed">
                    <svg viewBox="0 0 40 40" width="36" height="36">
                        <circle className="x-circle" cx="20" cy="20" r="16"
                            fill="none" stroke="#dc2626" strokeWidth="2.5"
                            strokeDasharray="100" strokeDashoffset="100" />
                        <line className="x-line-1" x1="13" y1="13" x2="27" y2="27"
                            stroke="#dc2626" strokeWidth="2.5" strokeLinecap="round"
                            strokeDasharray="20" strokeDashoffset="20" />
                        <line className="x-line-2" x1="27" y1="13" x2="13" y2="27"
                            stroke="#dc2626" strokeWidth="2.5" strokeLinecap="round"
                            strokeDasharray="20" strokeDashoffset="20" />
                    </svg>
                    <span className="paste-failed-label">Couldn't paste</span>
                </div>
            )}
        </div>
    );
}
