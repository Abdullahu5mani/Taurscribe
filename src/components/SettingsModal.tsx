import { useState, useEffect } from 'react';
import './SettingsModal.css';
import { toast } from 'sonner';
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { GeneralTab } from './settings/GeneralTab';
import { DownloadsTab } from './settings/DownloadsTab';
import { AudioTab } from './settings/AudioTab';
import { MODELS } from './settings/types';
import type { DownloadableModel, DownloadProgress } from './settings/types';

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

    llmBackend: "gpu" | "cpu";
    setLlmBackend: (val: "gpu" | "cpu") => void;

    transcriptionStyle: string;
    setTranscriptionStyle: (val: string) => void;

    soundVolume: number;
    soundMuted: boolean;
    setSoundVolume: (v: number) => void;
    setSoundMuted: (m: boolean) => void;
}

interface DownloadProgressPayload {
    model_id: string;
    total_bytes: number;
    downloaded_bytes: number;
    status: string;
    current_file?: number;
    total_files?: number;
}

type Tab = 'general' | 'downloads' | 'audio' | 'vad' | 'llm';

export function SettingsModal({
    isOpen,
    onClose,
    onModelDownloaded,
    enableGrammarLM,
    setEnableGrammarLM,
    llmStatus,
    enableSpellCheck,
    setEnableSpellCheck,
    spellCheckStatus,
    transcriptionStyle,
    setTranscriptionStyle,
    llmBackend,
    setLlmBackend,
    soundVolume,
    soundMuted,
    setSoundVolume,
    setSoundMuted,
}: SettingsModalProps) {
    const [activeTab, setActiveTab] = useState<Tab>('downloads');
    const [models, setModels] = useState<DownloadableModel[]>(MODELS);
    const [downloadProgress, setDownloadProgress] = useState<Record<string, DownloadProgress>>({});

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
                    // Clear progress so delete/verify buttons become enabled
                    setDownloadProgress(prev => {
                        const newState = { ...prev };
                        delete newState[payload.model_id];
                        return newState;
                    });
                    onModelDownloaded?.();
                }
            });
        };

        if (isOpen) {
            setupListener();

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
                [id]: { bytes: 0, total: 100, status: 'starting' }
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
                    <GeneralTab
                        enableSpellCheck={enableSpellCheck}
                        setEnableSpellCheck={setEnableSpellCheck}
                        spellCheckStatus={spellCheckStatus}
                        soundVolume={soundVolume}
                        soundMuted={soundMuted}
                        setSoundVolume={setSoundVolume}
                        setSoundMuted={setSoundMuted}
                    />
                );
            case 'downloads':
                return (
                    <DownloadsTab
                        models={models}
                        downloadProgress={downloadProgress}
                        onDownload={handleDownload}
                        onDelete={handleDelete}
                        onVerify={handleVerifyHash}
                    />
                );
            case 'audio':
                return <AudioTab />;
            case 'vad':
                return (
                    <div className="vad-settings">
                        <h3 className="settings-section-title">Voice Activity Detection (VAD)</h3>
                        <p style={{ color: '#94a3b8', marginBottom: '24px' }}>
                            Configure how the AI detects when you are speaking versus background noise.
                        </p>
                        <div style={{ background: 'rgba(30, 41, 59, 0.4)', padding: '24px', borderRadius: '12px', border: '1px solid rgba(148, 163, 184, 0.1)' }}>
                            <p style={{ color: '#64748b' }}>VAD configuration coming soon...</p>
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

                        <div className="setting-card" style={{ background: 'rgba(30, 41, 59, 0.4)', padding: '20px', borderRadius: '12px', border: '1px solid rgba(148, 163, 184, 0.1)', marginBottom: '20px' }}>
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
                                Uses local Qwen 2.5 0.5B (GGUF) to format and clean up transcripts.
                            </p>
                            <div style={{ display: 'flex', gap: '10px', alignItems: 'center', marginTop: '12px' }}>
                                <div className="status-badge" style={{ display: 'inline-block', padding: '6px 10px', borderRadius: '6px', background: 'rgba(255,255,255,0.05)', fontSize: '0.8rem', border: '1px solid rgba(255,255,255,0.05)' }}>
                                    Status: <span style={{ fontWeight: 500, color: llmStatus === "Loaded" ? "#22c55e" : (llmStatus === "Loading..." ? "#f59e0b" : "#f43f5e") }}>{llmStatus}</span>
                                </div>

                                <select
                                    value={llmBackend}
                                    onChange={(e) => setLlmBackend(e.target.value as "gpu" | "cpu")}
                                    disabled={llmStatus === "Loading..." || llmStatus === "Loaded"} // Force toggle off to change
                                    title={llmStatus === "Loaded" ? "Turn off LLM to change backend" : "Select compute backend"}
                                    style={{
                                        background: 'rgba(255,255,255,0.05)',
                                        border: '1px solid rgba(255,255,255,0.1)',
                                        color: '#cbd5e1',
                                        borderRadius: '6px',
                                        fontSize: '0.8rem',
                                        padding: '5px 8px',
                                        cursor: (llmStatus === "Loaded" || llmStatus === "Loading...") ? 'not-allowed' : 'pointer',
                                        opacity: (llmStatus === "Loaded" || llmStatus === "Loading...") ? 0.5 : 1,
                                        outline: 'none'
                                    }}
                                >
                                    <option value="gpu">Auto / GPU</option>
                                    <option value="cpu">CPU Only</option>
                                </select>
                            </div>
                        </div>

                        <div className="setting-card" style={{ background: 'rgba(30, 41, 59, 0.4)', padding: '20px', borderRadius: '12px', border: '1px solid rgba(148, 163, 184, 0.1)', marginBottom: '20px' }}>
                            <div className="setting-header" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '12px' }}>
                                <h4 style={{ margin: 0 }}>Transcription Style</h4>
                            </div>
                            <select
                                value={transcriptionStyle}
                                onChange={(e) => setTranscriptionStyle(e.target.value)}
                                disabled={llmStatus !== "Loaded"}
                                style={{
                                    width: '100%',
                                    padding: '10px',
                                    borderRadius: '8px',
                                    border: '1px solid rgba(148,163,184,0.2)',
                                    background: '#0f172a',
                                    color: '#e2e8f0',
                                    cursor: llmStatus !== "Loaded" ? 'not-allowed' : 'pointer',
                                    opacity: llmStatus !== "Loaded" ? 0.5 : 1
                                }}
                            >
                                <option value="Auto">Auto (Default)</option>
                                <option value="Casual">Casual</option>
                                <option value="Verbatim">Verbatim</option>
                                <option value="Enthusiastic">Enthusiastic</option>
                                <option value="Software_Dev">Software Dev</option>
                                <option value="Professional">Professional</option>
                            </select>
                            <p style={{ marginTop: '12px', fontSize: '0.9rem', color: '#94a3b8' }}>
                                Controls the tone and formatting of the corrected text.
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
                            {([
                                { id: 'general', label: '⚙️ General' },
                                { id: 'downloads', label: '⬇ Download Manager' },
                                { id: 'audio', label: '🎙️ Audio' },
                                { id: 'vad', label: '🌊 VAD' },
                                { id: 'llm', label: '🧠 Grammar / LLM' },
                            ] as { id: Tab; label: string }[]).map(tab => (
                                <div
                                    key={tab.id}
                                    className={`settings-nav-item ${activeTab === tab.id ? 'active' : ''}`}
                                    onClick={() => setActiveTab(tab.id)}
                                >
                                    {tab.label}
                                </div>
                            ))}
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
