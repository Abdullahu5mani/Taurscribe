import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { IconCheck, IconCopy } from "./Icons";
import { HOTKEY_CHANGED_EVENT, KEY_LABELS, loadHotkeyBinding, type HotkeyBinding } from "../utils/hotkey";

type TranscriptRecord = {
    id: number;
    created_at: string;
    transcript: string;
    engine: string;
    duration_ms: number | null;
    grammar_llm_used: boolean;
    processing_time_ms: number | null;
    audio_source: string | null;
    kind?: "dictation" | "file";
};

/** Bars in the idle waveform; their heights follow a bell curve across the row. */
const READY_WAVE_BARS = 61;
const READY_WAVE = Array.from({ length: READY_WAVE_BARS }, (_, i) => {
    const x = (i - (READY_WAVE_BARS - 1) / 2) / ((READY_WAVE_BARS - 1) / 2); // -1 … 1
    return Math.exp(-2.4 * x * x);
});

/** Shown before the first dictation: a calm idle waveform and how to start. */
function ReadyState() {
    const [binding, setBinding] = useState<HotkeyBinding | null>(null);
    useEffect(() => {
        let alive = true;
        const load = () => { loadHotkeyBinding().then((b) => { if (alive) setBinding(b); }); };
        load();
        window.addEventListener(HOTKEY_CHANGED_EVENT, load);
        return () => { alive = false; window.removeEventListener(HOTKEY_CHANGED_EVENT, load); };
    }, []);

    return (
        <div className="feed-empty" id="feed-empty-state" data-testid="feed-empty-state" role="status">
            <div className="feed-empty-wave" aria-hidden="true">
                {READY_WAVE.map((env, i) => (
                    <span
                        key={i}
                        className="feed-empty-bar"
                        style={{ '--env': env.toFixed(3), '--i': i } as React.CSSProperties}
                    />
                ))}
            </div>
            <p className="feed-empty-title">Ready when you are</p>
            <p className="feed-empty-hint">
                {binding ? (
                    <>
                        {binding.mode === "toggle" ? "Press" : "Hold"}{" "}
                        {binding.keys.map((k, i) => (
                            <span key={k}>
                                {i > 0 && <span className="feed-empty-plus">+</span>}
                                <kbd className="feed-empty-key">{KEY_LABELS[k] ?? k}</kbd>
                            </span>
                        ))}{" "}
                        to dictate, or press REC
                    </>
                ) : (
                    "Press REC to start dictating"
                )}
            </p>
        </div>
    );
}

const TRANSCRIPT_PREVIEW_CHARS = 1_200;

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

const transcriptPreview = (text: string) => {
    if (text.length <= TRANSCRIPT_PREVIEW_CHARS) return text;
    return `${text.slice(0, TRANSCRIPT_PREVIEW_CHARS).trimEnd()}…`;
};

/** One transcript. Every row uses the same size (the newest a little larger),
 *  so resizing the window only re-wraps the text; nothing grows or shrinks. */
