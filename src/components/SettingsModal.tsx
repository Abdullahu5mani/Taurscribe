import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { IconX } from './Icons';
import './SettingsModal.css';
import { ModelsTab } from './settings/ModelsTab';
import { RecordingTab } from './settings/RecordingTab';
import { PostProcessingTab } from './settings/PostProcessingTab';
import { TextTab } from './settings/TextTab';
import { AppTab } from './settings/AppTab';
import { AboutTab } from './settings/AboutTab';
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
    scrollTarget?: string;
    onScrollHandled?: () => void;
    closeBehavior: 'tray' | 'quit';
    setCloseBehavior: (val: 'tray' | 'quit') => void;
    overlayStyle: 'minimal' | 'full';
    setOverlayStyle: (val: 'minimal' | 'full') => void;
}

type Tab = 'models' | 'recording' | 'grammar' | 'text' | 'app' | 'about';

const TABS: { id: Tab; label: string }[] = [
    { id: 'models',    label: 'Models'    },
    { id: 'recording', label: 'Recording' },
    { id: 'grammar',   label: 'Grammar'   },
    { id: 'text',      label: 'Text'      },
    { id: 'app',       label: 'App'       },
    { id: 'about',     label: 'About'     },
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
    scrollTarget, onScrollHandled,
    closeBehavior, setCloseBehavior,
    overlayStyle, setOverlayStyle,
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
                        overlayStyle={overlayStyle}
                        setOverlayStyle={setOverlayStyle}
                        enableDenoise={enableDenoise}
                        setEnableDenoise={setEnableDenoise}
                        muteBackgroundAudio={muteBackgroundAudio}
                        setMuteBackgroundAudio={setMuteBackgroundAudio}
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
                <div className="settings-hotkey-warning" aria-live="polite">
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                        <circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/>
                    </svg>
                    Hotkey disabled while settings is open
                </div>
            )}
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
                        <nav className="settings-tabbar" aria-label="Settings sections">
                            {TABS.map(tab => (
                                <button
                                    key={tab.id}
                                    className={`settings-tab-btn ${activeTab === tab.id ? 'active' : ''}`}
                                    onClick={() => setActiveTab(tab.id)}
                                    aria-current={activeTab === tab.id ? 'page' : undefined}
                                >
                                    {tab.label}
                                </button>
                            ))}
                        </nav>

                        <div className="settings-content" key={activeTab}>
                            {renderContent()}
                        </div>
                    </div>
                </div>
            )}
        </div>
    );
}
