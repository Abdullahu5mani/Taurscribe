import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Store } from '@tauri-apps/plugin-store';
import { emitTo } from '@tauri-apps/api/event';

type RecordingMode = 'hold' | 'toggle';
interface HotkeyBinding { keys: string[]; mode: RecordingMode; }

const isMac = navigator.platform.toLowerCase().includes('mac');
const isLinux = navigator.platform.toLowerCase().includes('linux');

const KEY_LABELS: Record<string, string> = {
    ControlLeft: 'Ctrl', ControlRight: 'Ctrl R',
    MetaLeft: isMac ? 'Cmd' : isLinux ? 'Super' : 'Win',
    MetaRight: isMac ? 'Cmd R' : isLinux ? 'Super R' : 'Win R',
    ShiftLeft: 'Shift', ShiftRight: 'Shift R',
    AltLeft: isMac ? 'Option' : 'Alt', AltRight: isMac ? 'Option R' : 'Alt R',
    CapsLock: 'Caps Lock', Escape: 'Esc', Tab: 'Tab',
    F1: 'F1', F2: 'F2', F3: 'F3', F4: 'F4',
    F5: 'F5', F6: 'F6', F7: 'F7', F8: 'F8',
    F9: 'F9', F10: 'F10', F11: 'F11', F12: 'F12',
};

const ALLOWED_KEYS = new Set(Object.keys(KEY_LABELS));

const DEFAULT_BINDING: HotkeyBinding = {
    keys: isMac ? ['ControlLeft', 'AltLeft'] : ['ControlLeft', 'MetaLeft'],
    mode: 'hold',
};

interface RecordingTabProps {
    enableOverlay: boolean;
    setEnableOverlay: (val: boolean) => void;
    overlayStyle: 'minimal' | 'full';
    setOverlayStyle: (val: 'minimal' | 'full') => void;
    enableDenoise: boolean;
    setEnableDenoise: (val: boolean) => void;
    muteBackgroundAudio: boolean;
    setMuteBackgroundAudio: (val: boolean) => void;
}

