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
    /** Show the "⚡ ANE Accelerated" badge (Apple Silicon + ANE-capable Whisper model). */
    showAneBadge?: boolean;
}

type DeletePhase = 'idle' | 'confirm' | 'deleting' | 'deleted';

export function ModelRow({ model, downloadProgress, onDownload, onDelete, onCancelDownload, showAneBadge }: ModelRowProps) {
    const progress = downloadProgress[model.id];
    const graniteBadge = model.type === 'Granite'
        ? model.id.includes('cuda') ? 'CUDA' : 'PORTABLE'
        : null;
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
        : model.type === 'Whisper' || model.type === 'CoreML' ? 'model-tag--whisper'
        : model.type === 'Granite' ? 'model-tag--cohere'
        : 'model-tag--default';

    return (
        <div
            className="model-item"
            id={`model-row-${model.id}`}
            data-testid={`model-row-${model.id}`}
            role="region"
            aria-label={`Model ${model.name}`}
        >
            <div className="model-info">
                <div className="model-title-row">
                    <h3>{model.name}</h3>
                    {graniteBadge && (
                        <span className={`model-hardware-badge${graniteBadge === 'CUDA' ? ' model-hardware-badge--cuda' : ' model-hardware-badge--portable'}`}>
                            {graniteBadge}
                        </span>
                    )}
                    {showAneBadge && (
                        <span
                            className="model-hardware-badge model-hardware-badge--ane"
                            title="Downloads with the Apple Neural Engine encoder on this Mac — up to 85x real-time"
                        >
                            ⚡ ANE Accelerated
                        </span>
                    )}
                </div>
                <div className="model-meta">
                    <span className={`model-tag ${tagClass}`}>{model.type}</span>
                    <span>{model.size}</span>
                </div>
                <p className="model-desc">{model.description}</p>
                {deleteError && (
                    <p
                        id={`model-delete-error-${model.id}`}
                        data-testid={`model-delete-error-${model.id}`}
                        role="alert"
                        className="model-delete-error"
                    >
                        {deleteError}
                    </p>
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
                                        id={`model-cancel-download-btn-${model.id}`}
                                        data-testid={`model-cancel-download-btn-${model.id}`}
                                        className="model-cancel-btn"
                                        onClick={() => onCancelDownload(model.id)}
                                        aria-label={`Cancel download of ${model.name}`}
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
                                        type="button"
                                        id={`model-confirm-delete-yes-${model.id}`}
                                        data-testid={`model-confirm-delete-yes-${model.id}`}
                                        className="delete-confirm-btn delete-confirm-btn--yes"
                                        onClick={handleConfirmDelete}
                                        aria-label={`Confirm delete ${model.name}`}
                                    >
                                        Yes
                                    </button>
                                    <button
                                        type="button"
                                        id={`model-confirm-delete-no-${model.id}`}
                                        data-testid={`model-confirm-delete-no-${model.id}`}
                                        className="delete-confirm-btn delete-confirm-btn--no"
                                        onClick={handleCancelDelete}
                                        aria-label={`Cancel delete ${model.name}`}
                                    >
                                        No
                                    </button>
                                </div>
                            ) : (
                                <>
                                    <button
                                        type="button"
                                        id={`model-delete-btn-${model.id}`}
                                        data-testid={`model-delete-btn-${model.id}`}
                                        className="model-delete-icon-btn"
                                        onClick={handleDeleteClick}
                                        title="Delete Model"
                                        aria-label={`Delete ${model.name}`}
                                    >
                                        <IconTrash size={16} />
                                    </button>

                                    <button
                                        type="button"
                                        id={`model-status-badge-${model.id}`}
                                        data-testid={`model-status-badge-${model.id}`}
                                        className={`download-btn downloaded${!model.verified ? ' download-btn--unverified' : ''}`}
                                        disabled
                                        aria-label={`${model.name} is ${model.verified ? 'Verified' : 'Installed'}`}
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
                                type="button"
                                id={`model-download-btn-${model.id}`}
                                data-testid={`model-download-btn-${model.id}`}
                                className="download-btn"
                                onClick={() => onDownload(model.id, model.name)}
                                aria-label={`Download ${model.name}`}
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
