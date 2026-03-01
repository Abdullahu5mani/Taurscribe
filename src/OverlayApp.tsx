import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import "./OverlayApp.css";

type Phase = "recording" | "transcribing" | "correcting" | "done";
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
        </div>
    );
}
