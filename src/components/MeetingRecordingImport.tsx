import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { IconFileText, IconX } from "./Icons";
import { chronologicalRecordings, moveRecording, recordingWarnings, type RecordingSource } from "../utils/meetingImport";
import "./MeetingRecordingImport.css";

interface Props {
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

export function MeetingRecordingImport({ blocked = false, hasDiarizationModel = true, onProcessingChange, onComplete, dropTargetRef }: Props) {
  const [modalOpen, setModalOpen] = useState(false);
  const [sources, setSources] = useState<RecordingSource[]>([]);
  const [title, setTitle] = useState("");
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
  blockedRef.current = blocked;
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
    if (busy.current || blockedRef.current || pendingInspections.current || !sources.length || !title.trim()) return;
    busy.current = true;
    setImporting(true); setError("");
    callbacks.current.onProcessingChange?.(true);
    try {
      const id = await invoke<number>("import_meeting_recordings", { paths: sources.map(s => s.path), title: title.trim() });
      if (mounted.current) {
        modalOpenRef.current = false; setModalOpen(false);
        setSources([]); setTitle(""); manualOrder.current = false; callbacks.current.onComplete(id);
      }
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
    <div ref={zone} className={`meeting-import ${dragging && !modalOpen ? "meeting-import--dragging" : ""}`} data-testid="meeting-import-drop-zone" role="region" aria-label="Import meeting recordings" {...dropHandlers}>
      <div className="meeting-import-launcher-icon"><IconFileText size={20} /></div>
      <div className="meeting-import-launcher-copy"><strong>{sources.length ? `${sources.length} recording${sources.length === 1 ? "" : "s"} ready to import` : "Add a meeting recording"}</strong>
        <p>{dragging ? "Drop to open the recording importer" : "Drop audio files anywhere in Meetings to get started."}</p>
      </div>
      <button type="button" className="meeting-vault-btn" onClick={openModal} disabled={disabled} data-testid="meeting-import-open">{sources.length ? "Review import" : "Import recordings"}</button>
    </div>
    <ModalLayer>{modalOpen && <div className="meeting-import-overlay" data-testid="meeting-import-overlay" onClick={event => { if (event.target === event.currentTarget) closeModal(); }}>
      <div ref={modal} className={`meeting-import-modal ${dragging ? "meeting-import--dragging" : ""}`} role="dialog" aria-modal="true" aria-labelledby="meeting-import-dialog-title" aria-describedby="meeting-import-dialog-description" tabIndex={-1} data-testid="meeting-import-modal" aria-busy={importing || inspecting} {...dropHandlers}>
        <header className="meeting-import-modal-header">
          <div className="meeting-import-modal-heading"><span className="meeting-import-launcher-icon"><IconFileText size={22} /></span><div>
            <span className="meeting-import-eyebrow">MEETING RECORDINGS</span>
            <h2 id="meeting-import-dialog-title">Import a meeting</h2>
          </div></div>
          <button type="button" className="meeting-import-close" onClick={closeModal} disabled={importing} aria-label="Close recording importer" data-testid="meeting-import-close"><IconX size={18} /></button>
        </header>
        <div className="meeting-import-modal-body">
          <p id="meeting-import-dialog-description" className="meeting-import-description">Bring your recordings together, arrange the timeline, and identify the speakers.</p>
          <div className="meeting-import-add-files">
            <div><strong>{dragging ? "Drop to add recordings" : sources.length ? "Add more recordings" : "Drop your audio files here"}</strong>
              <p className="meeting-import-formats">WAV, MP3, M4A, AAC, FLAC, OGG · MP4/MOV audio</p>
            </div>
            <button type="button" className="meeting-vault-btn" onClick={browse} disabled={disabled || inspecting} data-testid="meeting-import-browse">Choose recordings</button>
          </div>
          {inspecting && <p className="meeting-import-reading" role="status">Reading recording metadata…</p>}
          {sources.length > 0 && <>
            <label className="meeting-import-title">Meeting title<input value={title} onChange={event => setTitle(event.target.value)} disabled={disabled} data-testid="meeting-import-title" /></label>
            {warnings.map((warning, index) => <p className="meeting-import-warning" role="status" key={index}>{warning}</p>)}
            <div className="meeting-import-order"><span>Playback order · {sources.length} recording{sources.length === 1 ? "" : "s"}</span>
              <button type="button" onClick={() => { manualOrder.current = false; setSources(chronologicalRecordings); }} disabled={disabled || inspecting} data-testid="meeting-import-sort">Sort by recording time</button>
            </div>
            <ol className="meeting-import-files">{sources.map((source, index) => <li key={source.path}>
              <div className="meeting-import-file-info"><strong>{source.name}</strong><span>{Math.round(source.duration_ms / 1000)} seconds · {recordingDate(source.recorded_start)}{source.recorded_end ? ` → ${recordingDate(source.recorded_end)}` : ""}</span>
                <small>{source.timestamp_source || `No embedded recording timestamp. File modified: ${recordingDate(source.file_modified_at)} (file date only).`}</small>
              </div>
              <div className="meeting-import-file-actions">
                <button type="button" aria-label={`Move ${source.name} earlier`} disabled={disabled || inspecting || index === 0} onClick={() => { manualOrder.current = true; setSources(previous => moveRecording(previous, index, -1)); }}>↑</button>
                <button type="button" aria-label={`Move ${source.name} later`} disabled={disabled || inspecting || index === sources.length - 1} onClick={() => { manualOrder.current = true; setSources(previous => moveRecording(previous, index, 1)); }}>↓</button>
                <button type="button" aria-label={`Remove ${source.name}`} disabled={disabled || inspecting} onClick={() => setSources(previous => previous.filter(s => s.path !== source.path))}>×</button>
              </div>
            </li>)}</ol>
            {undated && <p className="meeting-import-notice">Some files have no recording timestamp. Those files keep their selected positions; use the arrows to set their order. File dates can change when copied.</p>}
            <div className="meeting-import-processing-note"><strong>Transcription & speaker recognition</strong>
              <p>Uses your speech model and saved voices{hasDiarizationModel ? ", with Nemotron speaker separation." : ". Basic speaker separation will be used; install Nemotron in Settings → Models for better results."}</p>
            </div>
            <p className="meeting-import-notice">Files become one meeting in the order above. Gaps add no silence; overlaps are kept in full. Original files stay unchanged.</p>
          </>}
          {blocked && !importing && <p className="meeting-import-notice">Finish the current audio or model operation before importing.</p>}
          {error && <p className="meeting-import-error" role="alert">{error}</p>}
        </div>
        <footer className="meeting-import-modal-footer">
          <button type="button" className="meeting-vault-btn" onClick={closeModal} disabled={importing}>Close</button>
          <button type="button" className="meeting-import-submit" onClick={importMeeting} disabled={disabled || inspecting || !sources.length || !title.trim()} data-testid="meeting-import-submit">{importing ? "Importing and processing…" : warnings.length ? "Import together anyway" : "Import and process meeting"}</button>
        </footer>
      </div>
    </div>}</ModalLayer>
  </>;
}
