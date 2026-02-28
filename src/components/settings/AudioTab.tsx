import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Store } from '@tauri-apps/plugin-store';

export function AudioTab() {
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
        <div className="audio-settings">
            <h3 className="settings-section-title">Audio & Microphone</h3>

            <div style={{
                background: 'rgba(30, 41, 59, 0.4)',
                padding: '20px',
                borderRadius: '12px',
                border: '1px solid rgba(148, 163, 184, 0.1)',
            }}>
                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '12px' }}>
                    <h4 style={{ margin: 0 }}>Input Device</h4>
                    {saved && (
                        <span style={{ fontSize: '0.78rem', color: '#22c55e' }}>Saved ✓</span>
                    )}
                </div>

                <p style={{ margin: '0 0 14px', fontSize: '0.9rem', color: '#94a3b8' }}>
                    Choose which microphone Taurscribe records from.
                    Takes effect on the next recording.
                </p>

                {loading ? (
                    <div style={{ color: '#475569', fontSize: '0.85rem' }}>Detecting devices…</div>
                ) : (
                    <select
                        value={selected}
                        onChange={(e) => handleChange(e.target.value)}
                        style={{
                            width: '100%',
                            padding: '10px 12px',
                            borderRadius: '8px',
                            border: '1px solid rgba(148, 163, 184, 0.2)',
                            background: '#0f172a',
                            color: '#e2e8f0',
                            fontSize: '0.875rem',
                            cursor: 'pointer',
                            outline: 'none',
                        }}
                    >
                        <option value="">System Default</option>
                        {devices.map((name) => (
                            <option key={name} value={name}>{name}</option>
                        ))}
                    </select>
                )}

                {devices.length === 0 && !loading && (
                    <p style={{ marginTop: '10px', fontSize: '0.8rem', color: '#ef4444' }}>
                        No input devices detected. Check that a microphone is connected.
                    </p>
                )}
            </div>
        </div>
    );
}