export function RecordingTab({
    enableOverlay, setEnableOverlay,
    overlayStyle, setOverlayStyle,
    enableDenoise, setEnableDenoise,
    muteBackgroundAudio, setMuteBackgroundAudio,
}: RecordingTabProps) {

    // ── Hotkey state ─────────────────────────────────────────────
    const [currentBinding, setCurrentBinding] = useState<HotkeyBinding>(DEFAULT_BINDING);
    const [pendingMode, setPendingMode] = useState<RecordingMode>('hold');
    const [recording, setRecording] = useState(false);
    const [heldKeys, setHeldKeys] = useState<string[]>([]);
    const [pendingKeys, setPendingKeys] = useState<string[]>([]);
    const [hotkeySaved, setHotkeySaved] = useState(false);

    const heldRef = useRef<string[]>([]);
    const pendingRef = useRef<string[]>([]);

    useEffect(() => {
        const load = async () => {
            try {
                const store = await Store.load('settings.json');
                const saved = await store.get<Partial<HotkeyBinding>>('hotkey_binding');
                if (saved?.keys?.length) {
                    const binding: HotkeyBinding = { keys: saved.keys!, mode: saved.mode ?? 'hold' };
                    setCurrentBinding(binding);
                    setPendingMode(binding.mode);
                    return;
                }
            } catch { /* fall through */ }
            const fromRust = await invoke<HotkeyBinding>('get_hotkey').catch(() => null);
            if (fromRust) {
                const binding: HotkeyBinding = { keys: fromRust.keys, mode: fromRust.mode ?? 'hold' };
                setCurrentBinding(binding);
                setPendingMode(binding.mode);
            }
        };
        load();
    }, []);

    const onKeyDown = useCallback((e: KeyboardEvent) => {
        e.preventDefault(); e.stopPropagation();
        if (!ALLOWED_KEYS.has(e.code)) return;
        if (heldRef.current.includes(e.code)) return;
        if (heldRef.current.length >= 2) return;
        const next = [...heldRef.current, e.code];
        heldRef.current = next; setHeldKeys([...next]);
        pendingRef.current = next; setPendingKeys([...next]);
    }, []);

    const onKeyUp = useCallback((e: KeyboardEvent) => {
        e.preventDefault();
        heldRef.current = heldRef.current.filter(k => k !== e.code);
        setHeldKeys([...heldRef.current]);
    }, []);

    useEffect(() => {
        if (!recording) return;
        window.addEventListener('keydown', onKeyDown, true);
        window.addEventListener('keyup', onKeyUp, true);
        return () => {
            window.removeEventListener('keydown', onKeyDown, true);
            window.removeEventListener('keyup', onKeyUp, true);
        };
    }, [recording, onKeyDown, onKeyUp]);

    const startRecording = () => {
        heldRef.current = []; pendingRef.current = [];
        setHeldKeys([]); setPendingKeys([]);
        setHotkeySaved(false); setRecording(true);
    };

    const cancelRecording = () => {
        setRecording(false);
        heldRef.current = []; pendingRef.current = [];
        setHeldKeys([]); setPendingKeys([]);
    };

    const saveBinding = async () => {
        const keys = pendingRef.current;
        if (keys.length !== 2) return;
        const binding: HotkeyBinding = { keys, mode: pendingMode };
        try {
            await invoke('set_hotkey', { binding });
            const store = await Store.load('settings.json');
            await store.set('hotkey_binding', binding);
            await store.save();
            setCurrentBinding(binding);
            setHotkeySaved(true); setRecording(false);
            setPendingKeys([]); heldRef.current = []; pendingRef.current = [];
            setTimeout(() => setHotkeySaved(false), 2000);
        } catch (err) { console.error('Failed to save hotkey:', err); }
    };

    const handleModeChange = async (mode: RecordingMode) => {
        setPendingMode(mode);
        const binding: HotkeyBinding = { ...currentBinding, mode };
        try {
            await invoke('set_hotkey', { binding });
            const store = await Store.load('settings.json');
            await store.set('hotkey_binding', binding);
            await store.save();
            setCurrentBinding(binding);
            setHotkeySaved(true);
            setTimeout(() => setHotkeySaved(false), 2000);
        } catch (err) { console.error('Failed to save mode:', err); }
    };

    const chips = (keys: string[]) =>
        keys.map((k, i) => <span key={i} className="key-chip">{KEY_LABELS[k] ?? k}</span>);

    const captureChips = heldKeys.length > 0 ? heldKeys : pendingKeys;

    const modeDescription = currentBinding.mode === 'hold'
        ? 'Hold all keys while speaking. Releasing any key stops recording.'
        : 'Press the hotkey once to start recording, then press it again to stop.';

    // ── Audio state ──────────────────────────────────────────────
    const [devices, setDevices] = useState<string[]>([]);
    const [selected, setSelected] = useState('');
    const [audioSaved, setAudioSaved] = useState(false);
    const [audioError, setAudioError] = useState<string | null>(null);
    const [loadingDevices, setLoadingDevices] = useState(true);

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
                } else { setSelected(''); }
            } catch (e) { console.error('Failed to load audio devices:', e); }
            finally { setLoadingDevices(false); }
        };
        init();
    }, []);

    const handleDeviceChange = async (value: string) => {
        setSelected(value); setAudioSaved(false); setAudioError(null);
        try {
            await invoke('set_input_device', { name: value || null });
            const store = await Store.load('settings.json');
            if (value) { await store.set('input_device', value); }
            else { await store.delete('input_device'); }
            await store.save();
            setAudioSaved(true);
            setTimeout(() => setAudioSaved(false), 2000);
        } catch (e) {
            console.error('Failed to set input device:', e);
            setAudioError('Failed to set device. Try again or restart the app.');
            setTimeout(() => setAudioError(null), 5000);
        }
    };

    // ── Overlay style ────────────────────────────────────────────
    const handleOverlayStyleChange = async (val: 'minimal' | 'full') => {
        setOverlayStyle(val);
        const store = await Store.load('settings.json');
        await store.set('overlay_style', val);
        await store.save();
        emitTo('overlay', 'overlay-style-changed', val).catch(() => {});
    };

    return (
        <div className="recording-tab">

            {/* ── Hotkey ──────────────────────────────────────────── */}
            <h3 className="settings-section-title">Global Hotkey</h3>

            <div className="setting-card">
                <p className="setting-card-desc">
                    Press this combination from any application to start or stop recording.
                    Works even when Taurscribe is minimised to the tray.
                </p>

                <div className="recording-mode-seg" style={{ marginBottom: '16px' }}>
                    <button
                        className={currentBinding.mode === 'hold' ? 'active' : ''}
                        onClick={() => handleModeChange('hold')}
                    >Hold to Record</button>
                    <button
                        className={currentBinding.mode === 'toggle' ? 'active' : ''}
                        onClick={() => handleModeChange('toggle')}
                    >Click to Toggle</button>
                </div>
                <p className="setting-card-desc" style={{ marginBottom: '16px' }}>{modeDescription}</p>

                {!recording && (
                    <div className="hotkey-current">
                        <div className="hotkey-chips">{chips(currentBinding.keys)}</div>
                        <div className="hotkey-current-actions">
                            {hotkeySaved && <span className="saved-confirm">Saved ✓</span>}
                            <button className="ghost-btn" onClick={startRecording}>Change</button>
                        </div>
                    </div>
                )}

                {recording && (
                    <div className="hotkey-capture">
                        <p className="hotkey-capture-hint">
                            Press exactly <strong>2 keys</strong> (modifier or F-key), then click Save.
                        </p>
                        <div className="hotkey-capture-zone">
                            {captureChips.length > 0
                                ? captureChips.map((k, i) => (
                                    <span key={i} className="key-chip key-chip--active">{KEY_LABELS[k] ?? k}</span>
                                ))
                                : <span className="hotkey-capture-placeholder">Waiting for input…</span>
                            }
                        </div>
                        <div className="hotkey-capture-actions">
                            <button
                                className={`ghost-btn ghost-btn--confirm ${pendingKeys.length !== 2 ? 'ghost-btn--disabled' : ''}`}
                                onClick={saveBinding}
                                disabled={pendingKeys.length !== 2}
                            >Save</button>
                            <button className="ghost-btn" onClick={cancelRecording}>Cancel</button>
                        </div>
                        <p className="hotkey-supported-keys">
                            Supported: Ctrl · Shift · Alt{isMac ? ' · Option' : ''} · {isMac ? 'Cmd' : isLinux ? 'Super' : 'Win'} · Caps Lock · Esc · Tab · F1–F12
                        </p>
                    </div>
                )}
            </div>

            <div className="setting-card" style={{ marginTop: '12px' }}>
                <h4 className="setting-card-label-plain">How it works</h4>
                <div className="hotkey-steps">
                    {(currentBinding.mode === 'hold' ? [
                        'Focus any text field in any app',
                        'Hold your hotkey combination',
                        'Speak naturally while holding',
                        'Release the hotkey to stop',
                        'Transcribed text is typed at your cursor',
                    ] : [
                        'Focus any text field in any app',
                        'Press your hotkey to start recording',
                        'Speak naturally',
                        'Press the hotkey again to stop',
                        'Transcribed text is typed at your cursor',
                    ]).map((text, i) => (
                        <div className="hotkey-step" key={i}>
                            <span className="hotkey-step-num">{String(i + 1).padStart(2, '0')}</span>
                            <span>{text}</span>
                        </div>
                    ))}
                </div>
            </div>

            {/* ── Overlay ──────────────────────────────────────────── */}
            <h3 className="settings-section-title" style={{ marginTop: '36px' }}>Overlay</h3>

            <div className="setting-card">
                <div className="setting-card-header">
                    <div className="setting-card-label">
                        <span className="status-dot" style={{ background: enableOverlay ? 'var(--success)' : 'var(--text-muted)' }} />
                        <span>Recording Overlay</span>
                    </div>
                    <label className="switch">
                        <input
                            type="checkbox"
                            checked={enableOverlay}
                            onChange={e => setEnableOverlay(e.target.checked)}
                        />
                        <span className="slider round" />
                    </label>
                </div>
                <p className="setting-card-desc">
                    Shows a floating HUD on screen while recording via the global hotkey.
                </p>
            </div>

            <div className="setting-card" style={{ marginTop: '12px' }}>
                <div className="setting-card-header">
                    <span className="setting-card-label-plain">Overlay Style</span>
                </div>
                <p className="setting-card-desc">How the overlay HUD looks while recording.</p>
                <div className="close-behavior-options">
                    <label className={`close-behavior-option${overlayStyle === 'full' ? ' close-behavior-option--active' : ''}`}>
                        <input
                            type="radio"
                            name="overlay_style"
                            value="full"
                            checked={overlayStyle === 'full'}
                            onChange={() => handleOverlayStyleChange('full')}
                        />
                        <div className="close-behavior-option-content">
                            <span className="close-behavior-option-title">Full HUD</span>
                            <span className="close-behavior-option-desc">
                                Card with live transcript and waveform.
                            </span>
                        </div>
                    </label>
                    <label className={`close-behavior-option${overlayStyle === 'minimal' ? ' close-behavior-option--active' : ''}`}>
                        <input
                            type="radio"
                            name="overlay_style"
                            value="minimal"
                            checked={overlayStyle === 'minimal'}
                            onChange={() => handleOverlayStyleChange('minimal')}
                        />
                        <div className="close-behavior-option-content">
                            <span className="close-behavior-option-title">Minimal</span>
                            <span className="close-behavior-option-desc">
                                Compact status pill — engine and phase only.
                            </span>
                        </div>
                    </label>
                </div>
            </div>

            {/* ── Audio ────────────────────────────────────────────── */}
            <h3 className="settings-section-title" style={{ marginTop: '36px' }}>Audio</h3>

            <div className="setting-card">
                <div className="setting-card-header">
                    <span className="setting-card-label-plain">Input Device</span>
                    {audioSaved && <span className="saved-confirm">Saved ✓</span>}
                    {audioError && <span role="alert" className="setting-card-error">{audioError}</span>}
                </div>
                <p className="setting-card-desc">
                    Choose which microphone Taurscribe records from. Takes effect on the next recording.
                </p>
                {loadingDevices ? (
                    <div className="audio-detecting">Detecting devices…</div>
                ) : (
                    <select
                        className="select-input select-input--full"
                        value={selected}
                        onChange={e => handleDeviceChange(e.target.value)}
                        onFocus={() => invoke<string[]>('list_input_devices').then(setDevices).catch(() => {})}
                        onMouseEnter={() => invoke<string[]>('list_input_devices').then(setDevices).catch(() => {})}
                    >
                        <option value="">System Default</option>
                        {devices.map(name => (
                            <option key={name} value={name}>{name}</option>
                        ))}
                    </select>
                )}
                {devices.length === 0 && !loadingDevices && (
                    <p className="audio-no-devices">No input devices detected. Check that a microphone is connected.</p>
                )}
            </div>

            <div className="setting-card" style={{ marginTop: '12px' }}>
                <div className="setting-card-header">
                    <div className="setting-card-label">
                        <span className="status-dot" style={{ background: enableDenoise ? 'var(--success)' : 'var(--text-muted)' }} />
                        <span>RNNoise</span>
                        <span className="setting-card-meta">CPU · real-time</span>
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
                    Removes background noise before it reaches the transcription engine.
                    The saved WAV file always keeps the original audio.
                </p>
            </div>

            <div className="setting-card" style={{ marginTop: '12px' }}>
                <div className="setting-card-header">
                    <div className="setting-card-label">
                        <span className="status-dot" style={{ background: muteBackgroundAudio ? 'var(--success)' : 'var(--text-muted)' }} />
                        <span>Mute During Recording</span>
                    </div>
                    <label className="switch">
                        <input
                            type="checkbox"
                            checked={muteBackgroundAudio}
                            onChange={e => setMuteBackgroundAudio(e.target.checked)}
                        />
                        <span className="slider round" />
                    </label>
                </div>
                <p className="setting-card-desc">
                    Silences system audio output while recording so it doesn't bleed into the microphone.
                </p>
            </div>

            <div className="setting-card" style={{ marginTop: '12px' }}>
                <h4 className="setting-card-label-plain">Voice Activity Detection</h4>
                <p className="setting-card-desc">
                    VAD filters silence before sending audio to the engine, reducing hallucinations.
                </p>
                <div className="info-row">
                    <span className="info-row-label">Method</span>
                    <span className="info-row-value">Energy-based (RMS threshold)</span>
                </div>
                <div className="info-row">
                    <span className="info-row-label">Min recording</span>
                    <span className="info-row-value">1500 ms</span>
                </div>
                <div className="info-row info-row--muted" style={{ marginTop: '12px' }}>
                    <span className="info-row-label">Threshold control</span>
                    <span className="info-row-value">Coming in a future update</span>
                </div>
            </div>

        </div>
    );
}
