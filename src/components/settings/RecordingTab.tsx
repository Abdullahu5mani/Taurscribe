import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Store } from '@tauri-apps/plugin-store';

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
    enableDenoise: boolean;
    setEnableDenoise: (val: boolean) => void;
    muteBackgroundAudio: boolean;
    setMuteBackgroundAudio: (val: boolean) => void;
}

export function RecordingTab({
    enableOverlay, setEnableOverlay,
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

    // ── Meeting & Dual-Channel state ─────────────────────────────
    const [audioSourceMode, setAudioSourceMode] = useState<string>('mic');
    const [meetingDetectionEnabled, setMeetingDetectionEnabled] = useState(true);
    const [autoRecordMeetings, setAutoRecordMeetings] = useState(false);
    const [meetingSaved, setMeetingSaved] = useState(false);
    const [activeMeetingCount, setActiveMeetingCount] = useState(0);

    useEffect(() => {
        const initMeetings = async () => {
            try {
                const store = await Store.load('settings.json');
                const savedSource = await store.get<string>('audio_source_mode');
                if (savedSource) {
                    setAudioSourceMode(savedSource);
                    invoke('set_audio_source_mode', { mode: savedSource }).catch(() => {});
                } else {
                    const rustSource = await invoke<string>('get_audio_source_mode').catch(() => 'mic');
                    setAudioSourceMode(rustSource);
                }

                const savedAutoRecord = await store.get<boolean>('auto_record_meetings');
                if (savedAutoRecord !== null && savedAutoRecord !== undefined) {
                    setAutoRecordMeetings(savedAutoRecord);
                    invoke('set_auto_record_meetings', { enabled: savedAutoRecord }).catch(() => {});
                } else {
                    const rustAuto = await invoke<boolean>('get_auto_record_meetings').catch(() => false);
                    setAutoRecordMeetings(rustAuto);
                }

                const status = await invoke<{ is_watching: boolean; active_meetings_count: number }>('get_meeting_detection_status').catch(() => null);
                if (status) {
                    setMeetingDetectionEnabled(status.is_watching);
                    setActiveMeetingCount(status.active_meetings_count);
                }
            } catch (e) {
                console.error('Failed to load meeting settings:', e);
            }
        };
        initMeetings();
    }, []);

    const handleAudioSourceChange = async (mode: string) => {
        setAudioSourceMode(mode);
        try {
            await invoke('set_audio_source_mode', { mode });
            const store = await Store.load('settings.json');
            await store.set('audio_source_mode', mode);
            await store.save();
            setMeetingSaved(true);
            setTimeout(() => setMeetingSaved(false), 2000);
        } catch (e) {
            console.error('Failed to set audio source mode:', e);
        }
    };

    const handleToggleMeetingDetection = async (enabled: boolean) => {
        setMeetingDetectionEnabled(enabled);
        try {
            if (enabled) {
                await invoke('start_meeting_detection');
            } else {
                await invoke('stop_meeting_detection');
            }
            const store = await Store.load('settings.json');
            await store.set('meeting_detection_enabled', enabled);
            await store.save();
            setMeetingSaved(true);
            setTimeout(() => setMeetingSaved(false), 2000);
        } catch (e) {
            console.error('Failed to toggle meeting detection:', e);
        }
    };

    const handleToggleAutoRecord = async (enabled: boolean) => {
        setAutoRecordMeetings(enabled);
        try {
            await invoke('set_auto_record_meetings', { enabled });
            const store = await Store.load('settings.json');
            await store.set('auto_record_meetings', enabled);
            await store.save();
            setMeetingSaved(true);
            setTimeout(() => setMeetingSaved(false), 2000);
        } catch (e) {
            console.error('Failed to toggle auto record:', e);
        }
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

                <div
                    id="hotkey-mode-group"
                    data-testid="hotkey-mode-group"
                    className="recording-mode-seg"
                    role="radiogroup"
                    aria-label="Recording mode"
                    style={{ marginBottom: '16px' }}
                >
                    <button
                        type="button"
                        id="hotkey-mode-hold-btn"
                        data-testid="hotkey-mode-hold-btn"
                        role="radio"
                        aria-checked={currentBinding.mode === 'hold'}
                        aria-label="Hold to record mode"
                        className={currentBinding.mode === 'hold' ? 'active' : ''}
                        onClick={() => handleModeChange('hold')}
                    >Hold to Record</button>
                    <button
                        type="button"
                        id="hotkey-mode-toggle-btn"
                        data-testid="hotkey-mode-toggle-btn"
                        role="radio"
                        aria-checked={currentBinding.mode === 'toggle'}
                        aria-label="Click to toggle record mode"
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
                            <button
                                type="button"
                                id="hotkey-change-btn"
                                data-testid="hotkey-change-btn"
                                className="ghost-btn"
                                onClick={startRecording}
                                aria-label="Change global hotkey binding"
                            >
                                Change
                            </button>
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
                                type="button"
                                id="hotkey-save-btn"
                                data-testid="hotkey-save-btn"
                                className={`ghost-btn ghost-btn--confirm ${pendingKeys.length !== 2 ? 'ghost-btn--disabled' : ''}`}
                                onClick={saveBinding}
                                disabled={pendingKeys.length !== 2}
                                aria-label="Save hotkey binding"
                            >Save</button>
                            <button
                                type="button"
                                id="hotkey-cancel-btn"
                                data-testid="hotkey-cancel-btn"
                                className="ghost-btn"
                                onClick={cancelRecording}
                                aria-label="Cancel hotkey recording"
                            >Cancel</button>
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
                    <label className="switch" htmlFor="recording-overlay-toggle">
                        <input
                            id="recording-overlay-toggle"
                            data-testid="recording-overlay-toggle"
                            role="switch"
                            aria-checked={enableOverlay}
                            aria-label="Recording overlay HUD"
                            type="checkbox"
                            checked={enableOverlay}
                            onChange={e => setEnableOverlay(e.target.checked)}
                        />
                        <span className="slider round" />
                    </label>
                </div>
                <p className="setting-card-desc">
                    Shows the compact floating HUD on screen while recording via the global hotkey.
                </p>
            </div>

            {/* ── Audio ────────────────────────────────────────────── */}
            <h3 className="settings-section-title" style={{ marginTop: '36px' }}>Audio</h3>

            <div className="setting-card">
                <div className="setting-card-header">
                    {audioSaved && (
                        <span
                            id="recording-audio-saved"
                            data-testid="recording-audio-saved"
                            className="saved-confirm"
                            role="status"
                        >
                            Saved ✓
                        </span>
                    )}
                    {audioError && (
                        <span
                            id="recording-audio-error"
                            data-testid="recording-audio-error"
                            role="alert"
                            className="setting-card-error"
                        >
                            {audioError}
                        </span>
                    )}
                </div>
                <p className="setting-card-desc">
                    Choose which microphone Taurscribe records from. Takes effect on the next recording.
                </p>
                {loadingDevices ? (
                    <div className="audio-detecting">Detecting devices…</div>
                ) : (
                    <select
                        id="recording-device-select"
                        data-testid="recording-device-select"
                        aria-label="Microphone input device"
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
                    <label className="switch" htmlFor="recording-denoise-toggle">
                        <input
                            id="recording-denoise-toggle"
                            data-testid="recording-denoise-toggle"
                            role="switch"
                            aria-checked={enableDenoise}
                            aria-label="RNNoise background noise reduction"
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
                    <label className="switch" htmlFor="recording-mute-bg-toggle">
                        <input
                            id="recording-mute-bg-toggle"
                            data-testid="recording-mute-bg-toggle"
                            role="switch"
                            aria-checked={muteBackgroundAudio}
                            aria-label="Mute system audio during recording"
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

            {/* ── Meeting & Dual-Channel Audio ───────────────────── */}
            <h3 className="settings-section-title" style={{ marginTop: '36px' }}>Meeting &amp; Dual-Channel Audio</h3>

            <div className="setting-card">
                <div className="setting-card-header">
                    <span className="setting-card-label-plain" style={{ fontWeight: 600 }}>Default Recording Mode</span>
                    {meetingSaved && <span className="saved-confirm">Saved ✓</span>}
                </div>
                <p className="setting-card-desc">
                    Choose whether dictation uses standard single-mic input or dual-channel loopback.
                </p>

                <div
                    id="audio-source-mode-group"
                    data-testid="audio-source-mode-group"
                    className="recording-mode-seg"
                    role="radiogroup"
                    aria-label="Default audio source mode"
                    style={{ marginBottom: '16px' }}
                >
                    <button
                        type="button"
                        id="audio-source-mic-btn"
                        data-testid="audio-source-mic-btn"
                        role="radio"
                        aria-checked={audioSourceMode === 'mic'}
                        className={audioSourceMode === 'mic' ? 'active' : ''}
                        onClick={() => handleAudioSourceChange('mic')}
                    >Standard Microphone</button>
                    <button
                        type="button"
                        id="audio-source-dual-btn"
                        data-testid="audio-source-dual-btn"
                        role="radio"
                        aria-checked={audioSourceMode === 'dual_channel'}
                        className={audioSourceMode === 'dual_channel' ? 'active' : ''}
                        onClick={() => handleAudioSourceChange('dual_channel')}
                    >Dual-Channel (Mic + Call)</button>
                </div>
                <p className="setting-card-desc" style={{ marginBottom: '16px' }}>
                    {audioSourceMode === 'dual_channel'
                        ? 'Captures your voice on Channel 1 and internal call audio on Channel 2 into a stereo WAV.'
                        : 'Captures only your voice for fast, isolated voice dictation and command typing.'}
                </p>

                <div className="info-row">
                    <span className="info-row-label">Channel 1 (Left)</span>
                    <span className="info-row-value">Microphone (Your Voice)</span>
                </div>
                <div className="info-row">
                    <span className="info-row-label">Channel 2 (Right)</span>
                    <span className="info-row-value">System Loopback (Remote Participants)</span>
                </div>
                <div className="info-row">
                    <span className="info-row-label">Sample Rate</span>
                    <span className="info-row-value">48,000 Hz Stereo (Lossless WAV)</span>
                </div>
            </div>

            <div className="setting-card" style={{ marginTop: '12px' }}>
                <div className="setting-card-header">
                    <div className="setting-card-label">
                        <span className="status-dot" style={{ background: meetingDetectionEnabled ? 'var(--success)' : 'var(--text-muted)' }} />
                        <span>Auto-Detect Meetings</span>
                        <span className="setting-card-meta">Zoom · Teams · Meet · Slack · Discord</span>
                    </div>
                    <label className="switch" htmlFor="meeting-detection-toggle">
                        <input
                            id="meeting-detection-toggle"
                            data-testid="meeting-detection-toggle"
                            role="switch"
                            aria-checked={meetingDetectionEnabled}
                            aria-label="Auto-detect meetings"
                            type="checkbox"
                            checked={meetingDetectionEnabled}
                            onChange={e => handleToggleMeetingDetection(e.target.checked)}
                        />
                        <span className="slider round" />
                    </label>
                </div>
                <p className="setting-card-desc">
                    Scans native apps and web browser calls to display a convenient 1-click recording banner when a meeting starts.
                </p>
                <div className="info-row">
                    <span className="info-row-label">Detector Engine</span>
                    <span className="info-row-value">Native OS Process Audio Tap (Bot-Free)</span>
                </div>
                <div className="info-row">
                    <span className="info-row-label">Active Calls</span>
                    <span className="info-row-value">{activeMeetingCount > 0 ? `${activeMeetingCount} active call in progress` : 'None detected'}</span>
                </div>
            </div>

            <div className="setting-card" style={{ marginTop: '12px' }}>
                <div className="setting-card-header">
                    <div className="setting-card-label">
                        <span className="status-dot" style={{ background: autoRecordMeetings ? 'var(--success)' : 'var(--text-muted)' }} />
                        <span>Auto-Record Meetings</span>
                    </div>
                    <label className="switch" htmlFor="auto-record-meetings-toggle">
                        <input
                            id="auto-record-meetings-toggle"
                            data-testid="auto-record-meetings-toggle"
                            role="switch"
                            aria-checked={autoRecordMeetings}
                            aria-label="Auto-record detected meetings"
                            type="checkbox"
                            checked={autoRecordMeetings}
                            disabled={!meetingDetectionEnabled}
                            onChange={e => handleToggleAutoRecord(e.target.checked)}
                        />
                        <span className="slider round" />
                    </label>
                </div>
                <p className="setting-card-desc">
                    Automatically starts dual-channel recording the moment a meeting call is confirmed active.
                </p>
            </div>

        </div>
    );
}
