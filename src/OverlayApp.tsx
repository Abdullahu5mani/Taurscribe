import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import "./OverlayApp.css";

type Phase = "recording" | "transcribing" | "correcting" | "done" | "too_short";
interface Payload { phase: Phase | "hidden"; text?: string; }

export function OverlayApp() {
    const [phase, setPhase] = useState<Phase>("recording");

    useEffect(() => {
        let unlisten: (() => void) | undefined;
        listen<Payload>("overlay-state", (e) => {
            const p = e.payload.phase;
            if (p !== "hidden") setPhase(p);
        }).then(fn => { unlisten = fn; });
        return () => { if (unlisten) unlisten(); };
    }, []);

    return (
        <div className={`overlay-box overlay-box--${phase}`}>
            {phase === "recording" && (
                <div className="anim-recording">
                    <div className="ring ring-1" />
                    <div className="ring ring-2" />
                    <div className="ring ring-3" />
                    <div className="core" />
                </div>
            )}
            {phase === "transcribing" && (
                <div className="anim-processing">
                    <div className="arc arc-1" />
                    <div className="arc arc-2" />
                    <div className="arc arc-3" />
                </div>
            )}
            {phase === "correcting" && (
                <div className="anim-processing anim-correcting">
                    <div className="arc arc-1" />
                    <div className="arc arc-2" />
                    <div className="arc arc-3" />
                </div>
            )}
            {phase === "done" && (
                <div className="anim-done">
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
        </div>
    );
}
