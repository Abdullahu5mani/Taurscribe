import { useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";

interface FileItem {
    id: string;
    path: string;
    name: string;
    status: "queued" | "processing" | "done" | "error" | "cancelled";
    progress: number;
    transcript: string;
    error?: string;
}

interface ProgressPayload {
    path: string;
    percent: number;
    status: string;
    error?: string;
}

interface FileTranscriptionPanelProps {
    activeEngine: string;
}

export function FileTranscriptionPanel({ activeEngine }: FileTranscriptionPanelProps) {
    const isParakeet = activeEngine === "parakeet";
    const isParakeetRef = useRef(isParakeet);
    useEffect(() => { isParakeetRef.current = isParakeet; }, [isParakeet]);

    const [files, setFiles] = useState<FileItem[]>([]);
    const [isDragOver, setIsDragOver] = useState(false);
    const processingRef = useRef(false);
    const queueRef = useRef<FileItem[]>([]);

    const AUDIO_EXTS = ["wav", "mp3", "m4a", "aac", "flac", "ogg", "mp4", "mov"];

    const getExt = (name: string) => name.split(".").pop()?.toLowerCase() ?? "";

    const addPaths = useCallback((paths: string[]) => {
        if (isParakeetRef.current) return;
        const audio = paths.filter(p => AUDIO_EXTS.includes(getExt(p)));
        if (audio.length === 0) return;
        setFiles(prev => {
            const existing = new Set(prev.map(f => f.path));
            const newItems: FileItem[] = audio
                .filter(p => !existing.has(p))
                .map(p => ({
                    id: `${p}-${Date.now()}`,
                    path: p,
                    name: p.split(/[\\/]/).pop() ?? p,
                    status: "queued",
                    progress: 0,
                    transcript: "",
                }));
            return [...prev, ...newItems];
        });
    }, []);

    // Tauri OS-level file drop
    useEffect(() => {
        let unlisten: (() => void) | undefined;
        getCurrentWebview()
            .onDragDropEvent(event => {
                if (event.payload.type === "over") {
                    setIsDragOver(true);
                } else if (event.payload.type === "drop") {
                    setIsDragOver(false);
                    addPaths((event.payload as { type: "drop"; paths: string[] }).paths);
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
            const transcript = await invoke<string>("transcribe_file", { path: queued.path });
            setFiles(prev =>
                prev.map(f =>
                    f.id === queued.id
                        ? { ...f, status: "done", progress: 100, transcript }
                        : f
                )
            );
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
            // Trigger next in queue
            setTimeout(() => {
                const next = queueRef.current.find(f => f.status === "queued");
                if (next) processNext();
            }, 0);
        }
    }, []);

    const retranscribe = async (item: FileItem) => {
        setFiles(prev =>
            prev.map(f =>
                f.id === item.id
                    ? { ...f, status: "queued", progress: 0, transcript: "", error: undefined }
                    : f
            )
        );
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
    const onDragOver = (e: React.DragEvent) => { e.preventDefault(); setIsDragOver(true); };
    const onDragLeave = () => setIsDragOver(false);
    const onDrop = (e: React.DragEvent) => {
        e.preventDefault();
        setIsDragOver(false);
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

    return (
        <div className="file-panel">
            {/* Drop zone */}
            <div
                className={`file-drop-zone${!isParakeet && isDragOver ? " file-drop-zone--active" : ""}${isEmpty ? " file-drop-zone--empty" : " file-drop-zone--compact"}${isParakeet ? " file-drop-zone--disabled" : ""}`}
                onDragOver={isParakeet ? undefined : onDragOver}
                onDragLeave={isParakeet ? undefined : onDragLeave}
                onDrop={isParakeet ? undefined : onDrop}
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
                        <p className="file-drop-hint">Parakeet is a streaming engine · switch to Whisper or Granite for file transcription</p>
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
                        <p className="file-drop-hint">WAV · MP3 · M4A · FLAC · OGG</p>
                        <button className="file-browse-btn" onClick={handleBrowse}>Browse files</button>
                    </>
                ) : (
                    <p className="file-drop-hint file-drop-hint--inline">
                        {isDragOver ? "Drop to add more files" : "Drop more files to queue them"}
                    </p>
                )}
            </div>

            {/* File queue */}
            {files.length > 0 && (
                <div className="file-queue">
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
                                                title="Re-transcribe with current engine"
                                            >
                                                Re-run
                                            </button>
                                        </>
                                    )}
                                    {item.status === "error" && (
                                        <button
                                            className="file-card-btn file-card-btn--error"
                                            onClick={() => retranscribe(item)}
                                            title="Retry"
                                        >
                                            Retry
                                        </button>
                                    )}
                                    {item.status === "cancelled" && (
                                        <button
                                            className="file-card-btn file-card-btn--secondary"
                                            onClick={() => retranscribe(item)}
                                            title="Transcribe again"
                                        >
                                            Retry
                                        </button>
                                    )}
                                    {item.status !== "processing" && (
                                        <button
                                            className="file-card-btn file-card-btn--remove"
                                            onClick={() => removeFile(item.id)}
                                            title="Remove"
                                            aria-label={`Remove ${item.name}`}
                                        >
                                            ✕
                                        </button>
                                    )}
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

                            {/* Transcript */}
                            {item.status === "done" && item.transcript && (
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
