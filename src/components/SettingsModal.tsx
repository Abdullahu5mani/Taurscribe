import { useState, useEffect } from 'react';
import './SettingsModal.css';
import { toast } from 'sonner';
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ModelsTab } from './settings/ModelsTab';
import { PostProcessingTab } from './settings/PostProcessingTab';
import { AudioTab } from './settings/AudioTab';
import { HotkeyTab } from './settings/HotkeyTab';
import { SoundTab } from './settings/SoundTab';
import { AboutTab } from './settings/AboutTab';
import { MODELS } from './settings/types';
import type { DownloadableModel, DownloadProgress } from './settings/types';

interface SettingsModalProps {
    isOpen: boolean;
    onClose: () => void;
    onModelDownloaded?: () => void;
    enableGrammarLM: boolean;
    setEnableGrammarLM: (val: boolean) => void;
    llmStatus: string;
    enableSpellCheck: boolean;
    setEnableSpellCheck: (val: boolean) => void;
    spellCheckStatus: string;
    enableDenoise: boolean;
    setEnableDenoise: (val: boolean) => void;
    enableOverlay: boolean;
    setEnableOverlay: (val: boolean) => void;
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

type Tab = 'models' | 'post-processing' | 'audio' | 'hotkey' | 'sound' | 'about';

const TABS: { id: Tab; label: string; icon: React.ReactNode }[] = [
    {
        id: 'models',
        label: 'Models',
        icon: (
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                <polyline points="7 10 12 15 17 10" />
                <line x1="12" y1="15" x2="12" y2="3" />
            </svg>
        ),
    },
    {
        id: 'post-processing',
        label: 'Post-Processing',
        icon: (
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M12 20h9" />
                <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z" />
            </svg>
        ),
    },
    {
        id: 'audio',
        label: 'Audio',
        icon: (
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z" />
                <path d="M19 10v2a7 7 0 0 1-14 0v-2" />
                <line x1="12" y1="19" x2="12" y2="23" />
                <line x1="8" y1="23" x2="16" y2="23" />
            </svg>
        ),
    },
    {
        id: 'hotkey',
        label: 'Hotkey',
        icon: (
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <rect x="2" y="4" width="20" height="16" rx="2" />
                <path d="M6 8h.01M10 8h.01M14 8h.01M18 8h.01M8 12h.01M12 12h.01M16 12h.01M7 16h10" />
            </svg>
        ),
    },
    {
        id: 'sound',
        label: 'Sound',
        icon: (
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
                <path d="M19.07 4.93a10 10 0 0 1 0 14.14" />
                <path d="M15.54 8.46a5 5 0 0 1 0 7.07" />
            </svg>
        ),
    },
    {
        id: 'about',
        label: 'About',
        icon: (
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="12" cy="12" r="10" />
                <line x1="12" y1="8" x2="12" y2="12" />
                <line x1="12" y1="16" x2="12.01" y2="16" />
            </svg>
        ),
    },
];

export function SettingsModal({
    isOpen, onClose, onModelDownloaded,
    enableGrammarLM, setEnableGrammarLM, llmStatus,
    enableSpellCheck, setEnableSpellCheck, spellCheckStatus,
    enableDenoise, setEnableDenoise,
    enableOverlay, setEnableOverlay,
    transcriptionStyle, setTranscriptionStyle,
    llmBackend, setLlmBackend,
    soundVolume, soundMuted, setSoundVolume, setSoundMuted,
}: SettingsModalProps) {
    const [activeTab, setActiveTab] = useState<Tab>('models');
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
                        total_files: payload.total_files,
                    },
                }));

                if (payload.status === "done") {
                    toast.success(`Downloaded: ${payload.model_id}`);
                    setModels(prev => prev.map(m =>
                        m.id === payload.model_id ? { ...m, downloaded: true, verified: false } : m
                    ));
                    setDownloadProgress(prev => {
                        const next = { ...prev };
                        delete next[payload.model_id];
                        return next;
                    });
                    onModelDownloaded?.();
                }
            });
        };

        if (isOpen) {
            setupListener();
            invoke<any[]>("get_download_status", { modelIds: models.map(m => m.id) })
                .then(statuses => {
                    setModels(prev => prev.map(m => {
                        const s = statuses.find((x: any) => x.id === m.id);
                        return s ? { ...m, downloaded: s.downloaded, verified: s.verified } : m;
                    }));
                })
                .catch(e => console.error("Failed to fetch model status", e));
        }

        return () => { if (unlisten) unlisten(); };
    }, [isOpen]);

    const handleDownload = async (id: string, name: string) => {
        toast.info(`Starting download: ${name}`);
        setDownloadProgress(prev => ({ ...prev, [id]: { bytes: 0, total: 100, status: 'starting' } }));
        try {
            await invoke("download_model", { modelId: id });
        } catch (e) {
            toast.error(`Download failed: ${e}`);
            setDownloadProgress(prev => { const n = { ...prev }; delete n[id]; return n; });
        }
    };

    const handleDelete = async (id: string, name: string) => {
        if (!confirm(`Delete ${name}?`)) return;
        try {
            await invoke("delete_model", { modelId: id });
            toast.success(`Deleted ${name}`);
            setModels(prev => prev.map(m => m.id === id ? { ...m, downloaded: false } : m));
            setDownloadProgress(prev => { const n = { ...prev }; delete n[id]; return n; });
        } catch (e) {
            toast.error(`Delete failed: ${e}`);
        }
    };

    const handleVerify = async (id: string, name: string) => {
        toast.info(`Verifying ${name}…`);
        setDownloadProgress(prev => ({ ...prev, [id]: { bytes: 0, total: 100, status: 'verifying' } }));
        try {
            await invoke("verify_model_hash", { modelId: id });
            toast.success(`Verified: ${name}`);
            setModels(prev => prev.map(m => m.id === id ? { ...m, verified: true } : m));
        } catch (e) {
            toast.error(`Verification failed: ${e}`);
        } finally {
            setDownloadProgress(prev => { const n = { ...prev }; delete n[id]; return n; });
        }
    };

    const renderContent = () => {
        switch (activeTab) {
            case 'models':
                return (
                    <ModelsTab
                        models={models}
                        downloadProgress={downloadProgress}
                        onDownload={handleDownload}
                        onDelete={handleDelete}
                        onVerify={handleVerify}
                    />
                );
            case 'post-processing':
                return (
                    <PostProcessingTab
                        enableGrammarLM={enableGrammarLM}
                        setEnableGrammarLM={setEnableGrammarLM}
                        llmStatus={llmStatus}
                        llmBackend={llmBackend}
                        setLlmBackend={setLlmBackend}
                        transcriptionStyle={transcriptionStyle}
                        setTranscriptionStyle={setTranscriptionStyle}
                        enableSpellCheck={enableSpellCheck}
                        setEnableSpellCheck={setEnableSpellCheck}
                        spellCheckStatus={spellCheckStatus}
                    />
                );
            case 'audio':
                return <AudioTab enableDenoise={enableDenoise} setEnableDenoise={setEnableDenoise} />;
            case 'hotkey':
                return <HotkeyTab enableOverlay={enableOverlay} setEnableOverlay={setEnableOverlay} />;
            case 'sound':
                return <SoundTab soundVolume={soundVolume} soundMuted={soundMuted} setSoundVolume={setSoundVolume} setSoundMuted={setSoundMuted} />;
            case 'about':
                return <AboutTab />;
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
                            {TABS.map(tab => (
                                <div
                                    key={tab.id}
                                    className={`settings-nav-item ${activeTab === tab.id ? 'active' : ''}`}
                                    onClick={() => setActiveTab(tab.id)}
                                >
                                    {tab.icon}
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
