import type { DownloadableModel } from "./types";
import type { DownloadProgress } from "./types";

interface ModelRowProps {
    model: DownloadableModel;
    downloadProgress: Record<string, DownloadProgress>;
    onDownload: (id: string, name: string) => void;
    onDelete: (id: string, name: string) => void;
    onVerify: (id: string, name: string) => void;
}

export function ModelRow({ model, downloadProgress, onDownload, onDelete, onVerify }: ModelRowProps) {
    const progress = downloadProgress[model.id];

    return (
        <div className="model-item">
            <div className="model-info">
                <h3>{model.name}</h3>
                <div className="model-meta">
                    <span className="model-tag" style={{
                        background: model.type === 'LLM' ? 'rgba(236, 72, 153, 0.15)' :
                            model.type === 'Parakeet' ? 'rgba(16, 185, 129, 0.15)' :
                                model.type === 'Utility' ? 'rgba(245, 158, 11, 0.15)' :
                                    'rgba(148, 163, 184, 0.1)',
                        color: model.type === 'LLM' ? '#f472b6' :
                            model.type === 'Parakeet' ? '#34d399' :
                                model.type === 'Utility' ? '#fbbf24' :
                                    'inherit'
                    }}>{model.type}</span>
                    <span>{model.size}</span>
                </div>
                <p style={{ margin: '8px 0 0 0', fontSize: '0.9rem', color: '#94a3b8' }}>
                    {model.description}
                </p>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: '8px', minWidth: '160px' }}>
                {progress && !model.downloaded ? (
                    <div style={{ width: '100%' }}>
                        <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: '0.75rem', marginBottom: '4px', color: '#94a3b8' }}>
                            <span>
                                {progress.status === 'verifying' ? 'Verifying...' :
                                    ((progress.total_files || 0) > 1 ?
                                        `Downloading (${progress.current_file || 1}/${progress.total_files || 1})...` :
                                        'Downloading...')}
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
                ) : (
                    <div style={{ display: 'flex', gap: '8px' }}>
                        {model.downloaded && (
                            <>
                                <button
                                    className="delete-btn"
                                    onClick={() => onDelete(model.id, model.name)}
                                    disabled={!!progress}
                                    title={progress ? "Please wait…" : "Delete Model"}
                                    style={{
                                        background: 'rgba(239, 68, 68, 0.1)',
                                        color: '#ef4444',
                                        border: '1px solid rgba(239, 68, 68, 0.2)',
                                        padding: '8px 12px',
                                        borderRadius: '6px',
                                        cursor: progress ? 'not-allowed' : 'pointer',
                                        fontSize: '1rem',
                                        transition: 'all 0.2s',
                                        opacity: progress ? 0.6 : 1
                                    }}
                                >
                                    🗑️
                                </button>

                                {!model.verified && (
                                    <button
                                        onClick={() => onVerify(model.id, model.name)}
                                        disabled={!!progress}
                                        title={progress ? "Please wait…" : "Deep Verify Hash (SHA1)"}
                                        style={{
                                            background: 'rgba(148, 163, 184, 0.1)',
                                            color: '#94a3b8',
                                            border: '1px solid rgba(148, 163, 184, 0.2)',
                                            padding: '8px 12px',
                                            borderRadius: '6px',
                                            cursor: progress ? 'not-allowed' : 'pointer',
                                            fontSize: '1rem',
                                            transition: 'all 0.2s',
                                            opacity: progress ? 0.6 : 1
                                        }}
                                    >
                                        🔍
                                    </button>
                                )}
                            </>
                        )}

                        <button
                            className={`download-btn ${model.downloaded ? 'downloaded' : ''}`}
                            onClick={() => (!model.downloaded || (model.downloaded && !model.verified)) && onDownload(model.id, model.name)}
                            disabled={(model.downloaded && model.verified) || !!progress}
                            title={model.verified ? "Verified Integrity" : (model.downloaded ? "Click to Repair/Re-download" : "Download Model")}
                            style={model.downloaded && !model.verified ? { background: '#eab308', color: '#000', borderColor: '#ca8a04' } : {}}
                        >
                            {model.downloaded ? (
                                model.verified ? (
                                    <><span>🛡️</span> Verified</>
                                ) : (
                                    <><span>✓</span> Installed</>
                                )
                            ) : (
                                <><span>⬇</span> Download</>
                            )}
                        </button>
                    </div>
                )}
            </div>
        </div>
    );
}
