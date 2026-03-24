import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';
import { Store } from '@tauri-apps/plugin-store';

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
                {storageFolders.map(({ label, folder, pathByPlatform }) => (
                    <div className="about-row about-row--folder" key={folder}>
                        <span className="about-row-label">{label}</span>
                        <code className="about-path">{pathByPlatform[platform] ?? pathByPlatform['windows']}</code>
                        <button
                            type="button"
                            className="about-open-btn"
                            onClick={() => openFolder(folder)}
                            title={`Open ${label} folder`}
                        >
                            Open ↗
                        </button>
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
