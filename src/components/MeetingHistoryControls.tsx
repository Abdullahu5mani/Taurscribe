import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Store } from "@tauri-apps/plugin-store";
interface DuplicateReport { groups: { kind: string; meeting_ids: number[] }[]; checked_meetings: number; unavailable_audio: number }
export function MeetingHistoryControls({ count, onSelect }: { count: number; onSelect: (id: number) => void }) {
    const [limit, setLimit] = useState<number | null>(null);
    const [draft, setDraft] = useState("");
    const [ready, setReady] = useState(false);
    const [error, setError] = useState("");
    const [busy, setBusy] = useState(false);
    const [report, setReport] = useState<DuplicateReport | null>(null);
    const mutation = useRef(false);
    useEffect(() => {
        let active = true;
        void Store.load("settings.json").then(store => store.get<number>("meeting_history_limit")).then(value => {
            if (!active) return;
            const valid = value != null && Number.isSafeInteger(value) && value > 0 ? value : null;
            setLimit(valid); setDraft(valid == null ? "" : String(valid)); setReady(true);
        }).catch(err => { if (active) { setError(String(err)); setReady(true); } });
        return () => { active = false; };
    }, []);
    const save = async () => {
        if (mutation.current) return;
        const value = draft.trim() ? Number(draft) : null;
        if (value != null && (!Number.isSafeInteger(value) || value < 1)) { setError("Enter a positive whole number, or leave blank for unlimited."); return; }
        mutation.current = true; setBusy(true); setError("");
        try {
            const store = await Store.load("settings.json"); await store.set("meeting_history_limit", value); await store.save(); setLimit(value);
        } catch (err) { setError(String(err)); }
        finally { mutation.current = false; setBusy(false); }
    };
    const check = async () => {
        if (mutation.current) return;
        mutation.current = true; setBusy(true); setError(""); setReport(null);
        try { setReport(await invoke<DuplicateReport>("check_duplicate_meetings")); }
        catch (err) { setError(String(err)); }
        finally { mutation.current = false; setBusy(false); }
    };
    return <div className="meeting-history-controls">
        <span className="meeting-history-count">{count} meetings</span>
        <details className="meeting-history-settings">
        <summary>Manage history</summary>
        <div className="meeting-history-settings-body">
        <label>Meeting limit <input type="number" min="1" step="1" placeholder="Unlimited" aria-label="Meeting history advisory limit"
            value={draft} disabled={!ready || busy} onChange={event => setDraft(event.target.value)} /></label>
        <button type="button" className="turn-clip-btn" onClick={save} disabled={!ready || busy}>Save limit</button>
        
        <button type="button" className="turn-clip-btn" onClick={check} disabled={busy}>{busy ? "Working…" : "Check duplicates"}</button>
        <small>The limit is a reminder. Every meeting is kept and new meetings can still be saved.</small>

        {error && <p role="alert">{error}</p>}
        {report && <div role="status"><p>{report.groups.length ? `${report.groups.length} duplicate candidates` : "No duplicate candidates found"} · {report.checked_meetings} meetings checked. {report.unavailable_audio > 0 ? `${report.unavailable_audio} recordings unavailable; transcripts were still checked.` : ""}</p>
            {report.groups.map((group, index) => <div key={index}><span>{group.kind === "identical_recording" ? "Identical recording" : "Matching transcript and duration — review to confirm"}: </span>
                {group.meeting_ids.map(id => <button type="button" key={id} className="turn-clip-btn" aria-label={`Open meeting ${id}`} onClick={() => onSelect(id)}>Open meeting #{id}</button>)}
            </div>)}
        </div>}
        </div></details>
        {limit != null && count >= limit && <p role="status">You have reached your {limit}-meeting reminder.</p>}
    </div>;
}
