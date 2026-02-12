import { useState } from 'react';
import './SettingsModal.css';
import { toast } from 'sonner';

interface SettingsModalProps {
    isOpen: boolean;
    onClose: () => void;
    onModelDownloaded?: () => void;

    // Feature Toggles
    enableGrammarLM: boolean;
    setEnableGrammarLM: (val: boolean) => void;
    llmStatus: string;

    enableSpellCheck: boolean;
    setEnableSpellCheck: (val: boolean) => void;
    spellCheckStatus: string;
}

type Tab = 'general' | 'downloads' | 'audio' | 'vad' | 'llm';

interface DownloadableModel {
    id: string;
    name: string;
    type: 'Whisper' | 'Parakeet' | 'LLM' | 'Utility';
    size: string;
    description: string;
    downloaded: boolean;
    verified?: boolean;
}

const MOCK_MODELS: DownloadableModel[] = [
    // --- Tiny ---
    { id: 'whisper-tiny', name: 'Tiny (Multilingual)', type: 'Whisper', size: '75 MB', description: 'Fastest, lowest accuracy. 99+ languages.', downloaded: false },
    { id: 'whisper-tiny-q5_1', name: 'Tiny (Multi, Q5_1)', type: 'Whisper', size: '31 MB', description: 'Quantized Tiny. 99+ languages.', downloaded: false },
    { id: 'whisper-tiny-en', name: 'Tiny (English)', type: 'Whisper', size: '75 MB', description: 'Fastest model. English only.', downloaded: false },
    { id: 'whisper-tiny-en-q5_1', name: 'Tiny (English, Q5_1)', type: 'Whisper', size: '31 MB', description: 'Quantized, ultra-fast. English only.', downloaded: false },

    // --- Base ---
    { id: 'whisper-base', name: 'Base (Multilingual)', type: 'Whisper', size: '142 MB', description: 'Balanced entry model. 99+ languages.', downloaded: false },
    { id: 'whisper-base-en', name: 'Base (English)', type: 'Whisper', size: '142 MB', description: 'Standard balanced model. English only.', downloaded: false },
    { id: 'whisper-base-q5_1', name: 'Base (Multi, Q5_1)', type: 'Whisper', size: '57 MB', description: 'Quantized Base. 99+ languages.', downloaded: false },
    { id: 'whisper-base-en-q5_1', name: 'Base (English, Q5_1)', type: 'Whisper', size: '57 MB', description: 'Quantized Base. English only.', downloaded: false },

    // --- Small ---
    { id: 'whisper-small', name: 'Small (Multilingual)', type: 'Whisper', size: '466 MB', description: 'Good accuracy for general use. 99+ languages.', downloaded: false },
    { id: 'whisper-small-en', name: 'Small (English)', type: 'Whisper', size: '466 MB', description: 'Good accuracy model. English only.', downloaded: false },
    { id: 'whisper-small-q5_1', name: 'Small (Multi, Q5_1)', type: 'Whisper', size: '181 MB', description: 'Quantized Small. 99+ languages.', downloaded: false },
    { id: 'whisper-small-en-q5_1', name: 'Small (English, Q5_1)', type: 'Whisper', size: '181 MB', description: 'Quantized Small. English only.', downloaded: false },

    // --- Medium ---
    { id: 'whisper-medium', name: 'Medium (Multilingual)', type: 'Whisper', size: '1.5 GB', description: 'High accuracy, slower. 99+ languages.', downloaded: false },
    { id: 'whisper-medium-en', name: 'Medium (English)', type: 'Whisper', size: '1.5 GB', description: 'High accuracy. English only.', downloaded: false },
    { id: 'whisper-medium-q5_0', name: 'Medium (Multi, Q5_0)', type: 'Whisper', size: '514 MB', description: 'Quantized Medium. 99+ languages.', downloaded: false },
    { id: 'whisper-medium-en-q5_0', name: 'Medium (English, Q5_0)', type: 'Whisper', size: '514 MB', description: 'Quantized Medium. English only.', downloaded: false },

    // --- Large ---
    { id: 'whisper-large-v3', name: 'Large V3 (Multilingual)', type: 'Whisper', size: '2.9 GB', description: 'State of the art accuracy. 99+ languages.', downloaded: false },
    { id: 'whisper-large-v3-q5_0', name: 'Large V3 (Multi, Q5_0)', type: 'Whisper', size: '1.1 GB', description: 'Quantized Large V3. 99+ languages.', downloaded: false },
    { id: 'whisper-large-v3-turbo', name: 'Large V3 Turbo', type: 'Whisper', size: '1.5 GB', description: 'Optimized Large V3. 99+ languages.', downloaded: false },
    { id: 'whisper-large-v3-turbo-q5_0', name: 'Large V3 Turbo (Q5_0)', type: 'Whisper', size: '547 MB', description: 'Quantized Turbo. 99+ languages.', downloaded: false },

    // --- Parakeet ---
    { id: 'parakeet-nemotron', name: 'Nemotron Streaming', type: 'Parakeet', size: '1.2 GB', description: 'Ultra-low latency streaming. English only.', downloaded: true },

    // --- LLM ---
    { id: 'qwen2.5-0.5b-safetensors', name: 'Qwen 2.5 0.5B (GPU)', type: 'LLM', size: '~1 GB', description: 'Safetensors model for CUDA/CPU. Best for grammar correction.', downloaded: false },
    { id: 'qwen2.5-0.5b-instruct', name: 'Qwen 2.5 0.5B (Instruct, GGUF)', type: 'LLM', size: '429 MB', description: 'Quantized Q4_K_M. Use if you prefer smaller size.', downloaded: false },
    { id: 'qwen2.5-0.5b-instruct-tokenizer', name: 'Qwen 2.5 Tokenizer Files', type: 'LLM', size: '11.5 MB', description: 'Required for GGUF Instruct model only.', downloaded: false },

    // --- Utility ---
    { id: 'symspell-en-82k', name: 'English Dictionary (SymSpell)', type: 'Utility', size: '1 MB', description: 'Fast spelling correction (82k words). English only.', downloaded: false },
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
    current_file?: number;
    total_files?: number;
}

