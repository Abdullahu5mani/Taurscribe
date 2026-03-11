import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { IconCheck } from "./Icons";

type TranscriptRecord = {
    id: number;
    created_at: string;
    transcript: string;
    engine: string;
    duration_ms: number | null;
    grammar_llm_used: boolean;
};

interface TranscriptHistoryProps {
    /** Bump this counter from the parent after each successful save to auto-refresh the list. */
    refreshKey?: number;
}

const formatTimestamp = (iso: string) => {
    try {
        const d = new Date(iso);
        if (Number.isNaN(d.getTime())) return iso;
        return d.toLocaleString();
    } catch {
        return iso;
    }
};

const truncate = (text: string, max = 140) =>
    text.length > max ? text.slice(0, max).trimEnd() + "…" : text;

export function TranscriptHistory({ refreshKey }: TranscriptHistoryProps) {
    const [open, setOpen] = useState(false);
    const [items, setItems] = useState<TranscriptRecord[]>([]);
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [copiedId, setCopiedId] = useState<number | null>(null);
    const copyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    const loadHistory = useCallback(async () => {
        setLoading(true);
        setError(null);
        try {
            const rows = await invoke<TranscriptRecord[]>("list_transcript_history", {
                limit: 50,
                offset: 0,
            });
            setItems(rows);
        } catch (e) {
            console.error("Failed to load transcript history:", e);
            setError("Failed to load history");
        } finally {
            setLoading(false);
        }
    }, []);

    // Reload whenever the panel is opened.
    useEffect(() => {
        if (open) void loadHistory();
    }, [open, loadHistory]);

    // Reload when parent signals a new save (refreshKey bump) — only if already open.
    useEffect(() => {
        if (open && refreshKey !== undefined && refreshKey > 0) {
            void loadHistory();
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [refreshKey]);

    const onCopy = async (id: number, text: string) => {
        try {
            // Prefer Tauri's arboard-backed clipboard (works in WebView without extra permissions).
            await invoke("type_text", { text }).catch(async () => {
                // Fallback: browser clipboard API.
                await navigator.clipboard.writeText(text);
            });
        } catch {
            // Best-effort; non-critical.
        }
        // Show a brief "Typed" indicator on the row.
        if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
        setCopiedId(id);
        copyTimerRef.current = setTimeout(() => { setCopiedId(null); copyTimerRef.current = null; }, 1400);
    };

    const onDelete = async (e: React.MouseEvent, id: number) => {
        e.stopPropagation(); // don't trigger the row's type action
        try {
            await invoke("delete_transcript_history", { id });
            setItems(prev => prev.filter(item => item.id !== id));
        } catch (err) {
            console.warn("Failed to delete history row:", err);
        }
    };

    return (
        <section className="history-panel">
            <button
                type="button"
                className="history-header"
                onClick={() => setOpen(o => !o)}
            >
                <span>History</span>
                <span className="history-header-meta">
                    {loading ? "Loading…" : open ? "Hide" : "Show"}
                </span>
            </button>
            {open && (
                <div className="history-body">
                    {error && <div className="history-error">{error}</div>}
                    {!error && items.length === 0 && !loading && (
                        <div className="history-empty">No transcriptions yet.</div>
                    )}
                    {!error && items.length > 0 && (
                        <ul className="history-list">
                            {items.map(item => (
                                <li
                                    key={item.id}
                                    className={`history-item${copiedId === item.id ? " history-item--copied" : ""}`}
                                    onClick={() => onCopy(item.id, item.transcript)}
                                    title={copiedId === item.id ? "Typed!" : "Click to type transcript"}
                                >
                                    <div className="history-item-header">
                                        <span className="history-timestamp">
                                            {formatTimestamp(item.created_at)}
                                        </span>
                                        <div className="history-badges">
                                            {copiedId === item.id && (
                                                <span className="history-badge history-badge-copied"><IconCheck size={11} /> Typed</span>
                                            )}
                                            <span className={`history-badge history-badge-engine history-badge-engine--${item.engine}`}>
                                                {item.engine === "parakeet" ? "Parakeet" : item.engine === "granite_speech" ? "Granite" : "Whisper"}
                                            </span>
                                            {item.grammar_llm_used && (
                                                <span className="history-badge history-badge-llm">
                                                    LLM
                                                </span>
                                            )}
                                            <button
                                                type="button"
                                                className="history-delete-btn"
                                                onClick={(e) => onDelete(e, item.id)}
                                                title="Delete this record"
                                                aria-label="Delete record"
                                            >
                                                ×
                                            </button>
                                        </div>
                                    </div>
                                    <div className="history-snippet">
                                        {truncate(item.transcript)}
                                    </div>
                                </li>
                            ))}
                        </ul>
                    )}
                </div>
            )}
        </section>
    );
}

