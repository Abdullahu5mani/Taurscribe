import { useState } from "react";
import { useFileHistory } from "../hooks/useFileHistory";
import { importMethodLabel } from "../utils/fileHistory";
import { formatModelDisplay } from "../utils/modelDisplay";

export function FileHistory({ refreshKey }: { refreshKey: number }) {
    const { records, loading, error, hasMore, refresh, loadMore } = useFileHistory(refreshKey);
    const [copyError, setCopyError] = useState<string | null>(null);
    const [copyNotice, setCopyNotice] = useState<string | null>(null);
    const [expanded, setExpanded] = useState<Set<number>>(() => new Set());

    const copy = async (text: string) => {
        setCopyError(null);
        setCopyNotice(null);
        try {
            await navigator.clipboard.writeText(text);
            setCopyNotice("Transcript copied.");
        }
        catch (cause) { setCopyError(`Could not copy transcript: ${String(cause)}`); }
    };

    return (
        <section className="file-history" aria-labelledby="file-history-title">
            <div className="file-history-heading">
                <h2 id="file-history-title">File history</h2>
                <button className="file-card-btn" type="button" onClick={() => void refresh()} disabled={loading}>Refresh</button>
            </div>
            <p className="file-history-note">Saved transcripts and source details. No audio copies are stored in history.</p>
            {error && <p className="file-card-error" role="alert">Could not load file history: {error}</p>}
            {copyError && <p className="file-card-error" role="alert">{copyError}</p>}
            {copyNotice && <p className="file-card-queued" role="status">{copyNotice}</p>}
            {loading && <p className="file-card-queued" role="status">Loading history…</p>}
            {!loading && !error && records.length === 0 && <p className="file-card-queued">Transcribed files will appear here, including after you restart the app.</p>}
            {records.map(record => (
                <details key={record.id} className="file-history-entry" onToggle={event => {
                    const open = event.currentTarget.open;
                    setExpanded(previous => {
                        const next = new Set(previous);
                        if (open) next.add(record.id); else next.delete(record.id);
                        return next;
                    });
                }}>
                    <summary>
                        <span className="file-history-name">{record.audio_source ?? record.source_path?.split(/[\\/]/).pop() ?? "Audio file"}</span>
                        <time dateTime={record.created_at}>{new Date(record.created_at).toLocaleString()}</time>
                    </summary>
                    {expanded.has(record.id) && <>
                    <dl className="file-history-metadata">
                        <dt>Transcribed</dt><dd>{new Date(record.created_at).toLocaleString()}</dd>
                        <dt>Engine</dt><dd>{record.engine}</dd>
                        <dt>Model</dt><dd>{record.model_id ? <>{formatModelDisplay(record.model_id)} <code>({record.model_id})</code></> : "Unknown (older entry)"}</dd>
                        <dt>Source path</dt><dd>{record.source_path ?? "Not recorded for this older entry"}</dd>
                        <dt>Imported via</dt><dd>{importMethodLabel(record.import_method)}</dd>
                        {record.duration_ms != null && <><dt>Audio duration</dt><dd>{(record.duration_ms / 1000).toFixed(1)} seconds</dd></>}
                        {record.processing_time_ms != null && <><dt>Processing time</dt><dd>{(record.processing_time_ms / 1000).toFixed(1)} seconds</dd></>}
                    </dl>
                    <button className="file-card-btn" type="button" onClick={() => void copy(record.transcript)}>Copy transcript</button>
                    <div className="file-card-transcript file-history-transcript">{record.transcript}</div>
                    </>}
                </details>
            ))}
            {hasMore && <button className="file-card-btn file-history-load-more" type="button" disabled={loading} onClick={() => void loadMore()}>Load older transcripts</button>}
        </section>
    );
}