function TranscriptText({ text }: { text: string }) {
    const preview = useMemo(() => `"${transcriptPreview(text)}"`, [text]);
    return <pre className="feed-text">{preview}</pre>;
}


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
    const [copyError, setCopyError] = useState<string | null>(null);
    const prevTopIdRef = useRef<number | null>(null);
    const copyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    const animTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    const historyGenerationRef = useRef(0);
    const copyGenerationRef = useRef(0);
    const mountedRef = useRef(true);
    const deletedIdsRef = useRef(new Set<number>());

    const loadHistory = useCallback(async () => {
        const request = ++historyGenerationRef.current;
        try {
            // Only mic recordings here; file transcriptions have their own panel.
            // Filtered in the query so newer file rows can't push dictations out of the 50.
            const response = await invoke<TranscriptRecord[]>("list_transcript_history", {
                limit: 50,
                offset: 0,
                kind: "dictation",
            });
            if (!mountedRef.current || request !== historyGenerationRef.current) return;
            // A read can capture a row just before its successful deletion.
            const rows = response.filter(row => !deletedIdsRef.current.has(row.id));
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
    useEffect(() => {
        mountedRef.current = true;
        void loadHistory();
        return () => {
            mountedRef.current = false;
            ++historyGenerationRef.current;
            ++copyGenerationRef.current;
            if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
            if (animTimerRef.current) clearTimeout(animTimerRef.current);
        };
    }, [loadHistory]);

    // Reload whenever the parent signals a new save.
    useEffect(() => {
        if (refreshKey > 0) void loadHistory();
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [refreshKey]);

    const onCopy = async (e: React.MouseEvent, id: number, text: string) => {
        e.stopPropagation();
        const request = ++copyGenerationRef.current;
        if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
        setCopiedId(null);
        setCopyError(null);
        try {
            await navigator.clipboard.writeText(text);
            if (!mountedRef.current || request !== copyGenerationRef.current) return;
            setCopiedId(id);
            copyTimerRef.current = setTimeout(() => { setCopiedId(null); copyTimerRef.current = null; }, 1500);
        } catch (error) {
            if (mountedRef.current && request === copyGenerationRef.current) setCopyError(`Could not copy transcript: ${String(error)}`);
        }
    };

    const onDelete = async (e: React.MouseEvent, id: number) => {
        e.stopPropagation();
        try {
            await invoke("delete_transcript_history", { id });
            if (!mountedRef.current) return;
            deletedIdsRef.current.add(id);
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
        <div
            className="transcript-feed"
            id="transcript-feed"
            data-testid="transcript-feed"
            role="region"
            aria-label="Transcript feed"
            aria-live="polite"
        >
            {copyError && <p className="file-card-error" role="alert">{copyError}</p>}
            {showLive && (
                // Expands into place (like new transcripts) instead of pushing
                // the list down in one frame.
                <div className="feed-live-wrap">
                <div
                    className={`feed-live-row ${liveClass}`}
                    id="feed-live-status"
                    data-testid="feed-live-status"
                    role="status"
                    aria-live="assertive"
                >
                    <span className="feed-live-dot" />
                    {isRecording && !isPaused && (
                        <div className="feed-live-waveform" aria-hidden="true">
                            {['a', 'b', 'c', 'd', 'e', 'f'].map((c) => (
                                <span key={c} className={`feed-live-bar feed-live-bar--${c}`} />
                            ))}
                        </div>
                    )}
                    <span className="feed-live-label">{liveLabel}…</span>
                </div>
                </div>
            )}

            {items.length === 0 && !showLive && <ReadyState />}

            {items.map((item, index) => {
                const isNew = animatingId === item.id;
                const isLatest = index === 0;
                const displayLatency = item.processing_time_ms ?? (isLatest ? latestLatency : null);
                const distance = Math.min(index, 3);
                return (
                    // Outer wrapper: animates grid-template-rows 0fr→1fr (height: 0→auto)
                    // so the item EXPANDS smoothly instead of jumping into place.
                    <div
                        key={item.id}
                        className={`feed-item-wrapper${isNew ? " feed-item-wrapper--entering" : ""}`}
                    >
                        <div
                            className={`feed-item${isLatest ? " feed-item--latest" : ""}${isNew ? " feed-item--entering feed-item--fading-in" : ""}`}
                            data-distance={distance}
                            id={`transcript-card-${item.id}`}
                            data-testid={`transcript-card-${item.id}`}
                            role="article"
                            aria-label={`Transcript recorded at ${formatTimestamp(item.created_at)}`}
                        >
                            <div className="feed-item-header">
                                <span className="feed-timestamp">{formatTimestamp(item.created_at)}</span>
                                <div className="feed-badges">
                                    {displayLatency !== null ? (
                                        <span className="latency-badge">
                                            {isNew ? <LatencyOdometer target={displayLatency} /> : <>{displayLatency} ms</>}
                                        </span>
                                    ) : null}
                                    <span className={`feed-badge feed-badge-engine--${item.engine}`}>
                                        {item.engine === "granite" ? "Granite" : item.engine === "qwen3" ? "Qwen3" : item.engine === "whisper" ? "Whisper" : item.engine}
                                    </span>
                                    {item.grammar_llm_used && (
                                        <span className="feed-badge feed-badge-llm">LLM</span>
                                    )}
                                    <button
                                        type="button"
                                        id={`transcript-copy-${item.id}`}
                                        data-testid={`transcript-copy-${item.id}`}
                                        className={`feed-icon-btn feed-copy-btn${copiedId === item.id ? " feed-copy-btn--done" : ""}`}
                                        onClick={(e) => onCopy(e, item.id, item.transcript)}
                                        title={copiedId === item.id ? "Copied!" : "Copy to clipboard"}
                                        aria-label={copiedId === item.id ? "Copied transcript" : "Copy transcript to clipboard"}
                                    >
                                        {copiedId === item.id ? (
                                            <IconCheck size={12} />
                                        ) : (
                                            <IconCopy size={12} />
                                        )}
                                    </button>
                                    <button
                                        type="button"
                                        id={`transcript-delete-${item.id}`}
                                        data-testid={`transcript-delete-${item.id}`}
                                        className="feed-icon-btn feed-delete-btn"
                                        onClick={(e) => onDelete(e, item.id)}
                                        title="Delete this record"
                                        aria-label={`Delete transcript from ${formatTimestamp(item.created_at)}`}
                                    >
                                        ×
                                    </button>
                                </div>
                            </div>
                            <TranscriptText text={item.transcript} />
                        </div>
                    </div>
                );
            })}
        </div>
    );
}

export const TranscriptFeed = memo(TranscriptFeedComponent);
