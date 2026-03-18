import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { IconX } from './Icons';
import './SettingsModal.css';
import { GeneralTab } from './settings/GeneralTab';
import { ModelsTab } from './settings/ModelsTab';
import { PostProcessingTab } from './settings/PostProcessingTab';
import { AudioTab } from './settings/AudioTab';
import { HotkeyTab } from './settings/HotkeyTab';
import { SoundTab } from './settings/SoundTab';
import { AboutTab } from './settings/AboutTab';
import { DictionaryTab } from './settings/DictionaryTab';
import { SnippetsTab } from './settings/SnippetsTab';
import type { DownloadableModel, DownloadProgress } from './settings/types';
import type { DictEntry, SnippetEntry } from '../hooks/usePersonalization';

interface SettingsModalProps {
    isOpen: boolean;
    onClose: () => void;
    initialTab?: Tab;
    onModelDownloaded?: () => void;
    enableGrammarLM: boolean;
    setEnableGrammarLM: (val: boolean) => void;
    llmStatus: string;
    enableDenoise: boolean;
    setEnableDenoise: (val: boolean) => void;
    muteBackgroundAudio: boolean;
    setMuteBackgroundAudio: (val: boolean) => void;
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
    dictionary: DictEntry[];
    addDictEntry: (entry: Omit<DictEntry, "id">) => void;
    updateDictEntry: (id: string, updates: Partial<Omit<DictEntry, "id">>) => void;
    removeDictEntry: (id: string) => void;
    snippets: SnippetEntry[];
    addSnippet: (entry: Omit<SnippetEntry, "id">) => void;
    updateSnippet: (id: string, updates: Partial<Omit<SnippetEntry, "id">>) => void;
    removeSnippet: (id: string) => void;
    settingsModels: DownloadableModel[];
    downloadProgress: Record<string, DownloadProgress>;
    onDownload: (id: string, name: string) => void;
    onDelete: (id: string, name: string) => Promise<void>;
    onCancelDownload: (id: string) => void;
    closeBehavior: 'tray' | 'quit';
    setCloseBehavior: (val: 'tray' | 'quit') => void;
}

type Tab = 'general' | 'models' | 'post-processing' | 'audio' | 'hotkey' | 'sound' | 'dictionary' | 'snippets' | 'about';

