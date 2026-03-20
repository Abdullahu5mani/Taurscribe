import { useState, useRef, useEffect } from "react";
import type { DownloadableModel } from "./types";
import type { DownloadProgress } from "./types";
import { IconShieldCheck, IconTrash, IconCheck, IconWarning, IconDownload } from "../Icons";

interface ModelRowProps {
    model: DownloadableModel;
    downloadProgress: Record<string, DownloadProgress>;
    onDownload: (id: string, name: string) => void;
    onDelete: (id: string, name: string) => Promise<void>;
    onCancelDownload: (id: string) => void;
}

type DeletePhase = 'idle' | 'confirm' | 'deleting' | 'deleted';

export function ModelRow({ model, downloadProgress, onDownload, onDelete, onCancelDownload }: ModelRowProps) {
    const progress = downloadProgress[model.id];
    const [deletePhase, setDeletePhase] = useState<DeletePhase>('idle');
    const [deleteError, setDeleteError] = useState<string | null>(null);
    const confirmTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    // Auto-cancel the confirm prompt after 4s of inactivity
    useEffect(() => {
        if (deletePhase === 'confirm') {
            confirmTimerRef.current = setTimeout(() => setDeletePhase('idle'), 4000);
        }
        return () => {
            if (confirmTimerRef.current) clearTimeout(confirmTimerRef.current);
        };
    }, [deletePhase]);

    const handleDeleteClick = () => {
        if (deletePhase === 'idle') {
            setDeletePhase('confirm');
        }
    };

    const handleConfirmDelete = async () => {
        if (confirmTimerRef.current) clearTimeout(confirmTimerRef.current);
        setDeletePhase('deleting');
        setDeleteError(null);
        try {
            await onDelete(model.id, model.name);
            setDeletePhase('deleted');
            setTimeout(() => setDeletePhase('idle'), 1500);
        } catch (err) {
            setDeletePhase('idle');
            setDeleteError(err instanceof Error ? err.message : 'Delete failed');
            setTimeout(() => setDeleteError(null), 5000);
        }
    };

    const handleCancelDelete = () => {
        if (confirmTimerRef.current) clearTimeout(confirmTimerRef.current);
        setDeletePhase('idle');
    };

    const tagClass = model.type === 'LLM' ? 'model-tag--llm'
        : model.type === 'Parakeet' ? 'model-tag--parakeet'
        : 'model-tag--default';

    return (
        <div className="model-item">
            <div className="model-info">
                <h3>{model.name}</h3>
                <div className="model-meta">
                    <span className={`model-tag ${tagClass}`}>{model.type}</span>
                    <span>{model.size}</span>
                </div>
                <p className="model-desc">{model.description}</p>
                {deleteError && (
                    <p role="alert" className="model-delete-error">{deleteError}</p>
                )}
            </div>
            <div className="model-row-actions">
                {progress?.status === 'verifying' ? (
                    /* ── Verification in-progress — real progress bar ──── */
                    <div className="model-progress-area">
                        <div className="model-progress-header model-progress-header--verify">
                            <span style={{ display: 'flex', alignItems: 'center', gap: '5px' }}>
                                <span className="verify-pulse"><IconShieldCheck size={14} /></span>
                                Verifying{(progress.total_files || 0) > 1 ? ` (${progress.current_file || 1}/${progress.total_files})` : ''}...
                            </span>
                            <span>{progress.total > 0 ? Math.round((progress.bytes / progress.total) * 100) : 0}%</span>
                        </div>
                        <div className="progress-track progress-track--verify">
                            <div className="progress-fill progress-fill--verify" style={{ width: `${progress.total > 0 ? (progress.bytes / progress.total) * 100 : 0}%` }} />
                        </div>
                    </div>
                ) : progress?.status === 'extracting' ? (
                    /* ── Extraction in-progress — purple bar ─────────── */
                    <div className="model-progress-area">
                        <div className="model-progress-header model-progress-header--extract">
                            <span>Extracting...</span>
                            <span>{progress.total > 0 ? Math.round((progress.bytes / progress.total) * 100) : 0}%</span>
                        </div>
                        <div className="progress-track progress-track--extract">
                            <div className="progress-fill progress-fill--extract" style={{ width: `${progress.total > 0 ? (progress.bytes / progress.total) * 100 : 0}%` }} />
                        </div>
                    </div>
                ) : progress && !model.downloaded ? (
                    /* ── Download progress bar ──────────────────────────── */
                    <div className="model-progress-area">
                        <div className="model-progress-header model-progress-header--download">
                            <span>
                                {progress.status === 'starting' ? 'Starting download...' :
                                    progress.status === 'finalizing' ? 'Finalizing...' :
                                (progress.total_files || 0) > 1 ?
                                    `Downloading (${progress.current_file || 1}/${progress.total_files || 1})...` :
                                    'Downloading...'}
                            </span>
                            <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                                <span>{progress.total > 0 ? Math.round((progress.bytes / progress.total) * 100) : 0}%</span>
                                {progress.status !== 'finalizing' && (
                                    <button
                                        type="button"
                                        className="model-cancel-btn"
                                        onClick={() => onCancelDownload(model.id)}
                                        title="Cancel download and delete partial files"
                                    >
                                        Cancel
                                    </button>
                                )}
                            </div>
                        </div>
                        <div className="progress-track progress-track--download">
                            <div className="progress-fill progress-fill--download" style={{ width: `${progress.total > 0 ? (progress.bytes / progress.total) * 100 : 0}%` }} />
                        </div>
                    </div>
                ) : deletePhase === 'deleting' ? (
                    /* ── Deleting in-progress — real progress bar ──────── */
                    (() => {
                        const delProgress = progress?.status === 'deleting' ? progress : null;
                        const pct = delProgress && delProgress.total > 0 ? Math.round((delProgress.bytes / delProgress.total) * 100) : 0;
                        return (
                            <div className="model-progress-area">
                                <div className="model-progress-header model-progress-header--delete">
                                    <span style={{ display: 'flex', alignItems: 'center', gap: '5px' }}>
                                        <span className="verify-pulse"><IconTrash size={14} /></span>
                                        Deleting{delProgress && (delProgress.total_files || 0) > 1 ? ` (${delProgress.current_file || 1}/${delProgress.total_files})` : ''}...
                                    </span>
                                    <span>{pct}%</span>
                                </div>
                                <div className="progress-track progress-track--delete">
                                    <div className="progress-fill progress-fill--delete" style={{ width: `${pct}%` }} />
                                </div>
                            </div>
                        );
                    })()
                ) : deletePhase === 'deleted' ? (
                    /* ── Deleted confirmation ────────────────────────────── */
                    <div className="delete-confirmed">
                        <IconCheck size={14} /> Deleted
                    </div>
                ) : (
                    <div className="model-buttons-row">
                        {model.downloaded ? (
                            deletePhase === 'confirm' ? (
                                /* ── Confirm / Cancel inline prompt ─────────── */
                                <div className="delete-confirm-row">
                                    <span className="delete-confirm-label">Delete?</span>
                                    <button
                                        className="delete-confirm-btn delete-confirm-btn--yes"
                                        onClick={handleConfirmDelete}
                                    >
                                        Yes
                                    </button>
                                    <button
                                        className="delete-confirm-btn delete-confirm-btn--no"
                                        onClick={handleCancelDelete}
                                    >
                                        No
                                    </button>
                                </div>
                            ) : (
                                <>
                                    <button
                                        className="model-delete-icon-btn"
                                        onClick={handleDeleteClick}
                                        title="Delete Model"
                                        aria-label={`Delete ${model.name}`}
                                    >
                                        <IconTrash size={16} />
                                    </button>

                                    <button
                                        className={`download-btn downloaded${!model.verified ? ' download-btn--unverified' : ''}`}
                                        disabled
                                        title={model.verified ? "Verified Integrity" : "Unverified"}
                                    >
                                        {model.verified ? (
                                            <><IconShieldCheck size={14} /> Verified</>
                                        ) : (
                                            <><IconWarning size={14} /> Installed</>
                                        )}
                                    </button>
                                </>
                            )
                        ) : (
                            <button
                                className="download-btn"
                                onClick={() => onDownload(model.id, model.name)}
                                title="Download Model"
                            >
                                <IconDownload size={14} /> Download
                            </button>
                        )}
                    </div>
                )}
            </div>
        </div>
    );
}
