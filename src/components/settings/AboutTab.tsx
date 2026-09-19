import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';
import { Store } from '@tauri-apps/plugin-store';

export interface HardwareDiagnostics {
    platform: string;
    platform_label: string;
    os_detail: string;
    arch: string;
    is_apple_silicon: boolean;
    cpu_name: string;
    cpu_cores: number;
    ram_total_gb: number;
    ram_used_gb: number;
    gpu_name: string;
    gpu_cores: number | null;
    metal_version: string | null;
    vram_gb: number | null;
    neural_accelerator: string;
    ane_available: boolean;
    cuda_available: boolean;
    directml_available: boolean;
    vulkan_available: boolean;
    sim_dsp: string;
    audio_driver: string;
    whisper_framework: string;
    whisper_coreml_models: string[];
    parakeet_framework: string;
    granite_framework: string;
    active_engine: string;
    active_model_id: string | null;
    active_backend: string;
}

export function AboutTab() {
    const [platform, setPlatform] = useState('');
    const [version, setVersion] = useState('');
    const [hw, setHw] = useState<HardwareDiagnostics | null>(null);
    const [hwLoading, setHwLoading] = useState(true);
    const [confirmReset, setConfirmReset] = useState(false);
    const [resetting, setResetting] = useState(false);
    const [resetError, setResetError] = useState('');

    useEffect(() => {
        invoke<string>('get_platform').then(setPlatform).catch(() => setPlatform('unknown'));
        getVersion().then(setVersion).catch(() => setVersion('0.1.0'));

        setHwLoading(true);
        invoke<HardwareDiagnostics>('get_hardware_diagnostics')
            .then((data) => {
                setHw(data);
                setHwLoading(false);
            })
            .catch((err) => {
                console.warn('get_hardware_diagnostics failed:', err);
                setHwLoading(false);
            });
    }, []);

    const platformLabel: Record<string, string> = {
        windows: 'Windows',
        macos: 'macOS',
        linux: 'Linux',
        unknown: 'Unknown',
    };

    const storageFolders: { label: string; folder: string; pathByPlatform: Record<string, string> }[] = [
        {
            label: 'Models',
            folder: 'models',
            pathByPlatform: {
                windows: '%LOCALAPPDATA%\\Taurscribe\\models\\',
                macos: '~/Library/Application Support/Taurscribe/models/',
                linux: '~/.local/share/Taurscribe/models/',
            },
        },
        {
            label: 'Recordings',
            folder: 'recordings',
            pathByPlatform: {
                windows: '%LOCALAPPDATA%\\Taurscribe\\temp\\',
                macos: '~/Library/Application Support/Taurscribe/temp/',
                linux: '~/.local/share/Taurscribe/temp/',
            },
        },
        {
            label: 'Settings',
            folder: 'settings',
            pathByPlatform: {
                windows: '%LOCALAPPDATA%\\Taurscribe\\',
                macos: '~/Library/Application Support/Taurscribe/',
                linux: '~/.local/share/Taurscribe/',
            },
        },
    ];

    const openFolder = (folder: string) => {
        invoke('open_app_folder', { folder }).catch(err => console.warn('open_app_folder failed:', err));
    };

    const handleFactoryReset = async () => {
        if (resetting) return;
        if (!confirmReset) {
            setConfirmReset(true);
            setResetError('');
            return;
        }
        try {
            setResetting(true);
            setResetError('');
            const restarted = await invoke<boolean>('factory_reset_app_data');
            if (!restarted) {
                const store = await Store.load('settings.json');
                await store.clear();
                await store.save();
                await store.close();
                window.location.reload();
                return;
            }

            window.setTimeout(() => {
                setResetting(false);
                setConfirmReset(false);
                setResetError('Restart did not complete. Reopen Taurscribe manually; the pending reset will retry on next launch.');
            }, 8000);
        } catch (err) {
            setResetting(false);
            setResetError(String(err));
        }
    };

    return (
        <div className="about-tab">
            <h3 className="settings-section-title">About</h3>

            <div className="setting-card">
                <div className="about-hero">
                    <span className="about-app-name">Taurscribe</span>
                    <span className="about-version">v{version}</span>
                </div>
                <p className="setting-card-desc">
                    Local, offline speech-to-text. Nothing leaves your machine.
                </p>
                <div className="about-row">
                    <span className="about-row-label">Platform</span>
                    <span className="about-row-value">{hw?.os_detail ?? platformLabel[platform] ?? platform} ({hw?.arch ?? 'unknown'})</span>
                </div>
                <div className="about-row">
                    <span className="about-row-label">Engine</span>
                    <span className="about-row-value">Tauri 2 · React · Rust</span>
                </div>
            </div>

            {/* Hardware & AI Acceleration Card */}
            <div className="setting-card" style={{ marginTop: '12px' }}>
                <div className="about-section-header">
                    <h4 className="setting-card-label-plain" style={{ margin: 0 }}>Hardware & AI Acceleration</h4>
                    {hw?.is_apple_silicon ? (
                        <span className="about-chip-badge about-chip-badge--apple" title="Apple Neural Engine & Metal active">
                            <span className="about-chip-dot" /> Apple Silicon ANE
                        </span>
                    ) : hw?.cuda_available ? (
                        <span className="about-chip-badge about-chip-badge--cuda" title="NVIDIA CUDA & Tensor Cores active">
                            <span className="about-chip-dot" /> NVIDIA CUDA
                        </span>
                    ) : hw?.directml_available ? (
                        <span className="about-chip-badge about-chip-badge--directml" title="Microsoft DirectML active">
                            <span className="about-chip-dot" /> DirectML
                        </span>
                    ) : (
                        <span className="about-chip-badge about-chip-badge--cpu" title="CPU Vector SIMD active">
                            <span className="about-chip-dot" /> CPU Accelerated
                        </span>
                    )}
                </div>
                <p className="setting-card-desc">
                    Hardware acceleration frameworks and neural processing units active on this device.
                </p>

                {hwLoading ? (
                    <div className="about-hw-loading">Detecting hardware accelerators…</div>
                ) : hw ? (
                    <>
                        <div className="about-hw-group">
                            <div className="about-hw-group-title">System & Compute Topology</div>

                            <div className="about-row">
                                <span className="about-row-label">Processor</span>
                                <span className="about-row-value">{hw.cpu_name} · {hw.cpu_cores} Cores</span>
                            </div>

                            <div className="about-row">
                                <span className="about-row-label">Memory (RAM)</span>
                                <span className="about-row-value">
                                    {hw.ram_total_gb.toFixed(1)} GB Total
                                    {hw.ram_used_gb > 0 && <span className="about-subvalue"> ({hw.ram_used_gb.toFixed(1)} GB active)</span>}
                                </span>
                            </div>

                            <div className="about-row">
                                <span className="about-row-label">Graphics (GPU)</span>
                                <span className="about-row-value">
                                    {hw.gpu_name}
                                    {hw.gpu_cores ? ` · ${hw.gpu_cores} GPU Cores` : ''}
                                    {hw.metal_version ? ` · ${hw.metal_version}` : ''}
                                    {hw.vram_gb ? ` · ${hw.vram_gb.toFixed(1)} GB VRAM` : ''}
                                </span>
                            </div>

                            <div className="about-row">
                                <span className="about-row-label">Neural Accelerator</span>
                                <span className="about-row-value about-row-value--highlight">
                                    {hw.neural_accelerator}
                                </span>
                            </div>

                            <div className="about-row">
                                <span className="about-row-label">DSP Vector SIMD</span>
                                <span className="about-row-value">{hw.sim_dsp}</span>
                            </div>

                            <div className="about-row">
                                <span className="about-row-label">Audio Pipeline</span>
                                <span className="about-row-value">{hw.audio_driver}</span>
                            </div>
                        </div>

                        <div className="about-hw-group" style={{ marginTop: '16px' }}>
                            <div className="about-hw-group-title">Model Acceleration Frameworks</div>

                            <div className="about-engine-row">
                                <div className="about-engine-header">
                                    <span className="about-engine-name">Whisper ASR</span>
                                    <span className={`about-badge ${
                                        hw.active_backend.toLowerCase().includes('coreml')
                                            ? 'about-badge--ane'
                                            : hw.active_backend.toLowerCase().includes('metal') || hw.active_backend.toLowerCase().includes('cuda') || hw.active_backend.toLowerCase().includes('vulkan') || hw.active_backend.toLowerCase().includes('directml')
                                            ? 'about-badge--gpu'
                                            : 'about-badge--cpu'
                                    }`}>
                                        {hw.active_engine === 'whisper' ? `Active: ${hw.active_backend}` : (hw.ane_available ? 'CoreML ANE Ready' : 'GPU Ready')}
                                    </span>
                                </div>
                                <div className="about-engine-desc">{hw.whisper_framework}</div>
                                {hw.whisper_coreml_models.length > 0 && (
                                    <div className="about-engine-extra">
                                        ⚡ ANE Neural Engine Encoder: {hw.whisper_coreml_models.join(', ')} (85x Real-Time)
                                    </div>
                                )}
                            </div>

                            <div className="about-engine-row">
                                <div className="about-engine-header">
                                    <span className="about-engine-name">Parakeet Nemotron</span>
                                    <span className={`about-badge ${
                                        hw.is_apple_silicon ? 'about-badge--mlx' : hw.cuda_available ? 'about-badge--cuda' : 'about-badge--gpu'
                                    }`}>
                                        {hw.active_engine === 'parakeet' ? `Active: ${hw.active_backend}` : (hw.is_apple_silicon ? 'Apple MLX Metal' : 'ONNX GPU')}
                                    </span>
                                </div>
                                <div className="about-engine-desc">{hw.parakeet_framework}</div>
                            </div>

                            <div className="about-engine-row">
                                <div className="about-engine-header">
                                    <span className="about-engine-name">Grammar LLM (FlowScribe)</span>
                                    <span className={`about-badge ${
                                        hw.is_apple_silicon ? 'about-badge--ane' : hw.cuda_available ? 'about-badge--cuda' : 'about-badge--gpu'
                                    }`}>
                                        {hw.active_engine === 'granite' ? `Active: ${hw.active_backend}` : (hw.is_apple_silicon ? 'CoreML Hybrid EP' : 'Hardware Offload')}
                                    </span>
                                </div>
                                <div className="about-engine-desc">{hw.granite_framework}</div>
                            </div>
                        </div>
                    </>
                ) : (
                    <div className="about-hw-error">Hardware diagnostics unavailable</div>
                )}
            </div>

            <div className="setting-card" style={{ marginTop: '12px' }}>
                <h4 className="setting-card-label-plain">Storage Locations</h4>
                <p className="setting-card-desc">All data is stored locally on your machine.</p>
                {storageFolders.map(({ label, folder, pathByPlatform }) => (
                    <div className="about-row about-row--folder" key={folder}>
                        <span className="about-row-label">{label}</span>
                        <code className="about-path">{pathByPlatform[platform] ?? pathByPlatform['windows']}</code>
                        <button
                            type="button"
                            id={`open-folder-${folder}-btn`}
                            data-testid={`open-folder-${folder}-btn`}
                            className="about-open-btn"
                            onClick={() => openFolder(folder)}
                            aria-label={`Open ${label} storage folder`}
                            title={`Open ${label} folder`}
                        >
                            Open ↗
                        </button>
                    </div>
                ))}
            </div>

            <div className="setting-card" style={{ marginTop: '12px' }}>
                <h4 className="setting-card-label-plain">Factory Reset</h4>
                <p className="setting-card-desc">
                    Deletes all local app data and restarts Taurscribe into the setup wizard as a brand-new install.
                    This removes downloaded models, settings, transcript history, and temp files.
                </p>
                <div className="about-reset-actions">
                    <button
                        id={confirmReset ? "factory-reset-confirm-btn" : "factory-reset-btn"}
                        data-testid={confirmReset ? "factory-reset-confirm-btn" : "factory-reset-btn"}
                        className={`ghost-btn ghost-btn--danger ${resetting ? 'ghost-btn--disabled' : ''}`}
                        onClick={handleFactoryReset}
                        disabled={resetting}
                        aria-label={confirmReset ? "Confirm factory reset of all application data" : "Factory reset application data"}
                    >
                        {resetting ? 'Resetting…' : confirmReset ? 'Confirm Factory Reset' : 'Factory Reset'}
                    </button>
                    {confirmReset && !resetting && (
                        <button
                            id="factory-reset-cancel-btn"
                            data-testid="factory-reset-cancel-btn"
                            className="ghost-btn"
                            onClick={() => {
                                setConfirmReset(false);
                                setResetError('');
                            }}
                            aria-label="Cancel factory reset"
                        >
                            Cancel
                        </button>
                    )}
                </div>
                {confirmReset && !resetting && (
                    <p className="setting-card-error" style={{ marginTop: '10px' }}>
                        This action is permanent.
                    </p>
                )}
                {resetError && (
                    <p className="setting-card-error" style={{ marginTop: '10px' }}>
                        Reset failed: {resetError}
                    </p>
                )}
            </div>
        </div>
    );
}
