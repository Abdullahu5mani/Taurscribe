import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type TranscriptRecord = {
    id: number;
    created_at: string;
    transcript: string;
    engine: string;
    duration_ms: number | null;
    grammar_llm_used: boolean;
};

interface TranscriptFeedProps {
    refreshKey: number;
    isRecording: boolean;
    isProcessingTranscript: boolean;
    isCorrecting: boolean;
    latestLatency: number | null;
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

export function TranscriptFeed({
    refreshKey,
    isRecording,
    isProcessingTranscript,
    isCorrecting,
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
            const rows = await invoke<TranscriptRecord[]>("list_transcript_history", {
                limit: 50,
                offset: 0,
            });
            // Detect a newly added top item and trigger its enter animation.
            if (rows.length > 0 && rows[0].id !== prevTopIdRef.current) {
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
    const liveLabel = isRecording ? "Listening" : isCorrecting ? "Correcting" : "Processing";
    const liveClass = isRecording ? "feed-live-row--recording" : "feed-live-row--processing";

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
                    Your transcriptions will appear here
                </div>
            )}

            {items.map((item, index) => {
                const isNew = animatingId === item.id;
                const isLatest = index === 0;
                return (
                    // Outer wrapper: animates grid-template-rows 0fr→1fr (height: 0→auto)
                    // so the item EXPANDS smoothly instead of jumping into place.
                    <div
                        key={item.id}
                        className={`feed-item-wrapper${isNew ? " feed-item-wrapper--entering" : ""}`}
                    >
                        <div className={`feed-item${isLatest ? " feed-item--latest" : ""}${isNew ? " feed-item--fading-in" : ""}`}>
                            <div className="feed-item-header">
                                <span className="feed-timestamp">{formatTimestamp(item.created_at)}</span>
                                <div className="feed-badges">
                                    {isLatest && latestLatency !== null && (
                                        <span className="latency-badge">{latestLatency} ms</span>
                                    )}
                                    <span className={`feed-badge feed-badge-engine--${item.engine}`}>
                                        {item.engine === "parakeet" ? "Parakeet" : "Whisper"}
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
                                            "✓"
                                        ) : (
                                            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
                                                <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
                                                <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
                                            </svg>
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
