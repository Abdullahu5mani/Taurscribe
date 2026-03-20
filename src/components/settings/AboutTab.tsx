import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';

export function AboutTab() {
    const [platform, setPlatform] = useState('');
    const [version, setVersion] = useState('');
    const [confirmReset, setConfirmReset] = useState(false);
    const [resetting, setResetting] = useState(false);
    const [resetError, setResetError] = useState('');

    useEffect(() => {
        invoke<string>('get_platform').then(setPlatform).catch(() => setPlatform('unknown'));
        getVersion().then(setVersion).catch(() => setVersion('0.1.0'));
    }, []);

    const platformLabel: Record<string, string> = {
        windows: 'Windows',
        macos: 'macOS',
        linux: 'Linux',
        unknown: 'Unknown',
    };

    const storagePaths: { label: string; path: string; platform?: string }[] = [
        { label: 'Models & recordings', path: '%LOCALAPPDATA%\\Taurscribe\\', platform: 'windows' },
        { label: 'Models & recordings', path: '~/Library/Application Support/Taurscribe/', platform: 'macos' },
        { label: 'Models & recordings', path: '~/.local/share/Taurscribe/', platform: 'linux' },
        { label: 'Settings', path: '%LOCALAPPDATA%\\Taurscribe\\settings.json', platform: 'windows' },
        { label: 'Settings', path: '~/Library/Application Support/Taurscribe/settings.json', platform: 'macos' },
        { label: 'Settings', path: '~/.local/share/Taurscribe/settings.json', platform: 'linux' },
    ];

    const relevantPaths = storagePaths.filter(p => !p.platform || p.platform === platform);

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
            await invoke('factory_reset_app_data');
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
                    <span className="about-row-value">{platformLabel[platform] ?? platform}</span>
                </div>
                <div className="about-row">
                    <span className="about-row-label">Engine</span>
                    <span className="about-row-value">Tauri 2 · React · Rust</span>
                </div>
            </div>

            <div className="setting-card" style={{ marginTop: '12px' }}>
                <h4 className="setting-card-label-plain">Storage Locations</h4>
                <p className="setting-card-desc">All data is stored locally on your machine.</p>
                {relevantPaths.map(({ label, path }) => (
                    <div className="about-row" key={path}>
                        <span className="about-row-label">{label}</span>
                        <code className="about-path">{path}</code>
                    </div>
                ))}
            </div>

            <div className="setting-card" style={{ marginTop: '12px' }}>
                <h4 className="setting-card-label-plain">AI Engines</h4>
                <div className="about-row">
                    <span className="about-row-label">Whisper</span>
                    <span className="about-row-value">whisper.cpp via whisper-rs</span>
                </div>
                <div className="about-row">
                    <span className="about-row-label">Parakeet</span>
                    <span className="about-row-value">NVIDIA Nemotron via parakeet-rs + ONNX</span>
                </div>
                <div className="about-row">
                    <span className="about-row-label">Grammar LLM</span>
                    <span className="about-row-value">FlowScribe Qwen 2.5 0.5B via llama-cpp-2</span>
                </div>
            </div>

            <div className="setting-card" style={{ marginTop: '12px' }}>
                <h4 className="setting-card-label-plain">Factory Reset</h4>
                <p className="setting-card-desc">
                    Deletes all local app data and restarts Taurscribe into the setup wizard as a brand-new install.
                    This removes downloaded models, settings, transcript history, and temp files.
                </p>
                <div className="about-reset-actions">
                    <button
                        className={`ghost-btn ghost-btn--danger ${resetting ? 'ghost-btn--disabled' : ''}`}
                        onClick={handleFactoryReset}
                        disabled={resetting}
                    >
                        {resetting ? 'Resetting…' : confirmReset ? 'Confirm Factory Reset' : 'Factory Reset'}
                    </button>
                    {confirmReset && !resetting && (
                        <button
                            className="ghost-btn"
                            onClick={() => {
                                setConfirmReset(false);
                                setResetError('');
                            }}
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
