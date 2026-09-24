import React, { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { IconX } from './Icons';
import { Logo } from './Logo';
import './SettingsModal.css';
import './SettingsSheet.css';
import { ModelsTab } from './settings/ModelsTab';
import { RecordingTab } from './settings/RecordingTab';
import { PostProcessingTab } from './settings/PostProcessingTab';
import { TextTab } from './settings/TextTab';
import { AppTab } from './settings/AppTab';
import { MeetingsTab } from './settings/MeetingsTab';
import { AboutTab } from './settings/AboutTab';
import { StorageTab } from './settings/StorageTab';
import { SPEAKER_MODEL_ID, DIARIZATION_MODEL_ID, type DownloadableModel, type DownloadProgress } from './settings/types';
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
    customVocabulary?: string[];
    contextBiasEnabled?: boolean;
    addVocabWord?: (word: string) => void;
    removeVocabWord?: (word: string) => void;
    addVocabPreset?: (category: "developer" | "medical" | "legal") => void;
    clearVocab?: () => void;
    setContextBiasEnabled?: (enabled: boolean) => void;
    settingsModels: DownloadableModel[];
    downloadProgress: Record<string, DownloadProgress>;
    onDownload: (id: string, name: string) => void;
    onDelete: (id: string, name: string) => Promise<void>;
    onCancelDownload: (id: string) => void;
    scrollTarget?: string;
    onScrollHandled?: () => void;
    closeBehavior: 'tray' | 'quit';
    setCloseBehavior: (val: 'tray' | 'quit') => void;
}

type Tab = 'models' | 'storage' | 'recording' | 'meetings' | 'grammar' | 'text' | 'app' | 'about';

