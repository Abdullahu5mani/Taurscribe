import { memo, useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { formatModelDisplay } from "../utils/modelDisplay";

interface FileItem {
    id: string;
    path: string;
    name: string;
    engine: string;
    modelId?: string | null;
    status: "queued" | "processing" | "done" | "error" | "cancelled";
    progress: number;
    transcript: string;
    audioDurationMs?: number;
    processingTimeMs?: number;
    expanded: boolean;
    error?: string;
    historyError?: string;
    historySaving?: boolean;
}

async function saveFileHistory(item: FileItem): Promise<void> {
    await invoke("save_transcript_history", {
        transcript: item.transcript,
        engine: item.engine,
        durationMs: item.audioDurationMs,
        grammarLlmUsed: false,
        processingTimeMs: item.processingTimeMs,
        modelId: item.modelId ?? null,
        audioSource: item.name,
    });
}

interface FileTranscriptionResult {
    transcript: string;
    audio_duration_ms: number;
    processing_time_ms: number;
}

interface ProgressPayload {
    path: string;
    job_id?: string | null;
    percent: number;
    status: string;
    error?: string;
}

interface FileTranscriptionPanelProps {
    activeEngine: string;
    currentModel?: string | null;
    currentGraniteModel?: string | null;
    currentQwen3Model?: string | null;
    isModelLoading?: boolean;
    onFileProcessingChange?: (processing: boolean) => void;
}

function FileTranscriptionPanelComponent({ activeEngine, currentModel, currentGraniteModel, currentQwen3Model, isModelLoading = false, onFileProcessingChange }: FileTranscriptionPanelProps) {
    const isModelLoadingRef = useRef(isModelLoading);
    useEffect(() => { isModelLoadingRef.current = isModelLoading; }, [isModelLoading]);

    // Keep a ref to the active model ID so addPaths (a stable callback) can read it.
    const activeModelIdRef = useRef<string | null>(null);
    const activeEngineRef = useRef(activeEngine);
    const currentActiveModelId =
        activeEngine === "whisper" ? (currentModel ?? null) :
        activeEngine === "granite" ? (currentGraniteModel ?? null) :
        (currentQwen3Model ?? null);
    useEffect(() => { activeModelIdRef.current = currentActiveModelId; }, [currentActiveModelId]);
    useEffect(() => { activeEngineRef.current = activeEngine; }, [activeEngine]);

    const [files, setFiles] = useState<FileItem[]>([]);
    const [isDragOver, setIsDragOver] = useState(false);
    const processingRef = useRef(false);
    const activeFileRef = useRef<{ id: string; path: string } | null>(null);
    const queueRef = useRef<FileItem[]>([]);
    const pendingProgressRef = useRef<Map<string, ProgressPayload>>(new Map());
    const progressRafRef = useRef<number | null>(null);

    const AUDIO_EXTS = ["wav", "mp3", "m4a", "aac", "flac", "ogg", "mp4", "mov"];

    const getExt = (name: string) => name.split(".").pop()?.toLowerCase() ?? "";

    // Notify parent whenever a file transitions to/from active transcription
    const isFileProcessing = files.some(f => f.status === "processing");
    useEffect(() => {
        onFileProcessingChange?.(isFileProcessing);
    }, [isFileProcessing, onFileProcessingChange]);

    const addPaths = useCallback((paths: string[]) => {
        if (isModelLoadingRef.current) return;
        const audio = paths.filter(p => AUDIO_EXTS.includes(getExt(p)));
        if (audio.length === 0) return;
        const model = activeModelIdRef.current;
        const engine = activeEngineRef.current;
        setFiles(prev => {
            const newItems: FileItem[] = audio.map(p => {
                const baseName = p.split(/[\\/]/).pop() ?? p;
                // If this exact path + model combo already exists anywhere in the list,
                // suffix the display name with " (1)" so the user can tell it apart.
                const isDupe = prev.some(f => f.path === p && f.modelId === model && f.engine === engine);
                return {
                    id: crypto.randomUUID(),
                    path: p,
                    name: isDupe ? `${baseName} (1)` : baseName,
                    engine,
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

    const flushProgressUpdates = useCallback(() => {
        progressRafRef.current = null;
        const pending = Array.from(pendingProgressRef.current.entries());
        if (pending.length === 0) return;
        pendingProgressRef.current.clear();

        const updatesById = new Map(pending);

        setFiles(prev => {
            let changed = false;
            const next = prev.map(file => {
                const payload = updatesById.get(file.id);
                if (!payload || file.status !== "processing") return file;

                // Never regress a completed, cancelled, or errored file back to processing
                const nextStatus: FileItem["status"] = payload.status === "done"
                    ? "done"
                    : payload.status === "error"
                      ? "error"
                      : payload.status === "cancelled"
                        ? "cancelled"
                        : "processing";

                const nextProgress = payload.status === "cancelled" ? 0 : Math.max(file.progress, payload.percent);
                const nextError = payload.status === "cancelled"
                    ? payload.error ?? "Cancelled"
                    : payload.error;

                if (
                    file.status === nextStatus &&
                    file.progress === nextProgress &&
                    file.error === nextError
                ) {
                    return file;
                }

                changed = true;
                return {
                    ...file,
                    progress: nextProgress,
                    status: nextStatus,
                    error: nextError,
                };
            });

            return changed ? next : prev;
        });
    }, []);

    // Tauri OS-level file drop
    useEffect(() => {
        let unlisten: (() => void) | undefined;
        getCurrentWebview()
            .onDragDropEvent(event => {
                const blocked = isModelLoadingRef.current;
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
            const active = activeFileRef.current;
            if (!active || active.path !== event.payload.path || active.id !== event.payload.job_id) return;
            pendingProgressRef.current.set(active.id, event.payload);
            if (progressRafRef.current !== null) return;
            progressRafRef.current = requestAnimationFrame(flushProgressUpdates);
        }).then(fn => { unlisten = fn; });
        return () => {
            unlisten?.();
            if (progressRafRef.current !== null) {
                cancelAnimationFrame(progressRafRef.current);
                progressRafRef.current = null;
            }
            pendingProgressRef.current.clear();
        };
    }, [flushProgressUpdates]);

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
        activeFileRef.current = { id: queued.id, path: queued.path };
        setFiles(prev =>
            prev.map(f => f.id === queued.id ? { ...f, status: "processing", progress: 5 } : f)
        );

        try {
            const result = await invoke<FileTranscriptionResult>("transcribe_file", {
                path: queued.path,
                expectedEngine: queued.engine,
                expectedModelId: queued.modelId ?? null,
                jobId: queued.id,
                deferAutoUnload: true,
            });
            pendingProgressRef.current.delete(queued.id);
            const completed = {
                ...queued,
                status: "done" as const,
                progress: 100,
                transcript: result.transcript,
                audioDurationMs: result.audio_duration_ms,
                processingTimeMs: result.processing_time_ms,
                historyError: undefined,
                historySaving: true,
            };
            setFiles(prev =>
                prev.map(f =>
                    f.id === queued.id
                        ? completed
                        : f
                )
            );
            try {
                await saveFileHistory(completed);
                setFiles(prev => prev.map(f => f.id === queued.id
                    ? { ...f, historySaving: false }
                    : f));
            } catch (error) {
                setFiles(prev => prev.map(f => f.id === queued.id
                    ? { ...f, historySaving: false, historyError: String(error) }
                    : f));
            }
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
            activeFileRef.current = null;
            pendingProgressRef.current.delete(queued.id);
            processingRef.current = false;
            setTimeout(() => {
                const next = queueRef.current.find(f => f.status === "queued");
                if (next) processNext();
                else void invoke("finish_file_transcription_batch").catch(console.error);
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
                    ? {
                        ...f,
                        status: "queued",
                        progress: 0,
                        transcript: "",
                        expanded: false,
                        audioDurationMs: undefined,
                        processingTimeMs: undefined,
                        error: undefined,
                        historyError: undefined,
                        historySaving: false,
                        engine: activeEngineRef.current,
                        modelId: activeModelIdRef.current,
                    }
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

    const engineLabel = (engine: string, modelId?: string | null) => {
        const base = engine === "granite" ? "Granite"
            : engine === "granite" ? "Granite"
            : engine === "qwen3" ? "Qwen3-ASR"
            : "Whisper";
        const variant = formatModelDisplay(modelId ?? null);
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
    const isDisabled = isModelLoading;
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
    const queuedWithDifferentEngine = files.some(
        (file) => (file.status === "queued" || file.status === "processing") &&
            (file.engine !== activeEngine || file.modelId !== currentActiveModelId)
    );

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
                id="file-drop-zone"
                data-testid="file-drop-zone"
                role="region"
                aria-label="Audio file drop zone"
                onDragOver={isDisabled ? undefined : onDragOver}
                onDragLeave={isDisabled ? undefined : onDragLeave}
                onDrop={isDisabled ? undefined : onDrop}
            >
                {isModelLoading ? (
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
                        <button
                            type="button"
                            id="file-browse-btn"
                            data-testid="file-browse-btn"
                            className="file-browse-btn"
                            onClick={handleBrowse}
                            aria-label="Browse audio files"
                        >
                            Browse files
                        </button>
                    </>
                ) : (
                    <>
                        <p className="file-drop-hint file-drop-hint--inline">
                            {isDragOver ? "Drop to add more files" : "Drop more files or"}
                        </p>
                        <button
                            type="button"
                            id="file-browse-btn-compact"
                            data-testid="file-browse-btn-compact"
                            className="file-browse-btn file-browse-btn--compact"
                            onClick={handleBrowse}
                            aria-label="Browse more audio files"
                        >
                            Browse
                        </button>
                    </>
                )}
            </div>

            {/* File queue */}
            {files.length > 0 && (
                <div className="file-queue">
                    {queuedWithDifferentEngine && (
                        <div className="file-queue-context-warning">
                            A queued file needs the engine/model it was added with. If you switched models, that file will ask you to re-run it instead of saving a mislabeled transcript.
                        </div>
                    )}
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
                                    <button
                                        type="button"
                                        id="file-queue-cancel-all"
                                        data-testid="file-queue-cancel-all"
                                        className="file-queue-cancel-all"
                                        onClick={cancelAll}
                                        aria-label="Cancel all queued and active file transcriptions"
                                    >
                                        Cancel all
                                    </button>
                                )}
                            </div>
                        );
                    })()}
                    {files.map(item => (
                        <div
                            key={item.id}
                            id={`file-card-${item.id}`}
                            data-testid={`file-card-${item.id}`}
                            role="article"
                            aria-label={`File ${item.name}, status ${item.status}`}
                            className={`file-card file-card--${item.status}`}
                        >
                            <div className="file-card-header">
                                <span className="file-card-name" title={item.path}>{item.name}</span>
                                <div className="file-card-actions">
                                    {item.status === "done" && (
                                        <>
                                            {item.historyError && (
                                                <button
                                                    id={`file-retry-history-save-${item.id}`}
                                                    data-testid={`file-retry-history-save-${item.id}`}
                                                    type="button"
                                                    className="file-card-btn file-card-btn--error"
                                                    onClick={() => {
                                                        setFiles(prev => prev.map(f => f.id === item.id ? { ...f, historySaving: true } : f));
                                                        void saveFileHistory(item).then(() => {
                                                            setFiles(prev => prev.map(f => f.id === item.id ? { ...f, historySaving: false, historyError: undefined } : f));
                                                        }).catch(error => {
                                                            setFiles(prev => prev.map(f => f.id === item.id ? { ...f, historySaving: false, historyError: String(error) } : f));
                                                        });
                                                    }}
                                                    disabled={item.historySaving}
                                                    title="Retry saving transcript to history"
                                                >
                                                    Retry save
                                                </button>
                                            )}
                                            <button
                                                type="button"
                                                id={`file-copy-${item.id}`}
                                                data-testid={`file-copy-${item.id}`}
                                                className="file-card-btn"
                                                onClick={() => copyText(item.transcript)}
                                                title="Copy transcript"
                                                aria-label={`Copy transcript for ${item.name}`}
                                            >
                                                Copy
                                            </button>
                                            <button
                                                type="button"
                                                id={`file-rerun-${item.id}`}
                                                data-testid={`file-rerun-${item.id}`}
                                                className="file-card-btn file-card-btn--secondary"
                                                onClick={() => retranscribe(item)}
                                                disabled={item.historySaving}
                                                title={`Re-transcribe with ${engineLabel(activeEngineRef.current, activeModelIdRef.current)} (switch engine first to use a different model)`}
                                                aria-label={`Re-transcribe ${item.name} with ${engineLabel(activeEngineRef.current, activeModelIdRef.current)}`}
                                            >
                                                Re-run · {engineLabel(activeEngineRef.current, activeModelIdRef.current)}
                                            </button>
                                        </>
                                    )}
                                    {item.status === "error" && (
                                        <button
                                            type="button"
                                            id={`file-retry-${item.id}`}
                                            data-testid={`file-retry-${item.id}`}
                                            className="file-card-btn file-card-btn--error"
                                            onClick={() => retranscribe(item)}
                                            title={`Retry with ${engineLabel(activeEngineRef.current, activeModelIdRef.current)}`}
                                            aria-label={`Retry transcribing ${item.name}`}
                                        >
                                            Retry · {engineLabel(activeEngineRef.current, activeModelIdRef.current)}
                                        </button>
                                    )}
                                    {item.status === "cancelled" && (
                                        <button
                                            type="button"
                                            id={`file-run-${item.id}`}
                                            data-testid={`file-run-${item.id}`}
                                            className="file-card-btn file-card-btn--secondary"
                                            onClick={() => retranscribe(item)}
                                            title={`Transcribe with ${engineLabel(activeEngineRef.current, activeModelIdRef.current)}`}
                                            aria-label={`Transcribe ${item.name}`}
                                        >
                                            Run · {engineLabel(activeEngineRef.current, activeModelIdRef.current)}
                                        </button>
                                    )}
                                    <button
                                        type="button"
                                        id={`file-remove-${item.id}`}
                                        data-testid={`file-remove-${item.id}`}
                                        className={`file-card-btn file-card-btn--remove${item.status === "queued" ? " file-card-btn--remove-queued" : ""}`}
                                        onClick={() => item.status !== "processing" && !item.historySaving && removeFile(item.id)}
                                        disabled={item.status === "processing" || item.historySaving}
                                        title={item.historySaving ? "Wait for history save" : item.status === "processing" ? "Cannot remove — file is being transcribed" : item.status === "queued" ? "Remove from queue" : "Remove"}
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
                                                id={`file-cancel-${item.id}`}
                                                data-testid={`file-cancel-${item.id}`}
                                                className="file-card-btn file-card-btn--error"
                                                onClick={() => cancelTranscription(item.path)}
                                                title="Stop transcription"
                                                aria-label={`Cancel transcription for ${item.name}`}
                                            >
                                                Cancel
                                            </button>
                                        </div>
                                    </div>
                                    <div className="file-card-progress-bar">
                                        <div
                                            className="file-card-progress-fill"
                                            style={{ transform: `scaleX(${Math.min(1, Math.max(0, item.progress / 100))})` }}
                                        />
                                    </div>
                                </div>
                            )}

                            {/* Error */}
                            {item.status === "done" && item.historySaving && (
                                <p id={`file-history-saving-${item.id}`} data-testid={`file-history-saving-${item.id}`} className="file-card-queued" role="status">Saving transcript to history…</p>
                            )}
                            {item.status === "done" && item.historyError && (
                                <p id={`file-history-error-${item.id}`} data-testid={`file-history-error-${item.id}`} className="file-card-error" role="alert">
                                    Transcript ready, but history save failed: {item.historyError}
                                </p>
                            )}
                            {(item.status === "error" || item.status === "cancelled") && item.error && (
                                <p
                                    id={`file-error-${item.id}`}
                                    data-testid={`file-error-${item.id}`}
                                    className="file-card-error"
                                    role="alert"
                                >
                                    {item.error}
                                </p>
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
                                    <span className="file-meta-badge file-meta-badge--engine" title={item.modelId ?? undefined}>
                                        {engineLabel(item.engine, item.modelId)}
                                    </span>
                                    {item.transcript && (
                                        <button
                                            type="button"
                                            id={`file-toggle-transcript-${item.id}`}
                                            data-testid={`file-toggle-transcript-${item.id}`}
                                            className="file-meta-toggle"
                                            onClick={() => toggleExpanded(item.id)}
                                            aria-expanded={item.expanded}
                                            aria-label={item.expanded ? `Hide transcript for ${item.name}` : `Show transcript for ${item.name}`}
                                        >
                                            {item.expanded ? "Hide transcript ▲" : "Show transcript ▼"}
                                        </button>
                                    )}
                                </div>
                            )}

                            {/* Transcript — collapsed by default */}
                            {item.status === "done" && item.transcript && item.expanded && (
                                <div
                                    id={`file-transcript-text-${item.id}`}
                                    data-testid={`file-transcript-text-${item.id}`}
                                    className="file-card-transcript"
                                >
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

export const FileTranscriptionPanel = memo(FileTranscriptionPanelComponent);
