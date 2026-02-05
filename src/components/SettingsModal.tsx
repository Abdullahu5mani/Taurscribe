import { useState } from 'react';
import './SettingsModal.css';
import { toast } from 'sonner';

interface SettingsModalProps {
    isOpen: boolean;
    onClose: () => void;
}

type Tab = 'general' | 'downloads' | 'audio' | 'vad' | 'llm';

interface DownloadableModel {
    id: string;
    name: string;
    type: 'Whisper' | 'Parakeet' | 'LLM';
    size: string;
    description: string;
    downloaded: boolean;
    verified?: boolean;
}

const MOCK_MODELS: DownloadableModel[] = [
    // --- Tiny ---
    { id: 'whisper-tiny', name: 'Tiny (Multilingual)', type: 'Whisper', size: '75 MB', description: 'Fastest, lowest accuracy.', downloaded: false },
    { id: 'whisper-tiny-q5_1', name: 'Tiny (Multi, Q5_1)', type: 'Whisper', size: '31 MB', description: 'Quantized Tiny.', downloaded: false },
    { id: 'whisper-tiny-en', name: 'Tiny (English)', type: 'Whisper', size: '75 MB', description: 'English-only Tiny.', downloaded: false },
    { id: 'whisper-tiny-en-q5_1', name: 'Tiny (English, Q5_1)', type: 'Whisper', size: '31 MB', description: 'Quantized English Tiny.', downloaded: false },

    // --- Base ---
    { id: 'whisper-base', name: 'Base (Multilingual)', type: 'Whisper', size: '142 MB', description: 'Balanced entry model.', downloaded: false },
    { id: 'whisper-base-en', name: 'Base (English)', type: 'Whisper', size: '142 MB', description: 'Standard English model.', downloaded: false },
    { id: 'whisper-base-q5_1', name: 'Base (Multi, Q5_1)', type: 'Whisper', size: '57 MB', description: 'Quantized Base.', downloaded: false },

    // --- Small ---
    { id: 'whisper-small', name: 'Small (Multilingual)', type: 'Whisper', size: '466 MB', description: 'Good accuracy for general use.', downloaded: false },
    { id: 'whisper-small-en', name: 'Small (English)', type: 'Whisper', size: '466 MB', description: 'Good accuracy English model.', downloaded: false },
    { id: 'whisper-small-q5_1', name: 'Small (Multi, Q5_1)', type: 'Whisper', size: '181 MB', description: 'Quantized Small.', downloaded: false },
    { id: 'whisper-small-en-q5_1', name: 'Small (English, Q5_1)', type: 'Whisper', size: '181 MB', description: 'Quantized English Small.', downloaded: false },

    // --- Medium ---
    { id: 'whisper-medium', name: 'Medium (Multilingual)', type: 'Whisper', size: '1.5 GB', description: 'High accuracy, slower.', downloaded: false },
    { id: 'whisper-medium-en', name: 'Medium (English)', type: 'Whisper', size: '1.5 GB', description: 'High accuracy English.', downloaded: false },
    { id: 'whisper-medium-q5_0', name: 'Medium (Multi, Q5_0)', type: 'Whisper', size: '514 MB', description: 'Quantized Medium.', downloaded: false },
    { id: 'whisper-medium-en-q5_0', name: 'Medium (English, Q5_0)', type: 'Whisper', size: '514 MB', description: 'Quantized English Medium.', downloaded: false },

    // --- Large ---
    { id: 'whisper-large-v3', name: 'Large V3 (Multilingual)', type: 'Whisper', size: '2.9 GB', description: 'State of the art accuracy.', downloaded: false },
    { id: 'whisper-large-v3-q5_0', name: 'Large V3 (Multi, Q5_0)', type: 'Whisper', size: '1.1 GB', description: 'Quantized Large V3.', downloaded: false },
    { id: 'whisper-large-v3-turbo', name: 'Large V3 Turbo', type: 'Whisper', size: '1.5 GB', description: 'Optimized Large V3.', downloaded: false },
    { id: 'whisper-large-v3-turbo-q5_0', name: 'Large V3 Turbo (Q5_0)', type: 'Whisper', size: '547 MB', description: 'Quantized Turbo.', downloaded: false },

    // --- Parakeet ---
    { id: 'parakeet-nemotron', name: 'Nemotron Streaming', type: 'Parakeet', size: '1.2 GB', description: 'Ultra-low latency streaming.', downloaded: true },
];

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect } from 'react';

// ... (other imports)

interface DownloadProgressPayload {
    model_id: string;
    total_bytes: number;
    downloaded_bytes: number;
    status: string;
}

