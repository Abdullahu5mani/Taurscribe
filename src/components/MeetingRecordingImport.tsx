import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { IconFileText, IconX } from "./Icons";
import { chronologicalRecordings, moveRecording, recordingWarnings, nearbyRecordingGroups, recordingGroupKey, planRecordingImports, type RecordingSource } from "../utils/meetingImport";
import { speakerCountHint } from "../utils/meetingTimeline";
import "./MeetingRecordingImport.css";

interface Props {
  compact?: boolean;
  isVisible?: boolean;
  modelId?: string | null;
  useGpu?: boolean;
  dropTargetRef?: React.RefObject<HTMLDivElement | null>;
  blocked?: boolean;
  hasDiarizationModel?: boolean;
  onProcessingChange?: (busy: boolean) => void;
  onComplete: (meetingId: number) => void;
}

function ModalLayer({ children }: { children: React.ReactNode }) {
  return typeof document !== "undefined" && document.body ? createPortal(children, document.body) : children;
}

export function recordingDate(value: string | null): string {
  return value ? new Date(value).toLocaleString() : "Unknown recording time";
}

export function MeetingRecordingImport({ compact = false, isVisible = true, blocked = false, hasDiarizationModel = true, onProcessingChange, onComplete, dropTargetRef, modelId, useGpu }: Props) {
  const [modalOpen, setModalOpen] = useState(false);
  const [sources, setSources] = useState<RecordingSource[]>([]);
  const [expectedSpeakers, setExpectedSpeakers] = useState("");
  const [title, setTitle] = useState("");
  const [combineAll, setCombineAll] = useState(false);
  const [approvedGroups, setApprovedGroups] = useState<Set<string>>(new Set());
  const [batchProgress, setBatchProgress] = useState({ completed: 0, total: 0 });
  const [inspecting, setInspecting] = useState(false);
  const [importing, setImporting] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [error, setError] = useState("");
  const zone = useRef<HTMLDivElement>(null);
  const modal = useRef<HTMLDivElement>(null);
  const modalOpenRef = useRef(false);
  const dropTarget = useRef(dropTargetRef);
  dropTarget.current = dropTargetRef;
  const mounted = useRef(true);
  const busy = useRef(false);
  const blockedRef = useRef(blocked);
  blockedRef.current = blocked || !isVisible;
  const manualOrder = useRef(false);
  const inspectionQueue = useRef(Promise.resolve());
  const pendingInspections = useRef(0);
  const callbacks = useRef({ onComplete, onProcessingChange });
  callbacks.current = { onComplete, onProcessingChange };

  const openModal = useCallback(() => {
    if (busy.current || blockedRef.current) return;
    modalOpenRef.current = true;
    setModalOpen(true);
  }, []);

  const closeModal = useCallback(() => {
    if (busy.current) return;
    modalOpenRef.current = false;
    setModalOpen(false);
    setDragging(false);
  }, []);

  useEffect(() => {
    if (!modalOpen || typeof document === "undefined") return;
    const previous = document.activeElement as HTMLElement | null;
    modal.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.preventDefault(); closeModal(); }
      if (event.key !== "Tab") return;
      const controls = Array.from(modal.current?.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), [tabindex="0"]') ?? []);
      const first = controls[0], last = controls[controls.length - 1];
      if (!first) { event.preventDefault(); return; }
      if (event.shiftKey && (document.activeElement === first || document.activeElement === modal.current)) {
        event.preventDefault(); last.focus();
      } else if (!event.shiftKey && (document.activeElement === last || document.activeElement === modal.current)) {
        event.preventDefault(); first.focus();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => { document.removeEventListener("keydown", onKeyDown); if (previous?.isConnected) previous.focus(); };
  }, [modalOpen, closeModal]);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  const addPaths = useCallback((paths: string[]) => {
    if (!paths.length || busy.current || blockedRef.current || !mounted.current) return;
    modalOpenRef.current = true;
    setModalOpen(true);
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
      const target = modalOpenRef.current ? modal.current : dropTarget.current?.current ?? zone.current;
      const rect = target?.getBoundingClientRect();
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
    if (busy.current || blockedRef.current || pendingInspections.current || !sources.length || ((combineAll || sources.length === 1) && !title.trim())) return;
    let count: number | null;
    try { count = speakerCountHint(expectedSpeakers); }
    catch (reason) { setError(String(reason)); return; }
    busy.current = true;
    // Close directly: the busy guard intentionally prevents reopening during the job.
    modalOpenRef.current = false; setModalOpen(false);
    setImporting(true); setError("");
    callbacks.current.onProcessingChange?.(true);
    const jobs = planRecordingImports(sources, combineAll, approvedGroups);
    setBatchProgress({ completed: 0, total: jobs.length });
    const failed: RecordingSource[] = [], errors: string[] = [];
    let lastId: number | null = null;
    try {
      for (let index = 0; index < jobs.length; index++) {
        const job = jobs[index];
        const jobTitle = combineAll || sources.length === 1 ? title.trim() : job[0].name.replace(/\.[^.]+$/, "");
        try {
          lastId = await invoke<number>("import_meeting_recordings", { paths: job.map(s => s.path), title: jobTitle, expectedSpeakers: count, modelId, useGpu });
        } catch (reason) { failed.push(...job); errors.push(`${job[0].name}: ${String(reason)}`); }
        if (mounted.current) setBatchProgress({ completed: index + 1, total: jobs.length });
      }
      if (mounted.current) {
        setSources(failed);
        if (failed.length && !combineAll) setTitle(failed[0].name.replace(/\.[^.]+$/, ""));
        if (!failed.length) { setTitle(""); setExpectedSpeakers(""); manualOrder.current = false; setApprovedGroups(new Set()); }
        if (errors.length) setError(`${jobs.length - errors.length} of ${jobs.length} meetings imported. Failed files are ready to retry.\n${errors.join("\n")}`);
        if (lastId !== null) callbacks.current.onComplete(lastId);
      }
    }
    finally {
      busy.current = false;
      // Parent busy state must last until the backend finishes, even after unmount.
      callbacks.current.onProcessingChange?.(false);
      if (mounted.current) setImporting(false);
    }
  };
  const disabled = blocked || importing;
  const warnings = combineAll ? recordingWarnings(sources) : [];
  const nearbyGroups = nearbyRecordingGroups(sources);
  const plannedCount = planRecordingImports(sources, combineAll, approvedGroups).length;
  const undated = sources.some(s => !s.recorded_start);
  const dropHandlers = {
    onDragOver: (event: React.DragEvent) => { event.preventDefault(); if (!disabled) setDragging(true); },
    onDragLeave: () => setDragging(false),
    onDrop: (event: React.DragEvent) => {
      event.preventDefault(); setDragging(false);
      if (disabled) return;
      const paths = Array.from(event.dataTransfer.files).map(file => (file as File & { path?: string }).path).filter((path): path is string => !!path);
      if (paths.length) addPaths(paths);
      else { openModal(); setError("Use Choose recordings to select local audio files."); }
    },
  };
  return <>
    <div ref={zone} className={`meeting-import ${compact ? "meeting-import--compact" : ""} ${importing ? "meeting-import--busy" : ""} ${dragging && !modalOpen ? "meeting-import--dragging" : ""}`} data-testid="meeting-import-drop-zone" role="region" aria-label="Import meeting recordings" {...dropHandlers}>
      <div className="meeting-import-launcher-icon"><IconFileText size={20} /></div>
      <div className="meeting-import-launcher-copy"><strong>{importing ? `Importing meetings · ${batchProgress.completed}/${batchProgress.total}` : sources.length ? `${sources.length} recording${sources.length === 1 ? "" : "s"} ready to import` : "Add a meeting recording"}</strong>
        {importing && <p>You can keep using the app.</p>}
      </div>
      <button type="button" className="meeting-vault-btn" onClick={openModal} disabled={disabled} data-testid="meeting-import-open">{importing ? "Importing…" : sources.length ? "Review import" : compact ? "Import" : "Import recordings"}</button>
    </div>
    {error && !modalOpen && <p className="meeting-import-error" role="alert">{error} — choose Review import to retry.</p>}
    <ModalLayer>{modalOpen && <div className="meeting-import-overlay" data-testid="meeting-import-overlay" onClick={event => { if (event.target === event.currentTarget) closeModal(); }}>
      <div ref={modal} className={`meeting-import-modal ${dragging ? "meeting-import--dragging" : ""}`} role="dialog" aria-modal="true" aria-labelledby="meeting-import-dialog-title" aria-describedby="meeting-import-dialog-description" tabIndex={-1} data-testid="meeting-import-modal" aria-busy={importing || inspecting} {...dropHandlers}>
        <header className="meeting-import-modal-header">
          <div className="meeting-import-modal-heading"><span className="meeting-import-launcher-icon"><IconFileText size={22} /></span><div>
            
            <h2 id="meeting-import-dialog-title">Import recordings</h2>
          </div></div>
          <button type="button" className="meeting-import-close" onClick={closeModal} disabled={importing} aria-label="Close recording importer" data-testid="meeting-import-close"><IconX size={18} /></button>
        </header>
        <div className="meeting-import-modal-body">
          <p id="meeting-import-dialog-description" className="meeting-import-description">Each file becomes a separate meeting unless you choose to combine it.</p>
          <div className={`meeting-import-add-files ${sources.length ? "meeting-import-add-files--selected" : ""}`}>
            <div><strong>{dragging ? "Drop to add recordings" : sources.length ? "Add more recordings" : "Drop your audio files here"}</strong>
              <p className="meeting-import-formats">WAV, MP3, M4A, AAC, FLAC, OGG · MP4/MOV audio</p>
            </div>
            <button type="button" className="meeting-vault-btn" onClick={browse} disabled={disabled || inspecting} data-testid="meeting-import-browse">{sources.length ? "Add files" : "Choose recordings"}</button>
          </div>
          {inspecting && <p className="meeting-import-reading" role="status">Reading recording metadata…</p>}
          {sources.length > 0 && <>
            <label className="meeting-import-mode"><input type="checkbox" checked={combineAll} disabled={disabled} onChange={event => setCombineAll(event.target.checked)} data-testid="meeting-import-combine" />Combine all files into one meeting</label>
            {!combineAll && nearbyGroups.map(group => {
              const key = recordingGroupKey(group);
              return <label className="meeting-import-merge-suggestion" key={key}><input type="checkbox" checked={approvedGroups.has(key)} disabled={disabled} onChange={event => setApprovedGroups(previous => { const next = new Set(previous); if (event.target.checked) next.add(key); else next.delete(key); return next; })} />
                <span>These {group.length} recordings are within 5 minutes of each other. Combine? <strong>{group[0].name} → {group[group.length - 1].name}</strong></span></label>;
            })}
            {(combineAll || sources.length === 1) && <label className="meeting-import-title">Meeting title<input value={title} onChange={event => setTitle(event.target.value)} disabled={disabled} data-testid="meeting-import-title" /></label>}
            {warnings.map((warning, index) => <p className="meeting-import-warning" role="status" key={index}>{warning}</p>)}
            <div className="meeting-import-order"><span>Selected files · {sources.length} recording{sources.length === 1 ? "" : "s"}</span>
              <button type="button" onClick={() => { manualOrder.current = false; setSources(chronologicalRecordings); }} disabled={disabled || inspecting} data-testid="meeting-import-sort">Sort by recording time</button>
            </div>
            <ol className="meeting-import-files">{sources.map((source, index) => <li key={source.path}>
              <div className="meeting-import-file-info"><strong>{source.name}</strong><span title={source.timestamp_source || "No recording timestamp"}>{Math.round(source.duration_ms / 1000)} seconds{source.recorded_start ? ` · ${recordingDate(source.recorded_start)}` : ""}</span>
              </div>
              <div className="meeting-import-file-actions">
                <button type="button" aria-label={`Move ${source.name} earlier`} disabled={disabled || inspecting || index === 0} onClick={() => { manualOrder.current = true; setSources(previous => moveRecording(previous, index, -1)); }}>↑</button>
                <button type="button" aria-label={`Move ${source.name} later`} disabled={disabled || inspecting || index === sources.length - 1} onClick={() => { manualOrder.current = true; setSources(previous => moveRecording(previous, index, 1)); }}>↓</button>
                <button type="button" aria-label={`Remove ${source.name}`} disabled={disabled || inspecting} onClick={() => setSources(previous => previous.filter(s => s.path !== source.path))}>×</button>
              </div>
            </li>)}</ol>
            {undated && sources.length > 1 && <p className="meeting-import-notice">Undated files keep their selected order. Use the arrows to rearrange.</p>}
            <details className="meeting-import-options"><summary>Speaker options</summary>
            <label className="meeting-import-title">About how many people? (optional)<input type="number" min="1" max="32" step="1" aria-label="About how many people?" value={expectedSpeakers} disabled={disabled} onChange={e => setExpectedSpeakers(e.target.value)} /></label>
            {!hasDiarizationModel && <p className="meeting-import-notice">Install Meeting speakers in Settings → Models for better speaker recognition.</p>}
            </details>
          </>}
          {blocked && !importing && <p className="meeting-import-notice">Finish the current audio or model operation before importing.</p>}
          {error && <p className="meeting-import-error" role="alert">{error}</p>}
        </div>
        <footer className="meeting-import-modal-footer">
          <button type="button" className="meeting-vault-btn" onClick={closeModal} disabled={importing}>Close</button>
          <button type="button" className="meeting-import-submit" onClick={importMeeting} disabled={disabled || inspecting || !sources.length || ((combineAll || sources.length === 1) && !title.trim())} data-testid="meeting-import-submit">{importing ? "Importing and processing…" : warnings.length ? "Import together anyway" : `Import ${plannedCount} meeting${plannedCount === 1 ? "" : "s"}`}</button>
        </footer>
      </div>
    </div>}</ModalLayer>
  </>;
}