// ... (MOCK_MODELS definition)

export function SettingsModal({
    isOpen,
    onClose,
    onModelDownloaded,
    enableGrammarLM,
    setEnableGrammarLM,
    llmStatus,
    enableSpellCheck,
    setEnableSpellCheck,
    spellCheckStatus
}: SettingsModalProps) {
    const [activeTab, setActiveTab] = useState<Tab>('downloads');
    const [models, setModels] = useState(MOCK_MODELS);
    const [downloadProgress, setDownloadProgress] = useState<Record<string, { bytes: number, total: number, status: string, current_file?: number, total_files?: number }>>({});
    const [isWhisperExpanded, setIsWhisperExpanded] = useState(false);

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
                        status: payload.status,
                        current_file: payload.current_file,
                        total_files: payload.total_files
                    }
                }));

                if (payload.status === "done") {
                    toast.success(`Download complete: ${payload.model_id}`);
                    setModels(prev => prev.map(m =>
                        m.id === payload.model_id ? { ...m, downloaded: true, verified: false } : m
                    ));
                    // Notify parent to refresh model lists
                    onModelDownloaded?.();
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
            case 'general':
                return (
                    <div className="general-settings">
                        <h3 className="settings-section-title">General Settings</h3>

                        <div className="setting-card" style={{ background: 'rgba(30, 41, 59, 0.4)', padding: '20px', borderRadius: '12px', border: '1px solid rgba(148, 163, 184, 0.1)' }}>
                            <div className="setting-header" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '12px' }}>
                                <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                                    <div className="status-dot" style={{
                                        backgroundColor: !enableGrammarLM ? "#ef4444" : (llmStatus === "Loading..." ? "#f59e0b" : (llmStatus === "Loaded" ? "#22c55e" : "#ef4444"))
                                    }} />
                                    <h4 style={{ margin: 0 }}>Grammar Correction (LLM)</h4>
                                </div>
                                <label className={`switch ${llmStatus === "Loading..." ? "switch--disabled" : ""}`} title={llmStatus === "Loading..." ? "Loading… please wait" : undefined}>
                                    <input
                                        type="checkbox"
                                        checked={enableGrammarLM}
                                        onChange={(e) => setEnableGrammarLM(e.target.checked)}
                                        disabled={llmStatus === "Loading..."}
                                    />
                                    <span className="slider round"></span>
                                </label>
                            </div>
                            <p style={{ margin: 0, fontSize: '0.9rem', color: '#94a3b8' }}>
                                Uses local Qwen 2.5 0.5B (GPU safetensors or GGUF) to format and clean up transcripts.
                            </p>
                            <div className="status-badge" style={{ marginTop: '12px', display: 'inline-block', padding: '4px 8px', borderRadius: '4px', background: 'rgba(255,255,255,0.05)', fontSize: '0.8rem' }}>
                                Status: <span style={{ color: llmStatus === "Loaded" ? "#22c55e" : (llmStatus === "Loading..." ? "#f59e0b" : "#f43f5e") }}>{llmStatus}</span>
                            </div>
                        </div>

                        <div className="setting-card" style={{ marginTop: '16px', background: 'rgba(30, 41, 59, 0.4)', padding: '20px', borderRadius: '12px', border: '1px solid rgba(148, 163, 184, 0.1)' }}>
                            <div className="setting-header" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '12px' }}>
                                <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                                    <div className="status-dot" style={{
                                        backgroundColor: !enableSpellCheck ? "#ef4444" : (spellCheckStatus === "Loading..." ? "#f59e0b" : (spellCheckStatus === "Loaded" ? "#22c55e" : "#ef4444"))
                                    }} />
                                    <h4 style={{ margin: 0 }}>Spell Check (SymSpell)</h4>
                                </div>
                                <label className={`switch ${spellCheckStatus === "Loading..." ? "switch--disabled" : ""}`} title={spellCheckStatus === "Loading..." ? "Loading… please wait" : undefined}>
                                    <input
                                        type="checkbox"
                                        checked={enableSpellCheck}
                                        onChange={(e) => setEnableSpellCheck(e.target.checked)}
                                        disabled={spellCheckStatus === "Loading..."}
                                    />
                                    <span className="slider round"></span>
                                </label>
                            </div>
                            <p style={{ margin: 0, fontSize: '0.9rem', color: '#94a3b8' }}>
                                Fast dictionary-based spelling correction.
                            </p>
                        </div>
                    </div>
                );
            case 'downloads':
                const priorityWhisperIds = [
                    'whisper-tiny-en-q5_1',
                    'whisper-base-en-q5_1',
                    'whisper-small-en-q5_1',
                    'whisper-large-v3-turbo-q5_0'
                ];

                const whisperModels = models.filter(m => m.type === 'Whisper');
                const parakeetModels = models.filter(m => m.type === 'Parakeet');
                const llmModels = models.filter(m => m.type === 'LLM');
                const utilityModels = models.filter(m => m.type === 'Utility');

                const visibleWhisper = whisperModels.filter(m => priorityWhisperIds.includes(m.id));
                const hiddenWhisper = whisperModels.filter(m => !priorityWhisperIds.includes(m.id));

                // Sort visible whisper by size (Tiny -> Large)
                const sizeOrder = ['Tiny', 'Base', 'Small', 'Large'];
                visibleWhisper.sort((a, b) => {
                    const aIndex = sizeOrder.findIndex(s => a.name.includes(s));
                    const bIndex = sizeOrder.findIndex(s => b.name.includes(s));
                    return aIndex - bIndex;
                });

                const renderModelRow = (model: DownloadableModel) => (
                    <div key={model.id} className="model-item">
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
                            {downloadProgress[model.id] && !model.downloaded ? (
                                <div style={{ width: '100%' }}>
                                    <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: '0.75rem', marginBottom: '4px', color: '#94a3b8' }}>
                                        <span>
                                            {downloadProgress[model.id].status === 'verifying' ? 'Verifying...' :
                                                ((downloadProgress[model.id].total_files || 0) > 1 ?
                                                    `Downloading (${downloadProgress[model.id].current_file || 1}/${downloadProgress[model.id].total_files || 1})...` :
                                                    'Downloading...')}
                                        </span>
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
                                                disabled={!!downloadProgress[model.id]}
                                                title={downloadProgress[model.id] ? "Please wait…" : "Delete Model"}
                                                style={{
                                                    background: 'rgba(239, 68, 68, 0.1)',
                                                    color: '#ef4444',
                                                    border: '1px solid rgba(239, 68, 68, 0.2)',
                                                    padding: '8px 12px',
                                                    borderRadius: '6px',
                                                    cursor: downloadProgress[model.id] ? 'not-allowed' : 'pointer',
                                                    fontSize: '1rem',
                                                    transition: 'all 0.2s',
                                                    opacity: downloadProgress[model.id] ? 0.6 : 1
                                                }}
                                            >
                                                🗑️
                                            </button>

                                            {!model.verified && (
                                                <button
                                                    onClick={() => handleVerifyHash(model.id, model.name)}
                                                    disabled={!!downloadProgress[model.id]}
                                                    title={downloadProgress[model.id] ? "Please wait…" : "Deep Verify Hash (SHA1)"}
                                                    style={{
                                                        background: 'rgba(148, 163, 184, 0.1)',
                                                        color: '#94a3b8',
                                                        border: '1px solid rgba(148, 163, 184, 0.2)',
                                                        padding: '8px 12px',
                                                        borderRadius: '6px',
                                                        cursor: downloadProgress[model.id] ? 'not-allowed' : 'pointer',
                                                        fontSize: '1rem',
                                                        transition: 'all 0.2s',
                                                        opacity: downloadProgress[model.id] ? 0.6 : 1
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
                );

                return (
                    <div className="download-manager">
                        <h3 className="settings-section-title">Model Library</h3>
                        <div style={{ marginBottom: '16px', fontSize: '0.9rem', color: '#94a3b8', background: 'rgba(59, 130, 246, 0.1)', padding: '12px', borderRadius: '8px', border: '1px solid rgba(59, 130, 246, 0.2)' }}>
                            📂 <strong>Storage Location:</strong> <code style={{ fontFamily: 'monospace', color: '#e2e8f0' }}>%AppData%\Taurscribe\models</code>
                        </div>

                        <div className="model-list">
                            {/* Parakeet (Real-time) */}
                            {parakeetModels.length > 0 && (
                                <div style={{ marginBottom: '24px' }}>
                                    <h4 style={{ color: '#fff', borderBottom: '1px solid #334155', paddingBottom: '8px', marginBottom: '12px', display: 'flex', alignItems: 'center', gap: '8px' }}>
                                        🦜 Real-Time Streaming
                                    </h4>
                                    {parakeetModels.map(renderModelRow)}
                                </div>
                            )}

                            {/* LLM */}
                            {llmModels.length > 0 && (
                                <div style={{ marginBottom: '24px' }}>
                                    <h4 style={{ color: '#fff', borderBottom: '1px solid #334155', paddingBottom: '8px', marginBottom: '12px', display: 'flex', alignItems: 'center', gap: '8px' }}>
                                        🧠 AI Assistants
                                    </h4>
                                    {llmModels.map(renderModelRow)}
                                </div>
                            )}

                            {/* Whisper (Standard) */}
                            <div style={{ marginBottom: '24px' }}>
                                <h4 style={{ color: '#fff', borderBottom: '1px solid #334155', paddingBottom: '8px', marginBottom: '12px', display: 'flex', alignItems: 'center', gap: '8px' }}>
                                    📝 Speech Recognition (Whisper)
                                </h4>
                                {visibleWhisper.map(renderModelRow)}

                                <div style={{ marginTop: '12px', textAlign: 'center' }}>
                                    <button
                                        onClick={() => setIsWhisperExpanded(!isWhisperExpanded)}
                                        style={{
                                            background: 'transparent',
                                            border: '1px solid #475569',
                                            color: '#94a3b8',
                                            padding: '8px 16px',
                                            borderRadius: '6px',
                                            cursor: 'pointer',
                                            fontSize: '0.85rem'
                                        }}
                                    >
                                        {isWhisperExpanded ? 'Show Less Models' : `Show ${hiddenWhisper.length} More Models...`}
                                    </button>
                                </div>

                                {isWhisperExpanded && (
                                    <div style={{ marginTop: '12px', paddingLeft: '12px', borderLeft: '2px solid #334155' }}>
                                        {hiddenWhisper.map(renderModelRow)}
                                    </div>
                                )}
                            </div>

                            {/* Utility */}
                            {utilityModels.length > 0 && (
                                <div style={{ marginBottom: '24px' }}>
                                    <h4 style={{ color: '#fff', borderBottom: '1px solid #334155', paddingBottom: '8px', marginBottom: '12px', display: 'flex', alignItems: 'center', gap: '8px' }}>
                                        🛠️ Utilities
                                    </h4>
                                    {utilityModels.map(renderModelRow)}
                                </div>
                            )}
                        </div>
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
                                <h3>Qwen 2.5 0.5B (GPU / Safetensors)</h3>
                                <div className="model-meta">
                                    <span className="model-tag" style={{ color: '#f472b6', background: 'rgba(236, 72, 153, 0.15)' }}>Recommended</span>
                                    <span>Hugging Face · Qwen/Qwen2.5-0.5B</span>
                                </div>
                                <p style={{ margin: '8px 0 0 0', fontSize: '0.9rem', color: '#94a3b8' }}>
                                    Download in <strong>Download Manager</strong> → &quot;Qwen 2.5 0.5B (GPU)&quot;. Uses CUDA when available.
                                </p>
                            </div>
                            <button className="download-btn downloaded" disabled>Download in Download Manager</button>
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
        <div
            className={`settings-overlay ${isOpen ? 'settings-overlay--open' : 'settings-overlay--closed'}`}
            onClick={isOpen ? onClose : undefined}
            aria-hidden={!isOpen}
        >
            {isOpen && (
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
            )}
        </div>
    );
}
