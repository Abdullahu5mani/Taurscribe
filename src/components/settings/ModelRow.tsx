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

    return (
        <div className="model-item">
            <div className="model-info">
                <h3>{model.name}</h3>
                <div className="model-meta">
                    <span className="model-tag" style={{
                        background: model.type === 'LLM' ? 'rgba(236, 72, 153, 0.15)' :
                            model.type === 'Parakeet' ? 'rgba(16, 185, 129, 0.15)' :
                                'rgba(148, 163, 184, 0.1)',
                        color: model.type === 'LLM' ? '#f472b6' :
                            model.type === 'Parakeet' ? '#34d399' :
                                'inherit'
                    }}>{model.type}</span>
                    <span>{model.size}</span>
                </div>
                <p style={{ margin: '8px 0 0 0', fontSize: '0.9rem', color: 'var(--text-secondary)' }}>
                    {model.description}
                </p>
                {deleteError && (
                    <p role="alert" style={{ margin: '6px 0 0 0', fontSize: '0.78rem', color: 'var(--error-light, #f87171)' }}>
                        {deleteError}
                    </p>
                )}
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: '8px', minWidth: '180px', flexShrink: 0, marginLeft: '16px' }}>
                {progress?.status === 'verifying' ? (
                    /* ── Verification in-progress — real progress bar ──── */
                    <div style={{ width: '100%' }}>
                        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', fontSize: '0.75rem', marginBottom: '4px', color: '#06b6d4' }}>
                            <span style={{ display: 'flex', alignItems: 'center', gap: '5px' }}>
                                <span className="verify-pulse"><IconShieldCheck size={14} /></span>
                                Verifying{(progress.total_files || 0) > 1 ? ` (${progress.current_file || 1}/${progress.total_files})` : ''}...
                            </span>
                            <span>{progress.total > 0 ? Math.round((progress.bytes / progress.total) * 100) : 0}%</span>
                        </div>
                        <div style={{ height: '6px', width: '100%', background: 'rgba(6, 182, 212, 0.12)', borderRadius: '3px', overflow: 'hidden' }}>
                            <div style={{
                                height: '100%',
                                width: `${progress.total > 0 ? (progress.bytes / progress.total) * 100 : 0}%`,
                                background: '#06b6d4',
                                transition: 'width 0.15s ease-out'
                            }} />
                        </div>
                    </div>
                ) : progress?.status === 'extracting' ? (
                    /* ── Extraction in-progress — purple bar ─────────── */
                    <div style={{ width: '100%' }}>
                        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', fontSize: '0.75rem', marginBottom: '4px', color: '#a855f7' }}>
                            <span>Extracting...</span>
                            <span>{progress.total > 0 ? Math.round((progress.bytes / progress.total) * 100) : 0}%</span>
                        </div>
                        <div style={{ height: '6px', width: '100%', background: 'rgba(168, 85, 247, 0.12)', borderRadius: '3px', overflow: 'hidden' }}>
                            <div style={{
                                height: '100%',
                                width: `${progress.total > 0 ? (progress.bytes / progress.total) * 100 : 0}%`,
                                background: 'linear-gradient(90deg, #a855f7, #c084fc)',
                                transition: 'width 0.1s ease-out',
                            }} />
                        </div>
                    </div>
                ) : progress && !model.downloaded ? (
                    /* ── Download progress bar ──────────────────────────── */
                    <div style={{ width: '100%' }}>
                        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', fontSize: '0.75rem', marginBottom: '4px', color: 'var(--text-secondary)' }}>
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
                                        onClick={() => onCancelDownload(model.id)}
                                        title="Cancel download and delete partial files"
                                        style={{
                                            background: 'rgba(239, 68, 68, 0.15)',
                                            color: 'var(--error-light, #f87171)',
                                            border: '1px solid rgba(239, 68, 68, 0.3)',
                                            borderRadius: '4px',
                                            padding: '1px 6px',
                                            fontSize: '0.7rem',
                                            cursor: 'pointer',
                                            lineHeight: '1.4',
                                        }}
                                    >
                                        Cancel
                                    </button>
                                )}
                            </div>
                        </div>
                        <div style={{ height: '6px', width: '100%', background: 'rgba(255,255,255,0.1)', borderRadius: '3px', overflow: 'hidden' }}>
                            <div style={{
                                height: '100%',
                                width: `${progress.total > 0 ? (progress.bytes / progress.total) * 100 : 0}%`,
                                background: '#06b6d4',
                                transition: 'width 0.2s'
                            }} />
                        </div>
                    </div>
                ) : deletePhase === 'deleting' ? (
                    /* ── Deleting in-progress — real progress bar ──────── */
                    (() => {
                        const delProgress = progress?.status === 'deleting' ? progress : null;
                        const pct = delProgress && delProgress.total > 0 ? Math.round((delProgress.bytes / delProgress.total) * 100) : 0;
                        return (
                            <div style={{ width: '100%' }}>
                                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', fontSize: '0.75rem', marginBottom: '4px', color: 'var(--error-light, #f87171)' }}>
                                    <span style={{ display: 'flex', alignItems: 'center', gap: '5px' }}>
                                        <span className="verify-pulse"><IconTrash size={14} /></span>
                                        Deleting{delProgress && (delProgress.total_files || 0) > 1 ? ` (${delProgress.current_file || 1}/${delProgress.total_files})` : ''}...
                                    </span>
                                    <span>{pct}%</span>
                                </div>
                                <div style={{ height: '6px', width: '100%', background: 'rgba(239, 68, 68, 0.12)', borderRadius: '3px', overflow: 'hidden' }}>
                                    <div style={{
                                        height: '100%',
                                        width: `${pct}%`,
                                        background: '#ef4444',
                                        transition: 'width 0.15s ease-out'
                                    }} />
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
                    <div style={{ display: 'flex', gap: '8px' }}>
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
                                        className="delete-btn"
                                        onClick={handleDeleteClick}
                                        title="Delete Model"
                                        aria-label={`Delete ${model.name}`}
                                        style={{
                                            background: 'rgba(239, 68, 68, 0.1)',
                                            color: 'var(--error, #ef4444)',
                                            border: '1px solid rgba(239, 68, 68, 0.2)',
                                            padding: '8px 12px',
                                            borderRadius: '6px',
                                            cursor: 'pointer',
                                            fontSize: '1rem',
                                            transition: 'all 0.2s',
                                        }}
                                    >
                                        <IconTrash size={16} />
                                    </button>

                                    <button
                                        className="download-btn downloaded"
                                        disabled
                                        title={model.verified ? "Verified Integrity" : "Unverified"}
                                        style={!model.verified ? { background: '#eab308', color: '#000', borderColor: '#ca8a04' } : {}}
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