// ... (MOCK_MODELS definition)

export function SettingsModal({ isOpen, onClose }: SettingsModalProps) {
    const [activeTab, setActiveTab] = useState<Tab>('downloads');
    const [models, setModels] = useState(MOCK_MODELS);
    const [downloadProgress, setDownloadProgress] = useState<Record<string, { bytes: number, total: number, status: string }>>({});

    // VAD Settings Mock State
    const [vadSensitivity, setVadSensitivity] = useState(50);
    const [vadEnergyThreshold, setVadEnergyThreshold] = useState(0.005);

    useEffect(() => {
        let unlisten: (() => void) | undefined;

        const setupListener = async () => {
            unlisten = await listen("download-progress", (event) => {
                const payload = event.payload as DownloadProgressPayload;

                setDownloadProgress(prev => ({
                    ...prev,
                    [payload.model_id]: {
                        bytes: payload.downloaded_bytes,
                        total: payload.total_bytes,
                        status: payload.status
                    }
                }));

                if (payload.status === "done") {
                    toast.success(`Download complete: ${payload.model_id}`);
                    setModels(prev => prev.map(m =>
                        m.id === payload.model_id ? { ...m, downloaded: true, verified: false } : m
                    ));
                }
            });
        };

        if (isOpen) {
            setupListener();

            // Fetch status on open
            const fetchStatus = async () => {
                try {
                    const modelIds = models.map(m => m.id);
                    const statuses = await invoke("get_download_status", { modelIds }) as any[];

                    setModels(prev => prev.map(m => {
                        const status = statuses.find((s: any) => s.id === m.id);
                        if (status) {
                            return { ...m, downloaded: status.downloaded, verified: status.verified };
                        }
                        return m;
                    }));
                } catch (e) {
                    console.error("Failed to fetch model status", e);
                }
            };

            fetchStatus();
        }

        return () => {
            if (unlisten) unlisten();
        };
    }, [isOpen]);

    if (!isOpen) return null;

    const handleDownload = async (id: string, name: string) => {
        toast.info(`Starting download for ${name}...`);
        try {
            setDownloadProgress(prev => ({
                ...prev,
                [id]: { bytes: 0, total: 100, status: 'starting' } // Init dummy state
            }));

            await invoke("download_model", { modelId: id });
        } catch (e) {
            toast.error(`Download failed: ${e}`);
            setDownloadProgress(prev => {
                const newState = { ...prev };
                delete newState[id];
                return newState;
            });
        }
    };

    const handleDelete = async (id: string, name: string) => {
        if (!confirm(`Are you sure you want to delete ${name}?`)) return;

        try {
            await invoke("delete_model", { modelId: id });
            toast.success(`Deleted ${name}`);
            setModels(prev => prev.map(m =>
                m.id === id ? { ...m, downloaded: false } : m
            ));

            // Clear progress so it doesn't show "Downloading... 100%"
            setDownloadProgress(prev => {
                const newState = { ...prev };
                delete newState[id];
                return newState;
            });
        } catch (e) {
            toast.error(`Failed to delete: ${e}`);
        }
    };



    const handleVerifyHash = async (id: string, name: string) => {
        toast.info(`Verifying integrity of ${name}...`);
        try {
            setDownloadProgress(prev => ({
                ...prev,
                [id]: { bytes: 0, total: 100, status: 'verifying' }
            }));

            await invoke("verify_model_hash", { modelId: id });
            toast.success(`Verification Successful: ${name} matches official SHA1 hash.`);

            setModels(prev => prev.map(m =>
                m.id === id ? { ...m, verified: true } : m
            ));
        } catch (e) {
            toast.error(`Verification Failed: ${e}`);
        } finally {
            setDownloadProgress(prev => {
                const newState = { ...prev };
                delete newState[id];
                return newState;
            });
        }
    };

    const renderContent = () => {
        switch (activeTab) {
            case 'downloads':
                return (
                    <div className="download-manager">
                        <h3 className="settings-section-title">Model Library</h3>
                        <div style={{ marginBottom: '16px', fontSize: '0.9rem', color: '#94a3b8', background: 'rgba(59, 130, 246, 0.1)', padding: '12px', borderRadius: '8px', border: '1px solid rgba(59, 130, 246, 0.2)' }}>
                            📂 <strong>Storage Location:</strong> <code style={{ fontFamily: 'monospace', color: '#e2e8f0' }}>%AppData%\Taurscribe\models</code>
                        </div>
                        <div className="model-list">
                            {models.map(model => (
                                <div key={model.id} className="model-item">
                                    <div className="model-info">
                                        <h3>{model.name}</h3>
                                        <div className="model-meta">
                                            <span className="model-tag" style={{
                                                background: model.type === 'LLM' ? 'rgba(236, 72, 153, 0.15)' : 'rgba(148, 163, 184, 0.1)',
                                                color: model.type === 'LLM' ? '#f472b6' : 'inherit'
                                            }}>{model.type}</span>
                                            <span>{model.size}</span>
                                        </div>
                                        <p style={{ margin: '8px 0 0 0', fontSize: '0.9rem', color: '#94a3b8' }}>
                                            {model.description}
                                        </p>
                                    </div>
                                    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: '8px', minWidth: '160px' }}>
                                        {downloadProgress[model.id] && !model.downloaded ? (
                                            <div style={{ width: '100%' }}>
                                                <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: '0.75rem', marginBottom: '4px', color: '#94a3b8' }}>
                                                    <span>{downloadProgress[model.id].status === 'verifying' ? 'Verifying...' : 'Downloading...'}</span>
                                                    <span>{downloadProgress[model.id].total > 0 ? Math.round((downloadProgress[model.id].bytes / downloadProgress[model.id].total) * 100) : 0}%</span>
                                                </div>
                                                <div style={{ height: '6px', width: '100%', background: 'rgba(255,255,255,0.1)', borderRadius: '3px', overflow: 'hidden' }}>
                                                    <div style={{
                                                        height: '100%',
                                                        width: `${downloadProgress[model.id].total > 0 ? (downloadProgress[model.id].bytes / downloadProgress[model.id].total) * 100 : 0}%`,
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
                                                            onClick={() => handleDelete(model.id, model.name)}
                                                            title="Delete Model"
                                                            style={{
                                                                background: 'rgba(239, 68, 68, 0.1)',
                                                                color: '#ef4444',
                                                                border: '1px solid rgba(239, 68, 68, 0.2)',
                                                                padding: '8px 12px',
                                                                borderRadius: '6px',
                                                                cursor: 'pointer',
                                                                fontSize: '1rem',
                                                                transition: 'all 0.2s'
                                                            }}
                                                        >
                                                            🗑️
                                                        </button>

                                                        {!model.verified && (
                                                            <button
                                                                onClick={() => handleVerifyHash(model.id, model.name)}
                                                                title="Deep Verify Hash (SHA1)"
                                                                style={{
                                                                    background: 'rgba(148, 163, 184, 0.1)',
                                                                    color: '#94a3b8',
                                                                    border: '1px solid rgba(148, 163, 184, 0.2)',
                                                                    padding: '8px 12px',
                                                                    borderRadius: '6px',
                                                                    cursor: 'pointer',
                                                                    fontSize: '1rem',
                                                                    transition: 'all 0.2s'
                                                                }}
                                                            >
                                                                🔍
                                                            </button>
                                                        )}
                                                    </>
                                                )}

                                                <button
                                                    className={`download-btn ${model.downloaded ? 'downloaded' : ''}`}
                                                    onClick={() => (!model.downloaded || (model.downloaded && !model.verified)) && handleDownload(model.id, model.name)}
                                                    disabled={model.downloaded && model.verified || !!downloadProgress[model.id]}
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
                            ))}
                        </div>
                    </div>
                );
            case 'general':
                return (
                    <div className="general-settings">
                        <h3 className="settings-section-title">General Settings</h3>
                        <p style={{ color: '#64748b' }}>App preferences coming soon...</p>
                    </div>
                );
            case 'audio':
                return (
                    <div className="audio-settings">
                        <h3 className="settings-section-title">Audio & Microphone</h3>
                        <p style={{ color: '#64748b' }}>Device selection coming soon...</p>
                    </div>
                );
            case 'vad':
                return (
                    <div className="vad-settings">
                        <h3 className="settings-section-title">Voice Activity Detection (VAD)</h3>
                        <p style={{ color: '#94a3b8', marginBottom: '24px' }}>
                            Configure how the AI detects when you are speaking versus background noise.
                        </p>

                        <div style={{ background: 'rgba(30, 41, 59, 0.4)', padding: '24px', borderRadius: '12px', border: '1px solid rgba(148, 163, 184, 0.1)' }}>
                            <div style={{ marginBottom: '20px' }}>
                                <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '8px' }}>
                                    <label style={{ fontWeight: 600 }}>Silence Threshold</label>
                                    <span style={{ fontFamily: 'monospace', color: '#60a5fa' }}>{vadEnergyThreshold} RMS</span>
                                </div>
                                <input
                                    type="range"
                                    min="0.001"
                                    max="0.05"
                                    step="0.001"
                                    value={vadEnergyThreshold}
                                    onChange={(e) => setVadEnergyThreshold(parseFloat(e.target.value))}
                                    style={{ width: '100%', accentColor: '#06b6d4' }}
                                />
                                <p style={{ fontSize: '0.85rem', color: '#64748b', marginTop: '8px' }}>
                                    Higher values mean you need to speak louder to trigger recording.
                                </p>
                            </div>

                            <div style={{ marginBottom: '20px' }}>
                                <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '8px' }}>
                                    <label style={{ fontWeight: 600 }}>Speech Probability Sensitivity</label>
                                    <span style={{ fontFamily: 'monospace', color: '#60a5fa' }}>{vadSensitivity}%</span>
                                </div>
                                <input
                                    type="range"
                                    min="0"
                                    max="100"
                                    value={vadSensitivity}
                                    onChange={(e) => setVadSensitivity(parseInt(e.target.value))}
                                    style={{ width: '100%', accentColor: '#06b6d4' }}
                                />
                                <p style={{ fontSize: '0.85rem', color: '#64748b', marginTop: '8px' }}>
                                    How confident the AI needs to be that audio is human speech.
                                </p>
                            </div>
                        </div>
                    </div>
                );
            case 'llm':
                return (
                    <div className="llm-settings">
                        <h3 className="settings-section-title">LLM & Grammar</h3>
                        <p style={{ color: '#94a3b8', marginBottom: '24px' }}>
                            Local Large Language Models used for post-processing text corrections.
                        </p>

                        <div className="model-item" style={{ marginBottom: '16px' }}>
                            <div className="model-info">
                                <h3>Gemma 2B (Instruction Tuned)</h3>
                                <div className="model-meta">
                                    <span className="model-tag" style={{ color: '#f472b6', background: 'rgba(236, 72, 153, 0.15)' }}>Active</span>
                                    <span>Google DeepMind</span>
                                </div>
                                <p style={{ margin: '8px 0 0 0', fontSize: '0.9rem', color: '#94a3b8' }}>
                                    Optimized for grammar correction and formatting instruction.
                                </p>
                            </div>
                            <button className="download-btn downloaded" disabled>Built-in Support</button>
                        </div>

                        <div style={{ background: 'rgba(30, 41, 59, 0.4)', padding: '20px', borderRadius: '12px', border: '1px solid rgba(148, 163, 184, 0.1)' }}>
                            <h4 style={{ marginTop: 0, marginBottom: '12px' }}>Prompt Customization</h4>
                            <textarea
                                style={{ width: '100%', background: '#020617', border: '1px solid rgba(148,163,184,0.2)', color: '#cbd5e1', borderRadius: '8px', padding: '12px', minHeight: '100px', fontFamily: 'monospace' }}
                                defaultValue="You are a helpful assistant. Fix the grammar and punctuation of the user's text. Do not change the meaning."
                            />
                            <p style={{ fontSize: '0.85rem', color: '#64748b', marginTop: '8px' }}>
                                System prompt used when "Correct Grammar" is enabled.
                            </p>
                        </div>
                    </div>
                );
        }
    };

    return (
        <div className="settings-overlay" onClick={onClose}>
            <div className="settings-modal" onClick={e => e.stopPropagation()}>
                <div className="settings-header">
                    <h2>Settings</h2>
                    <button className="close-btn" onClick={onClose}>✕</button>
                </div>

                <div className="settings-body">
                    <div className="settings-sidebar">
                        <div
                            className={`settings-nav-item ${activeTab === 'general' ? 'active' : ''}`}
                            onClick={() => setActiveTab('general')}
                        >
                            ⚙️ General
                        </div>
                        <div
                            className={`settings-nav-item ${activeTab === 'downloads' ? 'active' : ''}`}
                            onClick={() => setActiveTab('downloads')}
                        >
                            ⬇ Download Manager
                        </div>
                        <div
                            className={`settings-nav-item ${activeTab === 'audio' ? 'active' : ''}`}
                            onClick={() => setActiveTab('audio')}
                        >
                            🎙️ Audio
                        </div>
                        <div
                            className={`settings-nav-item ${activeTab === 'vad' ? 'active' : ''}`}
                            onClick={() => setActiveTab('vad')}
                        >
                            🌊 VAD
                        </div>
                        <div
                            className={`settings-nav-item ${activeTab === 'llm' ? 'active' : ''}`}
                            onClick={() => setActiveTab('llm')}
                        >
                            🧠 Grammar / LLM
                        </div>
                    </div>

                    <div className="settings-content">
                        {renderContent()}
                    </div>
                </div>
            </div>
        </div>
    );
}
