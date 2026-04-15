import { memo, useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { IconCheck, IconCopy } from "./Icons";

type TranscriptRecord = {
    id: number;
    created_at: string;
    transcript: string;
    engine: string;
    duration_ms: number | null;
    grammar_llm_used: boolean;
    processing_time_ms: number | null;
    audio_source: string | null;
};

interface TranscriptFeedProps {
    refreshKey: number;
    isRecording: boolean;
    isPaused: boolean;
    isProcessingTranscript: boolean;
    latestLatency: number | null;
}

/** Animated odometer that counts from 0 to `target` over `duration` ms. */
function LatencyOdometer({ target, duration = 400 }: { target: number; duration?: number }) {
    const [display, setDisplay] = useState(0);
    const rafRef = useRef<number | null>(null);

    useEffect(() => {
        const start = performance.now();
        const tick = (now: number) => {
            const elapsed = now - start;
            const progress = Math.min(elapsed / duration, 1);
            // ease-out quad
            const eased = 1 - (1 - progress) * (1 - progress);
            setDisplay(Math.round(eased * target));
            if (progress < 1) {
                rafRef.current = requestAnimationFrame(tick);
            }
        };
        rafRef.current = requestAnimationFrame(tick);
        return () => { if (rafRef.current) cancelAnimationFrame(rafRef.current); };
    }, [target, duration]);

    return <>{display} ms</>;
}


/** Show HH:MM:SS for today, or "Mon 12, 5:42 PM" otherwise. */
const formatTimestamp = (iso: string) => {
    try {
        const d = new Date(iso);
        if (Number.isNaN(d.getTime())) return iso;
        const now = new Date();
        const isToday = d.toDateString() === now.toDateString();
        return isToday
            ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })
            : d.toLocaleString([], { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
    } catch {
        return iso;
    }
};

function TranscriptFeedComponent({
    refreshKey,
    isRecording,
    isPaused,
    isProcessingTranscript,
    latestLatency,
}: TranscriptFeedProps) {
    const [items, setItems] = useState<TranscriptRecord[]>([]);
    const [animatingId, setAnimatingId] = useState<number | null>(null);
    const [copiedId, setCopiedId] = useState<number | null>(null);
    const prevTopIdRef = useRef<number | null>(null);
    const copyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    const animTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    const loadHistory = useCallback(async () => {
        try {
            const all = await invoke<TranscriptRecord[]>("list_transcript_history", {
                limit: 50,
                offset: 0,
            });
            // Only show mic recordings here; file transcriptions have their own panel.
            const rows = all.filter(r => !r.audio_source || r.audio_source === "microphone");
            // Detect a newly added top item and trigger its enter animation.
            const isNewItem = rows.length > 0 && rows[0].id !== prevTopIdRef.current;
            if (isNewItem) {
                const newId = rows[0].id;
                if (animTimerRef.current) clearTimeout(animTimerRef.current);
                setAnimatingId(newId);
                animTimerRef.current = setTimeout(() => {
                    setAnimatingId(null);
                    animTimerRef.current = null;
                }, 650);
                prevTopIdRef.current = newId;
            }
            setItems(rows);
        } catch (e) {
            console.error("[TranscriptFeed] Failed to load history:", e);
        }
    }, []);


    // Load on mount.
    useEffect(() => { void loadHistory(); }, [loadHistory]);

    // Reload whenever the parent signals a new save.
    useEffect(() => {
        if (refreshKey > 0) void loadHistory();
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [refreshKey]);

    const onCopy = (e: React.MouseEvent, id: number, text: string) => {
        e.stopPropagation();
        try { navigator.clipboard.writeText(text).catch(() => { }); } catch { /* non-critical */ }
        if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
        setCopiedId(id);
        copyTimerRef.current = setTimeout(() => { setCopiedId(null); }, 1500);
    };

    const onDelete = async (e: React.MouseEvent, id: number) => {
        e.stopPropagation();
        try {
            await invoke("delete_transcript_history", { id });
            setItems(prev => prev.filter(item => item.id !== id));
        } catch (err) {
            console.warn("[TranscriptFeed] Failed to delete row:", err);
        }
    };

    /* ── Live status row ── */
    const showLive = isRecording || isProcessingTranscript;
    const liveLabel = isRecording ? (isPaused ? "Paused" : "Listening") : "Processing";
    const liveClass = isRecording && !isPaused ? "feed-live-row--recording" : "feed-live-row--processing";

    return (
        <div className="transcript-feed">
            {showLive && (
                <div className={`feed-live-row ${liveClass}`}>
                    <span className="feed-live-dot" />
                    <span className="feed-live-label">{liveLabel}…</span>
                </div>
            )}

            {items.length === 0 && !showLive && (
                <div className="feed-empty">
                    <div className="feed-empty-waveform" aria-hidden="true">
                        {[4, 8, 14, 18, 14, 8, 4].map((h, i) => (
                            <span
                                key={i}
                                className="feed-empty-bar"
                                style={{ '--bar-h': `${h}px`, '--bar-i': i } as React.CSSProperties}
                            />
                        ))}
                    </div>
                    <span className="feed-empty-text">
                        READY<span className="feed-empty-cursor" aria-hidden="true">_</span>
                    </span>
                </div>
            )}

            {items.map((item, index) => {
                const isNew = animatingId === item.id;
                const isLatest = index === 0;
                const displayLatency = item.processing_time_ms ?? (isLatest ? latestLatency : null);
                return (
                    // Outer wrapper: animates grid-template-rows 0fr→1fr (height: 0→auto)
                    // so the item EXPANDS smoothly instead of jumping into place.
                    <div
                        key={item.id}
                        className={`feed-item-wrapper${isNew ? " feed-item-wrapper--entering" : ""}`}
                    >
                        <div className={`feed-item${isLatest ? " feed-item--latest" : ""}${isNew ? " feed-item--entering feed-item--fading-in" : ""}`}>
                            <div className="feed-item-header">
                                <span className="feed-timestamp">{formatTimestamp(item.created_at)}</span>
                                <div className="feed-badges">
                                    {displayLatency !== null ? (
                                        <span className="latency-badge">
                                            {isNew ? <LatencyOdometer target={displayLatency} /> : <>{displayLatency} ms</>}
                                        </span>
                                    ) : null}
                                    <span className={`feed-badge feed-badge-engine--${item.engine}`}>
                                        {item.engine === "parakeet" ? "Parakeet" : item.engine === "cohere" ? "Cohere" : "Whisper"}
                                    </span>
                                    {item.grammar_llm_used && (
                                        <span className="feed-badge feed-badge-llm">LLM</span>
                                    )}
                                    <button
                                        type="button"
                                        className={`feed-icon-btn feed-copy-btn${copiedId === item.id ? " feed-copy-btn--done" : ""}`}
                                        onClick={(e) => onCopy(e, item.id, item.transcript)}
                                        title={copiedId === item.id ? "Copied!" : "Copy to clipboard"}
                                        aria-label="Copy transcript"
                                    >
                                        {copiedId === item.id ? (
                                            <IconCheck size={12} />
                                        ) : (
                                            <IconCopy size={12} />
                                        )}
                                    </button>
                                    <button
                                        type="button"
                                        className="feed-icon-btn feed-delete-btn"
                                        onClick={(e) => onDelete(e, item.id)}
                                        title="Delete this record"
                                        aria-label="Delete record"
                                    >
                                        ×
                                    </button>
                                </div>
                            </div>
                            <pre className="feed-text">{item.transcript}</pre>
                        </div>
                    </div>
                );
            })}
        </div>
    );
}

export const TranscriptFeed = memo(TranscriptFeedComponent);
