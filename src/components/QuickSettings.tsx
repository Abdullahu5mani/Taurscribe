
import { memo, useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { IconBolt, IconCpu, IconVolumeHigh, IconVolumeLow, IconVolumeMuted, InfoTooltip } from "./Icons";
import { deviceChoices, deviceLabel, loadDeviceReport, selectedDeviceId, type DeviceReport } from "../utils/computeDevices";

const STYLES: { value: string; label: string }[] = [
    { value: 'Casual', label: 'Casual' },
    { value: 'Verbatim', label: 'Verbatim' },
    { value: 'Enthusiastic', label: 'Enthusiastic' },
    { value: 'Software_Dev', label: 'Software Dev' },
    { value: 'Professional', label: 'Professional' },
];

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
    // Tone / transcription style
    transcriptionStyle: string;
    setTranscriptionStyle: (val: string) => void;
    // ASR hardware backend (Whisper/Granite/Qwen3 — GPU or CPU)
    backendInfo: string;
    asrBackend: "gpu" | "cpu";
    /** The chosen GPU ("auto" or an id); matters only with more than one GPU. */
    computeDevice?: string;
    onToggleAsrBackend: (backend: "gpu" | "cpu", device?: string, label?: string) => void;
    asrBackendLoading: boolean;
    activeEngine: string;
    // LLM backend
    llmBackend: "gpu" | "cpu";
    setLlmBackend: (val: "gpu" | "cpu") => void;
    // Sound
    soundVolume: number;
    soundMuted: boolean;
    setSoundVolume: (val: number) => void;
    setSoundMuted: (val: boolean) => void;
}


