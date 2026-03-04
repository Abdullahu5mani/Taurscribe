import { useState, useRef, useEffect } from "react";
import type { DownloadableModel } from "./types";
import type { DownloadProgress } from "./types";
import { IconShieldCheck, IconTrash, IconCheck, IconRetry, IconWarning, IconDownload } from "../Icons";

interface ModelRowProps {
    model: DownloadableModel;
    downloadProgress: Record<string, DownloadProgress>;
    onDownload: (id: string, name: string) => void;
    onDelete: (id: string, name: string) => Promise<void>;
}

type DeletePhase = 'idle' | 'confirm' | 'deleting' | 'deleted';

export function ModelRow({ model, downloadProgress, onDownload, onDelete }: ModelRowProps) {
    const progress = downloadProgress[model.id];
    const [deletePhase, setDeletePhase] = useState<DeletePhase>('idle');
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
        try {
            await onDelete(model.id, model.name);
            setDeletePhase('deleted');
            // Show "Deleted ✓" for 1.5s before resetting
            setTimeout(() => setDeletePhase('idle'), 1500);
        } catch {
            setDeletePhase('idle');
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
                <p style={{ margin: '8px 0 0 0', fontSize: '0.9rem', color: '#94a3b8' }}>
                    {model.description}
                </p>
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
                ) : progress && !model.downloaded ? (
                    /* ── Download progress bar ──────────────────────────── */
                    <div style={{ width: '100%' }}>
                        <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: '0.75rem', marginBottom: '4px', color: '#94a3b8' }}>
                            <span>
                                {(progress.total_files || 0) > 1 ?
                                    `Downloading (${progress.current_file || 1}/${progress.total_files || 1})...` :
                                    'Downloading...'}
                            </span>
                            <span>{progress.total > 0 ? Math.round((progress.bytes / progress.total) * 100) : 0}%</span>
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
                                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', fontSize: '0.75rem', marginBottom: '4px', color: '#f87171' }}>
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
                        {progress?.status === 'error' ? (
                            <button
                                className="download-btn"
                                onClick={() => onDownload(model.id, model.name)}
                                style={{
                                    background: 'rgba(239, 68, 68, 0.15)',
                                    color: '#f87171',
                                    border: '1px solid rgba(239, 68, 68, 0.4)',
                                    display: 'flex',
                                    alignItems: 'center',
                                    gap: '6px'
                                }}
                            >
                                <IconRetry size={14} /> Retry Download
                            </button>
                        ) : model.downloaded ? (
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
                                        style={{
                                            background: 'rgba(239, 68, 68, 0.1)',
                                            color: '#ef4444',
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
