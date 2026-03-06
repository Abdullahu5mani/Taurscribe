import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { IconBolt, IconCpu } from "../Icons";

interface PostProcessingTabProps {
    enableGrammarLM: boolean;
    setEnableGrammarLM: (val: boolean) => void;
    llmStatus: string;
    llmBackend: 'gpu' | 'cpu';
    setLlmBackend: (val: 'gpu' | 'cpu') => void;
    transcriptionStyle: string;
    setTranscriptionStyle: (val: string) => void;
}

const STYLES = [
    { value: 'Verbatim', label: 'Verbatim', desc: 'Minimal changes, preserve speech' },
    { value: 'Casual', label: 'Casual', desc: 'Relaxed, conversational tone' },
    { value: 'Enthusiastic', label: 'Enthusiastic', desc: 'Energetic and expressive' },
    { value: 'Software_Dev', label: 'Software Dev', desc: 'Technical language, code terms' },
    { value: 'Professional', label: 'Professional', desc: 'Formal and polished' },
];

function statusColor(status: string, enabled: boolean): string {
    if (!enabled) return '#4b4b55';
    if (status === 'Loaded') return '#3ecfa5';
    if (status === 'Loading...') return '#e09f3e';
    return '#ef4444';
}

export function PostProcessingTab({
    enableGrammarLM, setEnableGrammarLM, llmStatus, llmBackend, setLlmBackend,
    transcriptionStyle, setTranscriptionStyle,
}: PostProcessingTabProps) {
    const [platform, setPlatform] = useState("");

    useEffect(() => {
        invoke<string>("get_platform").then(setPlatform).catch(() => { });
    }, []);

    const isMac = platform === "macos";

    const llmLoading = llmStatus === 'Loading...';
    const llmLoaded = llmStatus === 'Loaded';
    const llmNotDownloaded = llmStatus === 'Not Downloaded';
    const llmBackendLocked = enableGrammarLM; // can't change backend while model is loaded/loading

    return (
        <div className="pp-tab">

            {/* ── Grammar Correction ──────────────────────────────── */}
            <h3 className="settings-section-title">Grammar Correction</h3>

            <div className="setting-card">
                <div className="setting-card-header">
                    <div className="setting-card-label">
                        <span className="status-dot" style={{ background: statusColor(llmStatus, enableGrammarLM) }} />
                        <span>Grammar LLM</span>
                        <span className="setting-card-meta">FlowScribe Qwen 2.5 0.5B · GGUF</span>
                    </div>
                    <label className={`switch ${llmLoading || llmNotDownloaded ? 'switch--disabled' : ''}`}>
                        <input
                            type="checkbox"
                            checked={enableGrammarLM}
                            onChange={e => setEnableGrammarLM(e.target.checked)}
                            disabled={llmLoading || llmNotDownloaded}
                        />
                        <span className="slider round" />
                    </label>
                </div>
                <p className="setting-card-desc">
                    Runs a local LLM after each recording to fix grammar, punctuation, and formatting.
                </p>

                {llmNotDownloaded && (
                    <p className="setting-card-desc" style={{ color: '#ef4444', marginTop: '8px' }}>
                        Model not downloaded. Download FlowScribe Qwen from the <strong>Models</strong> tab.
                    </p>
                )}

                <div className="setting-row">
                    <span className="setting-row-label">Status</span>
                    <span className="status-badge" style={{ color: statusColor(llmStatus, true) }}>{llmStatus}</span>
                </div>

                {!isMac && (
                    <div className="setting-row">
                        <span className="setting-row-label">
                            Backend
                            {llmBackendLocked && (
                                <span style={{ display: 'block', fontSize: '0.72rem', color: '#4b4b55', marginTop: '2px' }}>
                                    disable LLM to change
                                </span>
                            )}
                        </span>
                        <div className={`backend-toggle ${llmBackendLocked ? 'backend-toggle--locked' : ''}`}>
                            <button
                                className={`backend-toggle-btn ${llmBackend === 'gpu' ? 'active' : ''}`}
                                onClick={() => setLlmBackend('gpu')}
                                disabled={llmBackendLocked}
                            >
                                <IconBolt size={12} style={{ color: '#facc15' }} /> GPU
                                <span className="qs-backend-tag">NVIDIA/AMD</span>
                            </button>
                            <button
                                className={`backend-toggle-btn ${llmBackend === 'cpu' ? 'active' : ''}`}
                                onClick={() => setLlmBackend('cpu')}
                                disabled={llmBackendLocked}
                            >
                                <IconCpu size={12} /> CPU
                                <span className="qs-backend-tag">Processor</span>
                            </button>
                        </div>
                    </div>
                )}
            </div>

            {/* ── Transcription Style ─────────────────────────────── */}
            <div className="setting-card" style={{ marginTop: '12px' }}>
                <div className="setting-card-header">
                    <span className="setting-card-label-plain">Transcription Style</span>
                    {!llmLoaded && (
                        <span className="setting-card-meta">requires Grammar LLM</span>
                    )}
                </div>
                <p className="setting-card-desc">
                    Controls the tone the LLM applies when cleaning up the transcript.
                </p>
                <div className="style-grid">
                    {STYLES.map(s => (
                        <button
                            key={s.value}
                            className={`style-btn ${transcriptionStyle === s.value ? 'active' : ''}`}
                            onClick={() => setTranscriptionStyle(s.value)}
                            disabled={!llmLoaded}
                            title={s.desc}
                        >
                            {s.label}
                        </button>
                    ))}
                </div>
            </div>

            <p className="pp-tab-note">
                Download the required models from the <strong>Models</strong> tab.
            </p>

        </div>
    );
}
