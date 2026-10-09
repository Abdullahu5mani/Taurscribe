import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { chronologicalRecordings, moveRecording, recordingWarnings, type RecordingSource } from "../utils/meetingImport";
import "./MeetingRecordingImport.css";

interface Props {
  blocked?: boolean;
  hasDiarizationModel?: boolean;
  onProcessingChange?: (busy: boolean) => void;
  onComplete: (meetingId: number) => void;
}

export function recordingDate(value: string | null): string {
  return value ? new Date(value).toLocaleString() : "Unknown recording time";
}

export function MeetingRecordingImport({ blocked = false, hasDiarizationModel = true, onProcessingChange, onComplete }: Props) {
  const [sources, setSources] = useState<RecordingSource[]>([]);
  const [title, setTitle] = useState("");
  const [inspecting, setInspecting] = useState(false);
  const [importing, setImporting] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [error, setError] = useState("");
  const zone = useRef<HTMLDivElement>(null);
  const mounted = useRef(true);
  const busy = useRef(false);
  const blockedRef = useRef(blocked);
  blockedRef.current = blocked;
  const manualOrder = useRef(false);
  const inspectionQueue = useRef(Promise.resolve());
  const pendingInspections = useRef(0);
  const callbacks = useRef({ onComplete, onProcessingChange });
  callbacks.current = { onComplete, onProcessingChange };

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  const addPaths = useCallback((paths: string[]) => {
    if (!paths.length || busy.current || blockedRef.current || !mounted.current) return;
    pendingInspections.current++;
    setInspecting(true);
    setError("");
    inspectionQueue.current = inspectionQueue.current.then(async () => {
      if (!mounted.current) return;
      try {
        const result = await invoke<{ recordings: RecordingSource[]; errors: string[] }>("inspect_meeting_recordings", { paths });
        if (!mounted.current) return;
        setSources(previous => {
          const seen = new Set(previous.map(s => s.path));
          const added = result.recordings.filter(s => { if (seen.has(s.path)) return false; seen.add(s.path); return true; });
          const combined = [...previous, ...added];
          return manualOrder.current ? combined : chronologicalRecordings(combined);
        });
        if (result.recordings.length) setTitle(previous => previous || result.recordings[0].name.replace(/\.[^.]+$/, ""));
        if (result.errors.length) setError(previous => [previous, ...result.errors].filter(Boolean).join("\n"));
      } catch (reason) {
        if (mounted.current) setError(String(reason));
      } finally {
        pendingInspections.current--;
        if (mounted.current && !pendingInspections.current) setInspecting(false);
      }
    });
  }, []);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    getCurrentWebview().onDragDropEvent(event => {
      if (!active || busy.current || blockedRef.current) return;
      const payload = event.payload;
      if (payload.type === "leave") { setDragging(false); return; }
      if (payload.type !== "drop" && payload.type !== "over" && payload.type !== "enter") return;
      const rect = zone.current?.getBoundingClientRect();
      const scale = window.devicePixelRatio || 1;
      const x = payload.position.x / scale, y = payload.position.y / scale;
      const inside = !!rect && rect.width > 0 && x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
      setDragging(inside && payload.type !== "drop");
      if (inside && payload.type === "drop") addPaths(payload.paths);
    }).then(fn => { if (active) unlisten = fn; else fn(); }).catch(reason => {
      if (active) setError(`File drop is unavailable. Use Choose recordings. ${String(reason)}`);
    });
    return () => { active = false; unlisten?.(); };
  }, [addPaths]);

  const browse = async () => {
    if (busy.current || blockedRef.current) return;
    try {
      const selected = await open({ multiple: true, filters: [{ name: "Meeting recordings", extensions: ["wav", "mp3", "m4a", "aac", "flac", "ogg", "mp4", "mov"] }] });
      if (selected && mounted.current) addPaths(Array.isArray(selected) ? selected : [selected]);
    } catch (reason) { if (mounted.current) setError(String(reason)); }
  };

  const importMeeting = async () => {
    if (busy.current || blockedRef.current || pendingInspections.current || !sources.length || !title.trim()) return;
    busy.current = true;
    setImporting(true); setError("");
    callbacks.current.onProcessingChange?.(true);
    try {
      const id = await invoke<number>("import_meeting_recordings", { paths: sources.map(s => s.path), title: title.trim() });
      if (mounted.current) { setSources([]); setTitle(""); manualOrder.current = false; callbacks.current.onComplete(id); }
    } catch (reason) { if (mounted.current) setError(String(reason)); }
    finally {
      busy.current = false;
      // Parent busy state must last until the backend finishes, even after unmount.
      callbacks.current.onProcessingChange?.(false);
      if (mounted.current) setImporting(false);
    }
  };
  const disabled = blocked || importing;
  const warnings = recordingWarnings(sources);
  const undated = sources.some(s => !s.recorded_start);
  return (
    <div ref={zone} className={`meeting-import ${dragging ? "meeting-import--dragging" : ""}`} data-testid="meeting-import-drop-zone" role="region" aria-label="Import meeting recordings" aria-busy={importing || inspecting}
      onDragOver={event => { event.preventDefault(); if (!disabled) setDragging(true); }}
      onDragLeave={() => setDragging(false)}
      onDrop={event => {
        event.preventDefault(); setDragging(false);
        const paths = Array.from(event.dataTransfer.files).map(file => (file as File & { path?: string }).path).filter((path): path is string => !!path);
        if (paths.length) addPaths(paths);
        else if (!disabled) setError("Use Choose recordings to select local audio files.");
      }}>
      <div className="meeting-import-heading"><div><strong>Import a meeting recording</strong><p>Drop one or more audio files here, or choose recordings.</p></div>
        <button type="button" className="meeting-vault-btn" onClick={browse} disabled={disabled || inspecting} data-testid="meeting-import-browse">Choose recordings</button>
      </div>
      <p className="meeting-import-formats">WAV, MP3, M4A, AAC, FLAC, OGG · MP4/MOV with supported audio</p>
      {inspecting && <p role="status">Reading recording metadata…</p>}
      {sources.length > 0 && <>
        <label className="meeting-import-title">Meeting title<input value={title} onChange={event => setTitle(event.target.value)} disabled={disabled} data-testid="meeting-import-title" /></label>
        <div className="meeting-import-order"><span>Playback order · {sources.length} recording{sources.length === 1 ? "" : "s"}</span>
          <button type="button" onClick={() => { manualOrder.current = false; setSources(chronologicalRecordings); }} disabled={disabled || inspecting} data-testid="meeting-import-sort">Sort by recording time</button>
        </div>
        <ol className="meeting-import-files">{sources.map((source, index) => <li key={source.path}>
          <div><strong>{source.name}</strong><span>{Math.round(source.duration_ms / 1000)} seconds · {recordingDate(source.recorded_start)}{source.recorded_end ? ` → ${recordingDate(source.recorded_end)}` : ""}</span>
            <small>{source.timestamp_source || `No embedded recording timestamp. File modified: ${recordingDate(source.file_modified_at)} (file date only).`}</small>
          </div>
          <div className="meeting-import-file-actions">
            <button type="button" aria-label={`Move ${source.name} earlier`} disabled={disabled || inspecting || index === 0} onClick={() => { manualOrder.current = true; setSources(previous => moveRecording(previous, index, -1)); }}>↑</button>
            <button type="button" aria-label={`Move ${source.name} later`} disabled={disabled || inspecting || index === sources.length - 1} onClick={() => { manualOrder.current = true; setSources(previous => moveRecording(previous, index, 1)); }}>↓</button>
            <button type="button" aria-label={`Remove ${source.name}`} disabled={disabled || inspecting} onClick={() => setSources(previous => previous.filter(s => s.path !== source.path))}>×</button>
          </div>
        </li>)}</ol>
        {undated && <p className="meeting-import-notice">Some files have no recording timestamp. Those files keep their selected positions; use the arrows to set their order. File dates can change when copied.</p>}
        {warnings.map((warning, index) => <p className="meeting-import-warning" role="status" key={index}>{warning}</p>)}
        <p className="meeting-import-notice">Files become one meeting in the order above. Gaps add no silence; overlaps are kept in full. Original files stay unchanged.</p>
        {!hasDiarizationModel && <p className="meeting-import-warning">Nemotron speaker separation is not installed. Import will use basic speaker separation. Install the model in Settings → Models for better results.</p>}
        <button type="button" className="meeting-vault-btn meeting-import-submit" onClick={importMeeting} disabled={disabled || inspecting || !title.trim()} data-testid="meeting-import-submit">{importing ? "Importing and processing…" : warnings.length ? "Import together anyway" : "Import and process meeting"}</button>
      </>}
      {blocked && !importing && <p className="meeting-import-notice">Finish the current audio or model operation before importing.</p>}
      {error && <p className="meeting-import-error" role="alert">{error}</p>}
    </div>
  );
}
