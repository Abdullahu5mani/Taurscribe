import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Store } from '@tauri-apps/plugin-store';

interface AudioTabProps {
    enableDenoise: boolean;
    setEnableDenoise: (val: boolean) => void;
}

export function AudioTab({ enableDenoise, setEnableDenoise }: AudioTabProps) {
    const [devices, setDevices] = useState<string[]>([]);
    const [selected, setSelected] = useState<string>('');   // '' = system default
    const [saved, setSaved] = useState(false);
    const [loading, setLoading] = useState(true);

    // Load device list and saved preference on mount
    useEffect(() => {
        const init = async () => {
            try {
                const list = await invoke<string[]>('list_input_devices');
                setDevices(list);

                const store = await Store.load('settings.json');
                const savedDevice = await store.get<string>('input_device');
                if (savedDevice) {
                    setSelected(savedDevice);
                    invoke('set_input_device', { name: savedDevice }).catch(() => {});
                } else {
                    setSelected('');
                }
            } catch (e) {
                console.error('Failed to load audio devices:', e);
            } finally {
                setLoading(false);
            }
        };
        init();
    }, []);

    const handleChange = async (value: string) => {
        setSelected(value);
        setSaved(false);
        try {
            await invoke('set_input_device', { name: value || null });
            const store = await Store.load('settings.json');
            if (value) {
                await store.set('input_device', value);
            } else {
                await store.delete('input_device');
            }
            await store.save();
            setSaved(true);
            setTimeout(() => setSaved(false), 2000);
        } catch (e) {
            console.error('Failed to set input device:', e);
        }
    };

    return (
        <div className="audio-tab">
            {/* ── Microphone ──────────────────────────────────────── */}
            <h3 className="settings-section-title">Microphone</h3>

            <div className="setting-card">
                <div className="setting-card-header">
                    <span className="setting-card-label-plain">Input Device</span>
                    {saved && <span className="saved-confirm">Saved ✓</span>}
                </div>
                <p className="setting-card-desc">
                    Choose which microphone Taurscribe records from. Takes effect on the next recording.
                </p>

                {loading ? (
                    <div className="audio-detecting">Detecting devices…</div>
                ) : (
                    <select
                        className="select-input select-input--full"
                        value={selected}
                        onChange={e => handleChange(e.target.value)}
                    >
                        <option value="">System Default</option>
                        {devices.map(name => (
                            <option key={name} value={name}>{name}</option>
                        ))}
                    </select>
                )}

                {devices.length === 0 && !loading && (
                    <p className="audio-no-devices">
                        No input devices detected. Check that a microphone is connected.
                    </p>
                )}
            </div>

            {/* ── Noise Suppression ──────────────────────────────────── */}
            <h3 className="settings-section-title" style={{ marginTop: '32px' }}>Noise Suppression</h3>

            <div className="setting-card">
                <div className="setting-card-header">
                    <div className="setting-card-label">
                        <span className="status-dot" style={{ background: enableDenoise ? '#3ecfa5' : '#4b4b55' }} />
                        <span>RNNoise</span>
                        <span className="setting-card-meta">CPU · real-time · no GPU needed</span>
                    </div>
                    <label className="switch">
                        <input
                            type="checkbox"
                            checked={enableDenoise}
                            onChange={e => setEnableDenoise(e.target.checked)}
                        />
                        <span className="slider round" />
                    </label>
                </div>
                <p className="setting-card-desc">
                    Removes background noise from the audio before it reaches the transcription engine.
                    The saved WAV file always keeps the original unprocessed audio.
                </p>
                <div className="about-row">
                    <span className="about-row-label">Method</span>
                    <span className="about-row-value">RNNoise (recurrent neural network)</span>
                </div>
                <div className="about-row">
                    <span className="about-row-label">Operates at</span>
                    <span className="about-row-value">48 kHz · 480-sample frames</span>
                </div>
                <div className="about-row" style={{ marginTop: '16px' }}>
                    <span className="about-row-label" style={{ color: '#4b4b55' }}>High Quality (DeepFilterNet3)</span>
                    <span className="about-row-value" style={{ color: '#4b4b55' }}>Coming in a future update</span>
                </div>
            </div>

            {/* ── Voice Activity Detection ─────────────────────────── */}
            <h3 className="settings-section-title" style={{ marginTop: '32px' }}>Voice Activity Detection</h3>

            <div className="setting-card">
                <p className="setting-card-desc">
                    VAD filters silence before sending audio to the transcription engine,
                    reducing hallucinations and improving accuracy.
                </p>
                <div className="about-row">
                    <span className="about-row-label">Method</span>
                    <span className="about-row-value">Energy-based (RMS threshold)</span>
                </div>
                <div className="about-row">
                    <span className="about-row-label">Min recording</span>
                    <span className="about-row-value">1500 ms</span>
                </div>
                <div className="about-row" style={{ marginTop: '16px' }}>
                    <span className="about-row-label" style={{ color: '#4b4b55' }}>Threshold control</span>
                    <span className="about-row-value" style={{ color: '#4b4b55' }}>Coming in a future update</span>
                </div>
            </div>
        </div>
    );
}