/** Sidebar order and wording. Tab ids stay stable (deep links, tests). */
const TABS: { id: Tab; label: string; blurb: string; icon: React.ReactNode }[] = [
    { id: 'app', label: 'General', blurb: 'Startup, window, sounds and access for AI apps.',
      icon: <><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 0 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 0 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 0 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 0 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" /></> },
    { id: 'recording', label: 'Recording', blurb: 'Hotkey, microphone and the recording overlay.',
      icon: <><rect x="9" y="3" width="6" height="11" rx="3" /><path d="M5 11a7 7 0 0 0 14 0M12 18v3" /></> },
    { id: 'meetings', label: 'Meetings', blurb: 'Call detection, recording and speakers.',
      icon: <><rect x="3" y="6" width="13" height="12" rx="2" /><path d="M16 10.5l5-3v9l-5-3" /></> },
    { id: 'models', label: 'Models', blurb: 'Download and manage speech and speaker models.',
      icon: <><path d="M12 3l8 4.5v9L12 21l-8-4.5v-9z" /><path d="M12 12l8-4.5M12 12v9M12 12L4 7.5" /></> },
    { id: 'storage', label: 'Storage', blurb: 'Where models and recordings are kept, and how fast that drive is.',
      icon: <><ellipse cx="12" cy="6" rx="8" ry="3" /><path d="M4 6v6c0 1.7 3.6 3 8 3s8-1.3 8-3V6" /><path d="M4 12v6c0 1.7 3.6 3 8 3s8-1.3 8-3v-6" /></> },
    { id: 'grammar', label: 'Writing', blurb: 'Grammar clean-up and transcription style.',
      icon: <><path d="M4 20h4L19 9a2.8 2.8 0 0 0-4-4L4 16z" /><path d="M13.5 6.5l4 4" /></> },
    { id: 'text', label: 'Dictionary', blurb: 'Replacements, snippets and custom words.',
      icon: <><path d="M5 4h11a3 3 0 0 1 3 3v13H8a3 3 0 0 1-3-3z" /><path d="M5 17a3 3 0 0 1 3-3h11" /></> },
    { id: 'about', label: 'About', blurb: 'Version, hardware and app data.',
      icon: <><circle cx="12" cy="12" r="9" /><path d="M12 11v5M12 8h.01" /></> },
];

/** Matches the close animation in SettingsSheet.css. */
const CLOSE_MS = 180;

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
    customVocabulary, contextBiasEnabled,
    addVocabWord, removeVocabWord, addVocabPreset, clearVocab, setContextBiasEnabled,
    settingsModels, downloadProgress, onDownload, onDelete, onCancelDownload,
    scrollTarget, onScrollHandled,
    closeBehavior, setCloseBehavior,
}: SettingsModalProps) {
    const [activeTab, setActiveTab] = useState<Tab>('models');
    // Stay rendered for the close animation after isOpen turns false.
    const [mounted, setMounted] = useState(isOpen);
    useEffect(() => {
        if (isOpen) { setMounted(true); return; }
        const t = setTimeout(() => setMounted(false), CLOSE_MS);
        return () => clearTimeout(t);
    }, [isOpen]);
    const modalRef = useRef<HTMLDivElement>(null);
    const previousFocusRef = useRef<HTMLElement | null>(null);

    // Jump to the requested tab each time the modal is opened. Done during
    // render (not in an effect) so the first frame already shows that tab.
    // Page transitions only play for tab switches, not on top of the sheet's
    // own entrance.
    const [tabSwitched, setTabSwitched] = useState(false);
    const openKey = isOpen ? `open:${initialTab ?? 'models'}` : 'closed';
    const [prevOpenKey, setPrevOpenKey] = useState(openKey);
    if (openKey !== prevOpenKey) {
        setPrevOpenKey(openKey);
        if (isOpen) {
            setActiveTab(initialTab ?? 'models');
            if (!prevOpenKey.startsWith('open')) setTabSwitched(false);
        }
    }
    const switchTab = (tab: Tab) => {
        setTabSwitched(true);
        setActiveTab(tab);
    };

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
            case 'models':
                return (
                    <ModelsTab
                        models={settingsModels}
                        downloadProgress={downloadProgress}
                        onDownload={onDownload}
                        onDelete={onDelete}
                        onCancelDownload={onCancelDownload}
                        scrollTarget={scrollTarget}
                        onScrollHandled={onScrollHandled}
                    />
                );
            case 'recording':
                return (
                    <RecordingTab
                        enableOverlay={enableOverlay}
                        setEnableOverlay={setEnableOverlay}
                        enableDenoise={enableDenoise}
                        setEnableDenoise={setEnableDenoise}
                        muteBackgroundAudio={muteBackgroundAudio}
                        setMuteBackgroundAudio={setMuteBackgroundAudio}
                    />
                );
            case 'meetings':
                return (
                    <MeetingsTab
                        speakerModelDownloaded={settingsModels.some(m => m.id === SPEAKER_MODEL_ID && m.downloaded)}
                        diarizationModelDownloaded={settingsModels.some(m => m.id === DIARIZATION_MODEL_ID && m.downloaded)}
                        onOpenModels={() => switchTab('models')}
                    />
                );
            case 'grammar':
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
            case 'text':
                return (
                    <TextTab
                        dictionary={dictionary}
                        addDictEntry={addDictEntry}
                        updateDictEntry={updateDictEntry}
                        removeDictEntry={removeDictEntry}
                        snippets={snippets}
                        addSnippet={addSnippet}
                        updateSnippet={updateSnippet}
                        removeSnippet={removeSnippet}
                        customVocabulary={customVocabulary}
                        contextBiasEnabled={contextBiasEnabled}
                        addVocabWord={addVocabWord}
                        removeVocabWord={removeVocabWord}
                        addVocabPreset={addVocabPreset}
                        clearVocab={clearVocab}
                        setContextBiasEnabled={setContextBiasEnabled}
                    />
                );
            case 'app':
                return (
                    <AppTab
                        closeBehavior={closeBehavior}
                        setCloseBehavior={setCloseBehavior}
                        soundVolume={soundVolume}
                        soundMuted={soundMuted}
                        setSoundVolume={setSoundVolume}
                        setSoundMuted={setSoundMuted}
                    />
                );
            case 'storage':
                return <StorageTab />;
            case 'about':
                return <AboutTab />;
        }
    };

    const current = TABS.find(t => t.id === activeTab) ?? TABS[0];

    return (
        <div
            id="settings-modal-overlay"
            data-testid="settings-modal-overlay"
            className={`settings-overlay ${isOpen ? 'settings-overlay--open' : 'settings-overlay--closed'}${mounted && !isOpen ? ' settings-overlay--closing' : ''}`}
            onClick={isOpen ? onClose : undefined}
            aria-hidden={!isOpen}
        >
            {(isOpen || mounted) && (
                <div
                    id="settings-modal"
                    data-testid="settings-modal"
                    className={`settings-modal settings-sheet${tabSwitched ? ' settings-sheet--switched' : ''}`}
                    ref={modalRef}
                    role="dialog"
                    aria-modal="true"
                    aria-labelledby="settings-modal-title"
                    onClick={e => e.stopPropagation()}
                >
                    <aside className="settings-side">
                        <div className="settings-side-brand">
                            <Logo size={18} variant="small" />
                            <span>Settings</span>
                        </div>
                        <nav
                            id="settings-tablist"
                            data-testid="settings-tablist"
                            className="settings-nav"
                            role="tablist"
                            aria-label="Settings sections"
                        >
                            {TABS.map(tab => (
                                <button
                                    type="button"
                                    key={tab.id}
                                    id={`settings-tab-${tab.id}`}
                                    data-testid={`settings-tab-${tab.id}`}
                                    role="tab"
                                    aria-label={tab.label}
                                    aria-selected={activeTab === tab.id}
                                    aria-controls={`settings-tabpanel-${tab.id}`}
                                    className={`settings-nav-btn${activeTab === tab.id ? ' active' : ''}`}
                                    onClick={() => switchTab(tab.id)}
                                >
                                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                                        {tab.icon}
                                    </svg>
                                    <span>{tab.label}</span>
                                </button>
                            ))}
                        </nav>
                        <p className="settings-side-note" aria-live="polite">
                            <span className="settings-side-note-dot" />
                            Hotkey paused while Settings is open
                        </p>
                    </aside>

                    <div className="settings-main">
                        <header className="settings-page-header">
                            <div key={activeTab} className="settings-page-heading">
                                <h2 id="settings-modal-title">{current.label}</h2>
                                <p>{current.blurb}</p>
                            </div>
                            <button
                                type="button"
                                id="settings-close-btn"
                                data-testid="settings-close-btn"
                                className="settings-close"
                                onClick={onClose}
                                aria-label="Close settings"
                                title="Close (Esc)"
                            >
                                <IconX size={14} />
                            </button>
                        </header>

                        <div
                            className="settings-content"
                            key={activeTab}
                            id={`settings-tabpanel-${activeTab}`}
                            data-testid={`settings-tabpanel-${activeTab}`}
                            role="tabpanel"
                            aria-labelledby={`settings-tab-${activeTab}`}
                        >
                            <div className="settings-content-inner">
                                {renderContent()}
                            </div>
                        </div>
                    </div>
                </div>
            )}
        </div>
    );
}
