import { useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { formatModelDisplay } from "../utils/modelDisplay";

interface FileItem {
    id: string;
    path: string;
    name: string;
    modelId?: string | null;
    status: "queued" | "processing" | "done" | "error" | "cancelled";
    progress: number;
    transcript: string;
    audioDurationMs?: number;
    processingTimeMs?: number;
    expanded: boolean;
    error?: string;
}

interface FileTranscriptionResult {
    transcript: string;
    audio_duration_ms: number;
    processing_time_ms: number;
}

interface ProgressPayload {
    path: string;
    percent: number;
    status: string;
    error?: string;
}

interface FileTranscriptionPanelProps {
    activeEngine: string;
    currentModel?: string | null;
    currentParakeetModel?: string | null;
    currentCohereModel?: string | null;
    isModelLoading?: boolean;
    onFileProcessingChange?: (processing: boolean) => void;
}

export function FileTranscriptionPanel({ activeEngine, currentModel, currentParakeetModel, currentCohereModel, isModelLoading = false, onFileProcessingChange }: FileTranscriptionPanelProps) {
    const isParakeet = activeEngine === "parakeet";
    const isParakeetRef = useRef(isParakeet);
    const isModelLoadingRef = useRef(isModelLoading);
    useEffect(() => { isParakeetRef.current = isParakeet; }, [isParakeet]);
    useEffect(() => { isModelLoadingRef.current = isModelLoading; }, [isModelLoading]);

    // Keep a ref to the active model ID so addPaths (a stable callback) can read it.
    const activeModelIdRef = useRef<string | null>(null);
    const currentActiveModelId =
        activeEngine === "whisper" ? (currentModel ?? null) :
        activeEngine === "parakeet" ? (currentParakeetModel ?? null) :
        (currentCohereModel ?? null);
    useEffect(() => { activeModelIdRef.current = currentActiveModelId; }, [currentActiveModelId]);

    const [files, setFiles] = useState<FileItem[]>([]);
    const [isDragOver, setIsDragOver] = useState(false);
    const processingRef = useRef(false);
    const queueRef = useRef<FileItem[]>([]);

    const AUDIO_EXTS = ["wav", "mp3", "m4a", "aac", "flac", "ogg", "mp4", "mov"];

    const getExt = (name: string) => name.split(".").pop()?.toLowerCase() ?? "";

    // Notify parent whenever a file transitions to/from active transcription
    const isFileProcessing = files.some(f => f.status === "processing");
    useEffect(() => {
        onFileProcessingChange?.(isFileProcessing);
    }, [isFileProcessing, onFileProcessingChange]);

    const addPaths = useCallback((paths: string[]) => {
        if (isParakeetRef.current || isModelLoadingRef.current) return;
        const audio = paths.filter(p => AUDIO_EXTS.includes(getExt(p)));
        if (audio.length === 0) return;
        const model = activeModelIdRef.current;
        setFiles(prev => {
            const newItems: FileItem[] = audio.map(p => {
                const baseName = p.split(/[\\/]/).pop() ?? p;
                // If this exact path + model combo already exists anywhere in the list,
                // suffix the display name with " (1)" so the user can tell it apart.
                const isDupe = prev.some(f => f.path === p && f.modelId === model);
                return {
                    id: `${p}-${Date.now()}`,
                    path: p,
                    name: isDupe ? `${baseName} (1)` : baseName,
                    modelId: model,
                    status: "queued",
                    progress: 0,
                    transcript: "",
                    expanded: false,
                };
            });
            return [...prev, ...newItems];
        });
    }, []);

    // Tauri OS-level file drop
    useEffect(() => {
        let unlisten: (() => void) | undefined;
        getCurrentWebview()
            .onDragDropEvent(event => {
                const blocked = isParakeetRef.current || isModelLoadingRef.current;
                if (event.payload.type === "over") {
                    if (!blocked) setIsDragOver(true);
                } else if (event.payload.type === "drop") {
                    setIsDragOver(false);
                    if (!blocked) addPaths((event.payload as { type: "drop"; paths: string[] }).paths);
                } else {
                    setIsDragOver(false);
                }
            })
            .then(fn => { unlisten = fn; });
        return () => { unlisten?.(); };
    }, [addPaths]);

    // Listen for progress events from Rust
    useEffect(() => {
        let unlisten: (() => void) | undefined;
        listen<ProgressPayload>("file-transcription-progress", event => {
            const { path, percent, status, error } = event.payload;
            setFiles(prev =>
                prev.map(f => {
                    if (f.path !== path) return f;
                    if (status === "cancelled") {
                        return { ...f, progress: 0, status: "cancelled", error: error ?? "Cancelled" };
                    }
                    return {
                        ...f,
                        progress: percent,
                        status:
                            status === "done"
                                ? "done"
                                : status === "error"
                                  ? "error"
                                  : "processing",
                        error,
                    };
                })
            );
        }).then(fn => { unlisten = fn; });
        return () => { unlisten?.(); };
    }, []);

    // Process queue: one file at a time
    useEffect(() => {
        queueRef.current = files;
        processNext();
    }, [files]);

    const processNext = useCallback(async () => {
        if (processingRef.current) return;
        const queued = queueRef.current.find(f => f.status === "queued");
        if (!queued) return;

        processingRef.current = true;
        setFiles(prev =>
            prev.map(f => f.id === queued.id ? { ...f, status: "processing", progress: 5 } : f)
        );

        try {
            const result = await invoke<FileTranscriptionResult>("transcribe_file", { path: queued.path });
            setFiles(prev =>
                prev.map(f =>
                    f.id === queued.id
                        ? {
                              ...f,
                              status: "done",
                              progress: 100,
                              transcript: result.transcript,
                              audioDurationMs: result.audio_duration_ms,
                              processingTimeMs: result.processing_time_ms,
                          }
                        : f
                )
            );
            // Persist to history
            invoke("save_transcript_history", {
                transcript: result.transcript,
                engine: activeEngine,
                durationMs: result.audio_duration_ms,
                grammarLlmUsed: false,
                processingTimeMs: result.processing_time_ms,
                modelId: activeModelId,
                audioSource: queued.name,
            }).catch(() => {});
        } catch (e) {
            const msg = `${e ?? ""}`;
            const cancelled =
                msg.includes("Transcription cancelled") ||
                msg.includes("cancelled") ||
                msg.includes("Cancelled");
            setFiles(prev =>
                prev.map(f =>
                    f.id === queued.id
                        ? {
                              ...f,
                              status: cancelled ? "cancelled" : "error",
                              progress: 0,
                              error: cancelled ? "Cancelled" : msg,
                          }
                        : f
                )
            );
        } finally {
            processingRef.current = false;
            setTimeout(() => {
                const next = queueRef.current.find(f => f.status === "queued");
                if (next) processNext();
            }, 0);
        }
    }, []);

    const cancelAll = () => {
        const processing = files.find(f => f.status === "processing");
        if (processing) cancelTranscription(processing.path);
        setFiles(prev => prev.filter(f =>
            f.status === "done" || f.status === "error" || f.status === "cancelled"
        ));
    };

    const retranscribe = async (item: FileItem) => {
        setFiles(prev =>
            prev.map(f =>
                f.id === item.id
                    ? { ...f, status: "queued", progress: 0, transcript: "", expanded: false, audioDurationMs: undefined, processingTimeMs: undefined, error: undefined }
                    : f
            )
        );
    };

    const toggleExpanded = (id: string) => {
        setFiles(prev => prev.map(f => f.id === id ? { ...f, expanded: !f.expanded } : f));
    };

    const formatDuration = (ms: number) => {
        const s = Math.round(ms / 1000);
        if (s < 60) return `${s}s`;
        const m = Math.floor(s / 60);
        const rem = s % 60;
        return rem > 0 ? `${m}m ${rem}s` : `${m}m`;
    };

    const activeModelId =
        activeEngine === "whisper" ? (currentModel ?? null) :
        activeEngine === "parakeet" ? (currentParakeetModel ?? null) :
        activeEngine === "cohere" ? (currentCohereModel ?? null) : null;

    const engineLabel = () => {
        const base = activeEngine === "parakeet" ? "Parakeet"
            : activeEngine === "cohere" ? "Cohere"
            : "Whisper";
        const variant = formatModelDisplay(activeModelId);
        return variant ? `${base} · ${variant}` : base;
    };

    const formatRealtime = (audioDurationMs: number, processingTimeMs: number) => {
        if (audioDurationMs <= 0 || processingTimeMs <= 0) return null;
        const ratio = audioDurationMs / processingTimeMs;
        return `${ratio.toFixed(1)}x`;
    };

    const removeFile = (id: string) => {
        setFiles(prev => prev.filter(f => f.id !== id));
    };

    const copyText = (text: string) => {
        navigator.clipboard.writeText(text).catch(() => {});
    };

    const cancelTranscription = (filePath: string) => {
        invoke("cancel_file_transcription", { path: filePath }).catch(() => {});
    };

    const handleBrowse = async () => {
        const selected = await open({
            multiple: true,
            filters: [{ name: "Audio", extensions: ["wav", "mp3", "m4a", "aac", "flac", "ogg", "mp4", "mov"] }],
        });
        if (selected) {
            const paths = Array.isArray(selected) ? selected : [selected];
            addPaths(paths);
        }
    };

    // HTML5 drag events (visual feedback for webview drags)
    const isDisabled = isParakeet || isModelLoading;
    const onDragOver = (e: React.DragEvent) => { e.preventDefault(); if (!isDisabled) setIsDragOver(true); };
    const onDragLeave = () => setIsDragOver(false);
    const onDrop = (e: React.DragEvent) => {
        e.preventDefault();
        setIsDragOver(false);
        if (isDisabled) return;
        const paths: string[] = [];
        for (const item of Array.from(e.dataTransfer.items)) {
            const file = item.getAsFile();
            if (file && (file as unknown as { path?: string }).path) {
                paths.push((file as unknown as { path: string }).path);
            }
        }
        if (paths.length) addPaths(paths);
    };

    const isEmpty = files.length === 0;

    // Determine drop zone class
    const dropZoneClass = [
        "file-drop-zone",
        isDisabled
            ? "file-drop-zone--disabled"
            : isDragOver
              ? "file-drop-zone--active"
              : "",
        isEmpty ? "file-drop-zone--empty" : "file-drop-zone--compact",
    ].filter(Boolean).join(" ");

    return (
        <div className="file-panel">
            {/* Drop zone */}
            <div
                className={dropZoneClass}
                onDragOver={isDisabled ? undefined : onDragOver}
                onDragLeave={isDisabled ? undefined : onDragLeave}
                onDrop={isDisabled ? undefined : onDrop}
            >
                {isParakeet ? (
                    <>
                        <div className="file-drop-icon" aria-hidden="true" style={{ opacity: 0.3 }}>
                            <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                                <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                                <polyline points="17 8 12 3 7 8" />
                                <line x1="12" y1="3" x2="12" y2="15" />
                            </svg>
                        </div>
                        <p className="file-drop-title" style={{ opacity: 0.35 }}>File transcription unavailable</p>
                        <p className="file-drop-hint">Parakeet is a streaming engine · switch to Whisper or Cohere for file transcription</p>
                    </>
                ) : isModelLoading ? (
                    <>
                        <div className="file-drop-icon file-drop-icon--loading" aria-hidden="true">
                            <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                                <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                                <polyline points="17 8 12 3 7 8" />
                                <line x1="12" y1="3" x2="12" y2="15" />
                            </svg>
                        </div>
                        <p className="file-drop-title file-drop-title--loading">Loading model…</p>
                        <p className="file-drop-hint">Drop zone will be ready once the model finishes loading</p>
                    </>
                ) : isEmpty ? (
                    <>
                        <div className="file-drop-icon" aria-hidden="true">
                            <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                                <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                                <polyline points="17 8 12 3 7 8" />
                                <line x1="12" y1="3" x2="12" y2="15" />
                            </svg>
                        </div>
                        <p className="file-drop-title">Drop audio files here</p>
                        <p className="file-drop-hint">Drop one or more files · WAV, MP3, M4A, FLAC, OGG</p>
                        <button className="file-browse-btn" onClick={handleBrowse}>Browse files</button>
                    </>
                ) : (
                    <>
                        <p className="file-drop-hint file-drop-hint--inline">
                            {isDragOver ? "Drop to add more files" : "Drop more files or"}
                        </p>
                        <button className="file-browse-btn file-browse-btn--compact" onClick={handleBrowse}>Browse</button>
                    </>
                )}
            </div>

            {/* File queue */}
            {files.length > 0 && (
                <div className="file-queue">
                    {files.length > 1 && (() => {
                        const queuedCount = files.filter(f => f.status === "queued").length;
                        const doneCount = files.filter(f => f.status === "done").length;
                        const hasActive = files.some(f => f.status === "processing" || f.status === "queued");
                        return (
                            <div className="file-queue-header">
                                <span className="file-queue-summary">
                                    {files.length} files · {doneCount} done · {queuedCount} queued
                                </span>
                                {hasActive && (
                                    <button type="button" className="file-queue-cancel-all" onClick={cancelAll}>
                                        Cancel all
                                    </button>
                                )}
                            </div>
                        );
                    })()}
                    {files.map(item => (
                        <div key={item.id} className={`file-card file-card--${item.status}`}>
                            <div className="file-card-header">
                                <span className="file-card-name" title={item.path}>{item.name}</span>
                                <div className="file-card-actions">
                                    {item.status === "done" && (
                                        <>
                                            <button
                                                className="file-card-btn"
                                                onClick={() => copyText(item.transcript)}
                                                title="Copy transcript"
                                            >
                                                Copy
                                            </button>
                                            <button
                                                className="file-card-btn file-card-btn--secondary"
                                                onClick={() => retranscribe(item)}
                                                title={`Re-transcribe with ${engineLabel()} (switch engine first to use a different model)`}
                                            >
                                                Re-run · {engineLabel()}
                                            </button>
                                        </>
                                    )}
                                    {item.status === "error" && (
                                        <button
                                            className="file-card-btn file-card-btn--error"
                                            onClick={() => retranscribe(item)}
                                            title={`Retry with ${engineLabel()}`}
                                        >
                                            Retry · {engineLabel()}
                                        </button>
                                    )}
                                    {item.status === "cancelled" && (
                                        <button
                                            className="file-card-btn file-card-btn--secondary"
                                            onClick={() => retranscribe(item)}
                                            title={`Transcribe with ${engineLabel()}`}
                                        >
                                            Run · {engineLabel()}
                                        </button>
                                    )}
                                    <button
                                        className={`file-card-btn file-card-btn--remove${item.status === "queued" ? " file-card-btn--remove-queued" : ""}`}
                                        onClick={() => item.status !== "processing" && removeFile(item.id)}
                                        disabled={item.status === "processing"}
                                        title={item.status === "processing" ? "Cannot remove — file is being transcribed" : item.status === "queued" ? "Remove from queue" : "Remove"}
                                        aria-label={`Remove ${item.name}`}
                                    >
                                        {item.status === "queued" ? "Remove" : "✕"}
                                    </button>
                                </div>
                            </div>

                            {/* Progress bar */}
                            {(item.status === "processing") && (
                                <div className="file-card-progress-wrap">
                                    <div className="file-card-progress-label">
                                        <span>{item.progress < 25 ? "Decoding…" : "Transcribing…"}</span>
                                        <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
                                            <span>{item.progress}%</span>
                                            <button
                                                type="button"
                                                className="file-card-btn file-card-btn--error"
                                                onClick={() => cancelTranscription(item.path)}
                                                title="Stop transcription"
                                            >
                                                Cancel
                                            </button>
                                        </div>
                                    </div>
                                    <div className="file-card-progress-bar">
                                        <div
                                            className="file-card-progress-fill"
                                            style={{ width: `${item.progress}%` }}
                                        />
                                    </div>
                                </div>
                            )}

                            {/* Error */}
                            {(item.status === "error" || item.status === "cancelled") && item.error && (
                                <p className="file-card-error" role="alert">{item.error}</p>
                            )}

                            {/* Metadata row */}
                            {item.status === "done" && (
                                <div className="file-card-meta">
                                    {item.audioDurationMs != null && (
                                        <span className="file-meta-badge">
                                            {formatDuration(item.audioDurationMs)}
                                        </span>
                                    )}
                                    {item.audioDurationMs != null && item.processingTimeMs != null && formatRealtime(item.audioDurationMs, item.processingTimeMs) && (
                                        <span className="file-meta-badge file-meta-badge--speed" title="Transcription speed vs real-time">
                                            {formatRealtime(item.audioDurationMs, item.processingTimeMs)} speed
                                        </span>
                                    )}
                                    <span className="file-meta-badge file-meta-badge--engine" title={activeModelId ?? undefined}>
                                        {engineLabel()}
                                    </span>
                                    {item.transcript && (
                                        <button
                                            type="button"
                                            className="file-meta-toggle"
                                            onClick={() => toggleExpanded(item.id)}
                                        >
                                            {item.expanded ? "Hide transcript ▲" : "Show transcript ▼"}
                                        </button>
                                    )}
                                </div>
                            )}

                            {/* Transcript — collapsed by default */}
                            {item.status === "done" && item.transcript && item.expanded && (
                                <div className="file-card-transcript">
                                    {item.transcript}
                                </div>
                            )}

                            {item.status === "queued" && (
                                <p className="file-card-queued">Queued…</p>
                            )}
                        </div>
                    ))}
                </div>
            )}
        </div>
    );
}
