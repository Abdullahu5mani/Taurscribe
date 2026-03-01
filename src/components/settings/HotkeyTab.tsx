import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Store } from '@tauri-apps/plugin-store';

interface HotkeyBinding { keys: string[]; }

const KEY_LABELS: Record<string, string> = {
    ControlLeft: 'Ctrl', ControlRight: 'Ctrl R',
    MetaLeft: 'Win', MetaRight: 'Win R',
    ShiftLeft: 'Shift', ShiftRight: 'Shift R',
    AltLeft: 'Alt', AltRight: 'Alt R',
    CapsLock: 'Caps Lock', Escape: 'Esc', Tab: 'Tab',
    F1: 'F1', F2: 'F2', F3: 'F3', F4: 'F4',
    F5: 'F5', F6: 'F6', F7: 'F7', F8: 'F8',
    F9: 'F9', F10: 'F10', F11: 'F11', F12: 'F12',
};

const ALLOWED_KEYS = new Set(Object.keys(KEY_LABELS));

interface HotkeyTabProps {
    enableOverlay: boolean;
    setEnableOverlay: (val: boolean) => void;
}

export function HotkeyTab({ enableOverlay, setEnableOverlay }: HotkeyTabProps) {
    const [currentBinding, setCurrentBinding] = useState<HotkeyBinding>({ keys: ['ControlLeft', 'MetaLeft'] });
    const [recording, setRecording] = useState(false);
    const [heldKeys, setHeldKeys] = useState<string[]>([]);
    const [pendingKeys, setPendingKeys] = useState<string[]>([]);
    const [saved, setSaved] = useState(false);

    const heldRef = useRef<string[]>([]);
    const pendingRef = useRef<string[]>([]);

    useEffect(() => {
        const load = async () => {
            try {
                const store = await Store.load('settings.json');
                const saved = await store.get<HotkeyBinding>('hotkey_binding');
                if (saved?.keys?.length) { setCurrentBinding(saved); return; }
            } catch { /* fall through */ }
            const fromRust = await invoke<HotkeyBinding>('get_hotkey').catch(() => null);
            if (fromRust) setCurrentBinding(fromRust);
        };
        load();
    }, []);

    const onKeyDown = useCallback((e: KeyboardEvent) => {
        e.preventDefault();
        e.stopPropagation();
        if (!ALLOWED_KEYS.has(e.code)) return;
        if (heldRef.current.includes(e.code)) return;
        if (heldRef.current.length >= 2) return;
        const next = [...heldRef.current, e.code];
        heldRef.current = next;
        setHeldKeys([...next]);
        pendingRef.current = next;
        setPendingKeys([...next]);
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
        setSaved(false); setRecording(true);
    };

    const cancelRecording = () => {
        setRecording(false);
        heldRef.current = []; pendingRef.current = [];
        setHeldKeys([]); setPendingKeys([]);
    };

    const saveBinding = async () => {
        const keys = pendingRef.current;
        if (!keys.length) return;
        const binding: HotkeyBinding = { keys };
        try {
            await invoke('set_hotkey', { binding });
            const store = await Store.load('settings.json');
            await store.set('hotkey_binding', binding);
            await store.save();
            setCurrentBinding(binding);
            setSaved(true);
            setRecording(false);
            setPendingKeys([]);
            heldRef.current = []; pendingRef.current = [];
            setTimeout(() => setSaved(false), 2000);
        } catch (err) {
            console.error('Failed to save hotkey:', err);
        }
    };

    const chips = (keys: string[]) =>
        keys.map((k, i) => <span key={i} className="key-chip">{KEY_LABELS[k] ?? k}</span>);

    const captureChips = heldKeys.length > 0 ? heldKeys : pendingKeys;

    return (
        <div className="hotkey-tab">
            <h3 className="settings-section-title">Global Hotkey</h3>

            <div className="setting-card">
                <p className="setting-card-desc">
                    Press this combination from any application to start or stop recording.
                    The hotkey works even when Taurscribe is minimised to the tray.
                </p>

                {!recording && (
                    <div className="hotkey-current">
                        <div className="hotkey-chips">{chips(currentBinding.keys)}</div>
                        <div className="hotkey-current-actions">
                            {saved && <span className="saved-confirm">Saved ✓</span>}
                            <button className="ghost-btn" onClick={startRecording}>Change</button>
                        </div>
                    </div>
                )}

                {recording && (
                    <div className="hotkey-capture">
                        <p className="hotkey-capture-hint">
                            Hold up to <strong>2 keys</strong> (modifier or F-key), then click Save.
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
                                className={`ghost-btn ghost-btn--confirm ${!pendingKeys.length ? 'ghost-btn--disabled' : ''}`}
                                onClick={saveBinding}
                                disabled={!pendingKeys.length}
                            >
                                Save
                            </button>
                            <button className="ghost-btn" onClick={cancelRecording}>Cancel</button>
                        </div>
                        <p className="hotkey-supported-keys">
                            Supported: Ctrl · Shift · Alt · Win/Cmd · Caps Lock · Esc · Tab · F1–F12
                        </p>
                    </div>
                )}
            </div>

            {/* ── Overlay Indicator ──────────────────────────────── */}
            <h3 className="settings-section-title" style={{ marginTop: '32px' }}>Overlay Indicator</h3>

            <div className="setting-card">
                <div className="setting-card-header">
                    <div className="setting-card-label">
                        <span className="status-dot" style={{ background: enableOverlay ? '#3ecfa5' : '#4b4b55' }} />
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
                    Shows a small animated indicator on screen when recording via the global hotkey.
                    The overlay does not appear when using the in-app record button.
                </p>
            </div>

            <div className="setting-card" style={{ marginTop: '12px' }}>
                <h4 className="setting-card-label-plain">How it works</h4>
                <div className="hotkey-steps">
                    {[
                        'Focus any text field in any app',
                        'Press your hotkey to start recording',
                        'Speak naturally',
                        'Press the hotkey again to stop',
                        'Transcribed text is typed at your cursor',
                    ].map((text, i) => (
                        <div className="hotkey-step" key={i}>
                            <span className="hotkey-step-num">{String(i + 1).padStart(2, '0')}</span>
                            <span>{text}</span>
                        </div>
                    ))}
                </div>
            </div>
        </div>
    );
}