function Toggle({
    id, checked, onChange, disabled, 'aria-label': ariaLabel,
}: { id: string; checked: boolean; onChange: (v: boolean) => void; disabled?: boolean; 'aria-label'?: string }) {
    return (
        <label className={`qs-toggle${disabled ? " qs-toggle--disabled" : ""}`} htmlFor={id}>
            <input
                id={id}
                data-testid={id}
                role="switch"
                aria-checked={checked}
                aria-label={ariaLabel}
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

function Section({ label, info }: { label: string; info?: string }) {
    return (
        <div className="qs-section-label">
            {label}
            {info && <InfoTooltip text={info} size={10} />}
        </div>
    );
}

function Row({
    label, children, hint, info,
}: { label: string; children: React.ReactNode; hint?: string; info?: string }) {
    return (
        <div className="qs-row">
            <div className="qs-row-left">
                <div className="qs-row-label-wrap">
                    <span className="qs-row-label">{label}</span>
                    {info && <InfoTooltip text={info} size={11} />}
                </div>
                {hint && <span className="qs-row-hint">{hint}</span>}
            </div>
            <div className="qs-row-right">{children}</div>
        </div>
    );
}

function QuickSettingsComponent({
    enableGrammarLM, setEnableGrammarLM, llmStatus,
    enableDenoise, setEnableDenoise,
    enableOverlay, setEnableOverlay,
    muteBackgroundAudio, setMuteBackgroundAudio,
    transcriptionStyle, setTranscriptionStyle,
    backendInfo, asrBackend, computeDevice = "auto", onToggleAsrBackend, asrBackendLoading,
    llmBackend, setLlmBackend,
    soundVolume, soundMuted, setSoundVolume, setSoundMuted,
}: QuickSettingsProps) {
    // macOS fix: Detect platform to hide the GPU/CPU backend toggle which
    // is irrelevant on macOS (Apple Silicon uses Metal automatically).
    const [platform, setPlatform] = useState('');
    useEffect(() => { invoke<string>('get_platform').then(setPlatform).catch(() => {}); }, []);
    const isMac = platform === 'macos';
    // With more than one GPU (CUDA and Vulkan, or an iGPU beside a card) each
    // gets a button; otherwise the plain GPU / CPU pair.
    const [deviceReport, setDeviceReport] = useState<DeviceReport | null>(null);
    useEffect(() => { if (!isMac && platform) loadDeviceReport().then(setDeviceReport); }, [isMac, platform]);
    const { gpus, showPicker } = deviceChoices(deviceReport);
    const selectedDevice = selectedDeviceId(asrBackend, computeDevice, gpus);

    const llmHint =
        llmStatus === "Not Downloaded" ? "not downloaded" :
            llmStatus === "Loading..." ? "loading…" :
                llmStatus === "Loaded" ? "loaded" : undefined;

    const volumeLabel = useMemo(() => {
        return soundMuted ? "Off" : `${Math.round(soundVolume * 100)}%`;
    }, [soundMuted, soundVolume]);

    const onVolumeChange = useCallback((value: number) => {
        setSoundVolume(value);
        if (value > 0 && soundMuted) {
            setSoundMuted(false);
        }
    }, [setSoundMuted, setSoundVolume, soundMuted]);

    return (
        <aside className="quick-settings" aria-label="Quick Settings">
            <div className="qs-header">
                <span className="qs-title">Quick Settings</span>
            </div>

            <div className="qs-body">
                {/* ── Post-processing ─────────────────────────── */}
                <Section label="Post-processing" />

                <Row label="Grammar LLM" hint={llmHint}
                    info="FlowScribe 2.5 0.5B fixes grammar, punctuation, and tone after each recording.">
                    <Toggle
                        id="qs-grammar"
                        aria-label="Grammar LLM post-processing"
                        checked={enableGrammarLM}
                        onChange={setEnableGrammarLM}
                        disabled={llmStatus === "Loading..." || llmStatus === "Not Downloaded"}
                    />
                </Row>


                <Row label="Denoise"
                    info="Background noise reduction. Only affects the AI input — saved WAV keeps original audio.">
                    <Toggle
                        id="qs-denoise"
                        aria-label="Background noise reduction"
                        checked={enableDenoise}
                        onChange={setEnableDenoise}
                    />
                </Row>

                <Row label="Overlay"
                    info="Floating transcript window — see words appear as you speak.">
                    <Toggle
                        id="qs-overlay"
                        aria-label="Floating transcript overlay"
                        checked={enableOverlay}
                        onChange={setEnableOverlay}
                    />
                </Row>

                <Row label="Mute mic BG"
                    info="Mutes system audio while recording to prevent bleed-in.">
                    <Toggle
                        id="qs-mute-bg"
                        aria-label="Mute mic background audio"
                        checked={muteBackgroundAudio}
                        onChange={setMuteBackgroundAudio}
                    />
                </Row>

                {/* ── Tone and where the LLM runs: only while the grammar LLM is on ── */}
                {enableGrammarLM && (
                  <>
                <Section label="Tone" info="Transcription style applied by the grammar LLM." />
                <div
                    id="qs-style-pills"
                    data-testid="qs-style-pills"
                    className="qs-style-pills"
                    role="radiogroup"
                    aria-label="Transcription tone style"
                >
                    {STYLES.map(s => (
                        <button
                            key={s.value}
                            type="button"
                            id={`qs-style-pill-${s.value.toLowerCase()}`}
                            data-testid={`qs-style-pill-${s.value.toLowerCase()}`}
                            role="radio"
                            aria-checked={transcriptionStyle === s.value}
                            aria-label={`${s.label} transcription style`}
                            className={`qs-style-pill${transcriptionStyle === s.value ? " qs-style-pill--active" : ""}`}
                            disabled={!enableGrammarLM || llmStatus !== 'Loaded'}
                            onClick={() => setTranscriptionStyle(s.value)}
                        >
                            {s.label}
                        </button>
                    ))}
                </div>

                    {!isMac && (
                        <div className="qs-backend-row" role="group" aria-label="Grammar LLM runs on" id="qs-llm-backend-group" data-testid="qs-llm-backend-group">
                            <span className="qs-backend-label">LLM runs on</span>
                            <button
                                type="button"
                                id="qs-llm-backend-gpu"
                                data-testid="qs-llm-backend-gpu"
                                className={`qs-backend-btn${llmBackend === "gpu" ? " qs-backend-btn--active" : ""}`}
                                onClick={() => setLlmBackend("gpu")}
                                aria-pressed={llmBackend === "gpu"}
                                title="Run the grammar LLM on the GPU (fast, uses VRAM)"
                            ><IconBolt size={11} style={{ color: '#facc15' }} /> GPU</button>
                            <button
                                type="button"
                                id="qs-llm-backend-cpu"
                                data-testid="qs-llm-backend-cpu"
                                className={`qs-backend-btn${llmBackend === "cpu" ? " qs-backend-btn--active" : ""}`}
                                onClick={() => setLlmBackend("cpu")}
                                aria-pressed={llmBackend === "cpu"}
                                title="Run the grammar LLM on the CPU (works everywhere, slower)"
                            ><IconCpu size={11} /> CPU</button>
                        </div>
                    )}
                  </>
                )}

                {/* ── Speech engine hardware (hidden on macOS: always Metal) ── */}
                {!isMac && (
                  <>
                    <Section label="Speech engine" info="Where the speech-recognition engine (Whisper, Granite, Qwen3) runs. A GPU is much faster." />
                    {showPicker ? (
                        <div className="qs-select-row">
                            <select
                                id="qs-asr-device"
                                data-testid="qs-asr-device"
                                className="qs-select"
                                aria-label="Speech engine device"
                                title={backendInfo}
                                value={selectedDevice}
                                disabled={asrBackendLoading}
                                onChange={(e) => {
                                    const id = e.target.value;
                                    const d = gpus.find((g) => g.id === id);
                                    if (d) onToggleAsrBackend("gpu", d.id, deviceLabel(d));
                                    else onToggleAsrBackend("cpu");
                                }}
                            >
                                {gpus.map((d) => <option key={d.id} value={d.id}>{deviceLabel(d)}</option>)}
                                <option value="cpu">CPU</option>
                            </select>
                        </div>
                    ) : (
                        <div className="qs-backend-row" role="group" aria-label="Speech engine backend" id="qs-asr-backend-group" data-testid="qs-asr-backend-group">
                            <button
                                type="button"
                                id="qs-asr-backend-gpu"
                                data-testid="qs-asr-backend-gpu"
                                className={`qs-backend-btn${asrBackend === "gpu" ? " qs-backend-btn--active" : ""}`}
                                onClick={() => onToggleAsrBackend("gpu")}
                                disabled={asrBackendLoading}
                                aria-pressed={asrBackend === "gpu"}
                                title={backendInfo || "Run the speech engine on the GPU (faster, uses VRAM)"}
                            ><IconBolt size={11} style={{ color: '#facc15' }} /> GPU</button>
                            <button
                                type="button"
                                id="qs-asr-backend-cpu"
                                data-testid="qs-asr-backend-cpu"
                                className={`qs-backend-btn${asrBackend === "cpu" ? " qs-backend-btn--active" : ""}`}
                                onClick={() => onToggleAsrBackend("cpu")}
                                disabled={asrBackendLoading}
                                aria-pressed={asrBackend === "cpu"}
                                title="Run the speech engine on the CPU (works everywhere, slower)"
                            ><IconCpu size={11} /> CPU</button>
                        </div>
                    )}
                  </>
                )}

                {/* ── Sound ───────────────────────────────────── */}
                <Section label="Sound" info="Volume for in-app sounds (start, stop, error beeps)." />
                <div className="qs-volume-row">
                    <button
                        type="button"
                        id="qs-sound-mute-btn"
                        data-testid="qs-sound-mute-btn"
                        className="qs-mute-btn"
                        onClick={() => setSoundMuted(!soundMuted)}
                        aria-pressed={soundMuted}
                        aria-label={soundMuted ? "Unmute sounds" : "Mute sounds"}
                        title={soundMuted ? "Unmute sounds" : "Mute sounds"}
                    >
                        {soundMuted ? <IconVolumeMuted size={14} /> : soundVolume > 50 ? <IconVolumeHigh size={14} /> : <IconVolumeLow size={14} />}
                    </button>
                    <input
                        type="range"
                        id="qs-volume-slider"
                        data-testid="qs-volume-slider"
                        aria-label="Sound volume"
                        role="slider"
                        aria-valuemin={0}
                        aria-valuemax={100}
                        aria-valuenow={Math.round((soundMuted ? 0 : soundVolume) * 100)}
                        aria-valuetext={volumeLabel}
                        className="qs-volume-slider"
                        min={0}
                        max={1}
                        step={0.01}
                        value={soundMuted ? 0 : soundVolume}
                        onChange={e => onVolumeChange(Number(e.target.value))}
                    />
                    <span className="qs-volume-label">{volumeLabel}</span>
                </div>
            </div>
        </aside>
    );
}

export const QuickSettings = memo(QuickSettingsComponent);