const TABS: { id: Tab; label: string; icon: React.ReactNode }[] = [
    {
        id: 'general',
        label: 'General',
        icon: (
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="12" cy="12" r="3" />
                <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
            </svg>
        ),
    },
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
        id: 'dictionary',
        label: 'Dictionary',
        icon: (
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20" />
                <path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z" />
            </svg>
        ),
    },
    {
        id: 'snippets',
        label: 'Snippets',
        icon: (
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <polyline points="13 2 13 9 20 9" />
                <path d="M20 9L13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z" />
                <line x1="8" y1="13" x2="16" y2="13" />
                <line x1="8" y1="17" x2="16" y2="17" />
                <line x1="10" y1="9" x2="8" y2="9" />
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
    isOpen, onClose, initialTab,
    enableGrammarLM, setEnableGrammarLM, llmStatus,
    enableDenoise, setEnableDenoise,
    muteBackgroundAudio, setMuteBackgroundAudio,
    enableOverlay, setEnableOverlay,
    transcriptionStyle, setTranscriptionStyle,
    llmBackend, setLlmBackend,
    soundVolume, soundMuted, setSoundVolume, setSoundMuted,
    dictionary, addDictEntry, updateDictEntry, removeDictEntry,
    snippets, addSnippet, updateSnippet, removeSnippet,
    settingsModels, downloadProgress, onDownload, onDelete, onCancelDownload,
    closeBehavior, setCloseBehavior,
}: SettingsModalProps) {
    const [activeTab, setActiveTab] = useState<Tab>('models');
    const modalRef = useRef<HTMLDivElement>(null);
    const previousFocusRef = useRef<HTMLElement | null>(null);

    // Jump to the requested tab each time the modal is opened
    useEffect(() => {
        if (isOpen) setActiveTab(initialTab ?? 'models');
    }, [isOpen, initialTab]);

    // ── Focus trap + Escape handler ──────────────────────────────
    const handleKeyDown = useCallback((e: KeyboardEvent) => {
        if (e.key === 'Escape') {
            e.stopPropagation();
            onClose();
            return;
        }
        if (e.key !== 'Tab' || !modalRef.current) return;

        const focusable = modalRef.current.querySelectorAll<HTMLElement>(
            'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])'
        );
        if (focusable.length === 0) return;

        const first = focusable[0];
        const last = focusable[focusable.length - 1];

        if (e.shiftKey && document.activeElement === first) {
            e.preventDefault();
            last.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
            e.preventDefault();
            first.focus();
        }
    }, [onClose]);

    useEffect(() => {
        if (!isOpen) return;

        invoke('set_hotkey_suppressed', { suppressed: true }).catch(console.error);

        // Save current focus so we can restore it on close
        previousFocusRef.current = document.activeElement as HTMLElement;

        // Move focus into the modal
        requestAnimationFrame(() => {
            modalRef.current?.querySelector<HTMLElement>('button, [tabindex]')?.focus();
        });

        document.addEventListener('keydown', handleKeyDown);
        return () => {
            document.removeEventListener('keydown', handleKeyDown);
            invoke('set_hotkey_suppressed', { suppressed: false }).catch(console.error);
            // Restore focus when modal closes
            previousFocusRef.current?.focus();
        };
    }, [isOpen, handleKeyDown]);

    const renderContent = () => {
        switch (activeTab) {
            case 'general':
                return <GeneralTab closeBehavior={closeBehavior} setCloseBehavior={setCloseBehavior} />;
            case 'models':
                return (
                    <ModelsTab
                        models={settingsModels}
                        downloadProgress={downloadProgress}
                        onDownload={onDownload}
                        onDelete={onDelete}
                        onCancelDownload={onCancelDownload}
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
                    />
                );
            case 'audio':
                return <AudioTab enableDenoise={enableDenoise} setEnableDenoise={setEnableDenoise} muteBackgroundAudio={muteBackgroundAudio} setMuteBackgroundAudio={setMuteBackgroundAudio} />;
            case 'hotkey':
                return <HotkeyTab enableOverlay={enableOverlay} setEnableOverlay={setEnableOverlay} />;
            case 'sound':
                return <SoundTab soundVolume={soundVolume} soundMuted={soundMuted} setSoundVolume={setSoundVolume} setSoundMuted={setSoundMuted} />;
            case 'dictionary':
                return (
                    <DictionaryTab
                        dictionary={dictionary}
                        addDictEntry={addDictEntry}
                        updateDictEntry={updateDictEntry}
                        removeDictEntry={removeDictEntry}
                    />
                );
            case 'snippets':
                return (
                    <SnippetsTab
                        snippets={snippets}
                        addSnippet={addSnippet}
                        updateSnippet={updateSnippet}
                        removeSnippet={removeSnippet}
                    />
                );
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
                <div
                    className="settings-modal"
                    ref={modalRef}
                    role="dialog"
                    aria-modal="true"
                    aria-labelledby="settings-modal-title"
                    onClick={e => e.stopPropagation()}
                >
                    <div className="settings-header">
                        <h2 id="settings-modal-title">Settings</h2>
                        <button className="close-btn" onClick={onClose} aria-label="Close settings"><IconX size={14} /></button>
                    </div>

                    <div className="settings-body">
                        <nav className="settings-sidebar" aria-label="Settings sections">
                            {TABS.map(tab => (
                                <button
                                    key={tab.id}
                                    className={`settings-nav-item ${activeTab === tab.id ? 'active' : ''}`}
                                    onClick={() => setActiveTab(tab.id)}
                                    aria-current={activeTab === tab.id ? 'page' : undefined}
                                >
                                    {tab.icon}
                                    {tab.label}
                                </button>
                            ))}
                        </nav>

                        <div className="settings-content">
                            {renderContent()}
                        </div>
                    </div>
                </div>
            )}
        </div>
    );
}
