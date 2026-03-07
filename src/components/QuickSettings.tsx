
import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { IconBolt, IconCpu, IconVolumeHigh, IconVolumeLow, IconVolumeMuted } from "./Icons";

interface QuickSettingsProps {
    // Post-processing toggles
    enableGrammarLM: boolean;
    setEnableGrammarLM: (val: boolean) => void;
    llmStatus: string;
    enableDenoise: boolean;
    setEnableDenoise: (val: boolean) => void;
    enableOverlay: boolean;
    setEnableOverlay: (val: boolean) => void;
    muteBackgroundAudio: boolean;
    setMuteBackgroundAudio: (val: boolean) => void;
    // Style
    transcriptionStyle: string;
    setTranscriptionStyle: (val: string) => void;
    // LLM backend
    llmBackend: "gpu" | "cpu";
    setLlmBackend: (val: "gpu" | "cpu") => void;
    // Sound
    soundVolume: number;
    soundMuted: boolean;
    setSoundVolume: (val: number) => void;
    setSoundMuted: (val: boolean) => void;
    // Counts for personalisation
    dictionaryCount: number;
    snippetsCount: number;
    // Nav
    onOpenSettingsTab: (tab?: string) => void;
}


function Toggle({
    id, checked, onChange, disabled,
}: { id: string; checked: boolean; onChange: (v: boolean) => void; disabled?: boolean }) {
    return (
        <label className={`qs-toggle${disabled ? " qs-toggle--disabled" : ""}`} htmlFor={id}>
            <input
                id={id}
                type="checkbox"
                checked={checked}
                disabled={disabled}
                onChange={e => onChange(e.target.checked)}
                style={{ position: "absolute", opacity: 0, width: 0, height: 0 }}
            />
            <span className={`qs-toggle-track${checked ? " qs-toggle-track--on" : ""}`}>
                <span className="qs-toggle-thumb" />
            </span>
        </label>
    );
}

function Section({ label }: { label: string }) {
    return <div className="qs-section-label">{label}</div>;
}

function Row({
    label, children, hint,
}: { label: string; children: React.ReactNode; hint?: string }) {
    return (
        <div className="qs-row">
            <div className="qs-row-left">
                <span className="qs-row-label">{label}</span>
                {hint && <span className="qs-row-hint">{hint}</span>}
            </div>
            <div className="qs-row-right">{children}</div>
        </div>
    );
}

export function QuickSettings({
    enableGrammarLM, setEnableGrammarLM, llmStatus,
    enableDenoise, setEnableDenoise,
    enableOverlay, setEnableOverlay,
    muteBackgroundAudio, setMuteBackgroundAudio,
    transcriptionStyle: _transcriptionStyle,
    setTranscriptionStyle: _setTranscriptionStyle,
    llmBackend, setLlmBackend,
    soundVolume, soundMuted, setSoundVolume, setSoundMuted,
    dictionaryCount, snippetsCount,
    onOpenSettingsTab,
}: QuickSettingsProps) {
    // macOS fix: Detect platform to hide the GPU/CPU backend toggle which
    // is irrelevant on macOS (Apple Silicon uses Metal automatically).
    const [platform, setPlatform] = useState('');
    useEffect(() => { invoke<string>('get_platform').then(setPlatform).catch(() => {}); }, []);
    const isMac = platform === 'macos';

    const llmHint =
        llmStatus === "Not Downloaded" ? "not downloaded" :
            llmStatus === "Loading..." ? "loading…" :
                llmStatus === "Loaded" ? "loaded" : undefined;

    return (
        <aside className="quick-settings">
            <div className="qs-header">
                <span className="qs-title">Quick Settings</span>
                <button
                    type="button"
                    className="qs-settings-link"
                    onClick={() => onOpenSettingsTab()}
                    title="Open full settings"
                    aria-label="Open settings"
                >
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none"
                        stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                        <circle cx="12" cy="12" r="3" />
                        <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
                    </svg>
                </button>
            </div>

            <div className="qs-body">
                {/* ── Post-processing ─────────────────────────── */}
                <Section label="Post-processing" />

                <Row label="Grammar LLM" hint={llmHint}>
                    <Toggle
                        id="qs-grammar"
                        checked={enableGrammarLM}
                        onChange={setEnableGrammarLM}
                        disabled={llmStatus === "Loading..." || llmStatus === "Not Downloaded"}
                    />
                </Row>


                <Row label="Denoise">
                    <Toggle
                        id="qs-denoise"
                        checked={enableDenoise}
                        onChange={setEnableDenoise}
                    />
                </Row>

                <Row label="Overlay">
                    <Toggle
                        id="qs-overlay"
                        checked={enableOverlay}
                        onChange={setEnableOverlay}
                    />
                </Row>

                <Row label="Mute mic BG">
                    <Toggle
                        id="qs-mute-bg"
                        checked={muteBackgroundAudio}
                        onChange={setMuteBackgroundAudio}
                    />
                </Row>

                {/* ── Hardware ────────────────────────────────── */}
                {/* macOS fix: Hide the LLM GPU/CPU backend toggle on macOS —
                    Apple Silicon uses Metal automatically, no user choice needed. */}
                {!isMac && (
                  <>
                    <Section label="LLM Backend" />
                    <div className="qs-backend-row">
                        <button
                            type="button"
                            className={`qs-backend-btn${llmBackend === "gpu" ? " qs-backend-btn--active" : ""}`}
                            onClick={() => setLlmBackend("gpu")}
                        ><IconBolt size={12} style={{ color: '#facc15' }} /> GPU</button>
                        <button
                            type="button"
                            className={`qs-backend-btn${llmBackend === "cpu" ? " qs-backend-btn--active" : ""}`}
                            onClick={() => setLlmBackend("cpu")}
                        ><IconCpu size={12} /> CPU</button>
                    </div>
                  </>
                )}

                {/* ── Sound ───────────────────────────────────── */}
                <Section label="Sound" />
                <div className="qs-volume-row">
                    <button
                        type="button"
                        className="qs-mute-btn"
                        onClick={() => setSoundMuted(!soundMuted)}
                        title={soundMuted ? "Unmute sounds" : "Mute sounds"}
                    >
                        {soundMuted ? <IconVolumeMuted size={14} /> : soundVolume > 50 ? <IconVolumeHigh size={14} /> : <IconVolumeLow size={14} />}
                    </button>
                    <input
                        type="range"
                        className="qs-volume-slider"
                        min={0}
                        max={1}
                        step={0.01}
                        value={soundMuted ? 0 : soundVolume}
                        onChange={e => {
                            const v = Number(e.target.value);
                            setSoundVolume(v);
                            if (v > 0 && soundMuted) setSoundMuted(false);
                        }}
                        aria-label="Sound volume"
                    />
                    <span className="qs-volume-label">{soundMuted ? "Off" : `${Math.round(soundVolume * 100)}%`}</span>
                </div>

                {/* ── Personalisation ─────────────────────────── */}
                <Section label="Personalisation" />
                <button type="button" className="qs-personal-row" onClick={() => onOpenSettingsTab('dictionary')}>
                    <span>Dictionary</span>
                    <span className="qs-personal-count">{dictionaryCount} entries →</span>
                </button>
                <button type="button" className="qs-personal-row" onClick={() => onOpenSettingsTab('snippets')}>
                    <span>Snippets</span>
                    <span className="qs-personal-count">{snippetsCount} entries →</span>
                </button>
            </div>
        </aside>
    );
}
