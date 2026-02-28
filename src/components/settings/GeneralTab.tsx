import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Store } from '@tauri-apps/plugin-store';

// ── Types ─────────────────────────────────────────────────────────────────────

interface HotkeyBinding {
  keys: string[];
}

// ── Key label mappings ────────────────────────────────────────────────────────

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

// ── Props ─────────────────────────────────────────────────────────────────────

interface GeneralTabProps {
  enableSpellCheck: boolean;
  setEnableSpellCheck: (val: boolean) => void;
  spellCheckStatus: string;
  soundVolume: number;
  soundMuted: boolean;
  setSoundVolume: (v: number) => void;
  setSoundMuted: (m: boolean) => void;
}

// ── Component ─────────────────────────────────────────────────────────────────

export function GeneralTab({ enableSpellCheck, setEnableSpellCheck, spellCheckStatus, soundVolume, soundMuted, setSoundVolume, setSoundMuted }: GeneralTabProps) {
  const [currentBinding, setCurrentBinding] = useState<HotkeyBinding>({ keys: ['ControlLeft', 'MetaLeft'] });
  const [recording, setRecording] = useState(false);
  const [heldKeys, setHeldKeys] = useState<string[]>([]);
  const [pendingKeys, setPendingKeys] = useState<string[]>([]);
  const [saved, setSaved] = useState(false);

  const heldRef = useRef<string[]>([]);
  const pendingRef = useRef<string[]>([]);

  // Load saved binding on mount
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
    heldRef.current = [];
    pendingRef.current = [];
    setHeldKeys([]);
    setPendingKeys([]);
    setSaved(false);
    setRecording(true);
  };

  const cancelRecording = () => {
    setRecording(false);
    heldRef.current = [];
    pendingRef.current = [];
    setHeldKeys([]);
    setPendingKeys([]);
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
      heldRef.current = [];
      pendingRef.current = [];
      setTimeout(() => setSaved(false), 2000);
    } catch (err) {
      console.error('Failed to save hotkey:', err);
    }
  };

  const displayChips = (keys: string[]) =>
    keys.map((k, i) => <span key={i} style={chipStyle}>{KEY_LABELS[k] ?? k}</span>);

  const captureChips = heldKeys.length > 0 ? heldKeys : pendingKeys;

  return (
    <div className="general-settings">
      <h3 className="settings-section-title">General Settings</h3>

      {/* ── Spell check ── */}
      <div className="setting-card" style={{ marginTop: '0', background: 'rgba(30, 41, 59, 0.4)', padding: '20px', borderRadius: '12px', border: '1px solid rgba(148, 163, 184, 0.1)' }}>
        <div className="setting-header" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '12px' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <div className="status-dot" style={{
              backgroundColor: !enableSpellCheck ? '#ef4444' : spellCheckStatus === 'Loading...' ? '#f59e0b' : spellCheckStatus === 'Loaded' ? '#22c55e' : '#ef4444'
            }} />
            <h4 style={{ margin: 0 }}>Spell Check (SymSpell)</h4>
          </div>
          <label className={`switch ${spellCheckStatus === 'Loading...' ? 'switch--disabled' : ''}`} title={spellCheckStatus === 'Loading...' ? 'Loading… please wait' : undefined}>
            <input type="checkbox" checked={enableSpellCheck} onChange={(e) => setEnableSpellCheck(e.target.checked)} disabled={spellCheckStatus === 'Loading...'} />
            <span className="slider round"></span>
          </label>
        </div>
        <p style={{ margin: 0, fontSize: '0.9rem', color: '#94a3b8' }}>Fast dictionary-based spelling correction.</p>
      </div>

      {/* ── Sound Effects ── */}
      <div className="setting-card" style={{ marginTop: '16px', background: 'rgba(30, 41, 59, 0.4)', padding: '20px', borderRadius: '12px', border: '1px solid rgba(148, 163, 184, 0.1)' }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '12px' }}>
          <h4 style={{ margin: 0 }}>Sound Effects</h4>
          <button
            onClick={() => setSoundMuted(!soundMuted)}
            title={soundMuted ? 'Unmute sounds' : 'Mute sounds'}
            style={{
              background: soundMuted ? 'rgba(239,68,68,0.12)' : 'rgba(34,197,94,0.12)',
              border: `1px solid ${soundMuted ? 'rgba(239,68,68,0.35)' : 'rgba(34,197,94,0.35)'}`,
              color: soundMuted ? '#fca5a5' : '#86efac',
              borderRadius: '6px',
              padding: '5px 12px',
              fontSize: '0.8rem',
              cursor: 'pointer',
              display: 'flex',
              alignItems: 'center',
              gap: '5px',
            }}
          >
            {soundMuted ? (
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <line x1="1" y1="1" x2="23" y2="23" />
                <path d="M9 9v3a3 3 0 0 0 5.12 2.12M15 9.34V4a3 3 0 0 0-5.94-.6" />
                <path d="M17 16.95A7 7 0 0 1 5 12v-2m14 0v2a7 7 0 0 1-.11 1.23" />
                <line x1="12" y1="19" x2="12" y2="23" />
                <line x1="8" y1="23" x2="16" y2="23" />
              </svg>
            ) : (
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
                <path d="M19.07 4.93a10 10 0 0 1 0 14.14" />
                <path d="M15.54 8.46a5 5 0 0 1 0 7.07" />
              </svg>
            )}
            {soundMuted ? 'Muted' : 'On'}
          </button>
        </div>
        <p style={{ margin: '0 0 14px', fontSize: '0.9rem', color: '#94a3b8' }}>
          Plays audio feedback on recording start, paste, and error.
        </p>
        <div style={{ display: 'flex', alignItems: 'center', gap: '10px', opacity: soundMuted ? 0.4 : 1, transition: 'opacity 0.2s' }}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#94a3b8" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
          </svg>
          <input
            type="range"
            min={0}
            max={1}
            step={0.01}
            value={soundVolume}
            disabled={soundMuted}
            onChange={(e) => setSoundVolume(Number(e.target.value))}
            style={{ flex: 1, accentColor: '#63b3ed', cursor: soundMuted ? 'not-allowed' : 'pointer' }}
          />
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#94a3b8" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
            <path d="M19.07 4.93a10 10 0 0 1 0 14.14" />
            <path d="M15.54 8.46a5 5 0 0 1 0 7.07" />
          </svg>
          <span style={{ fontSize: '0.78rem', color: '#64748b', minWidth: '32px', textAlign: 'right' }}>
            {Math.round(soundVolume * 100)}%
          </span>
        </div>
      </div>

      {/* ── Hotkey ── */}
      <div className="setting-card" style={{ marginTop: '16px', background: 'rgba(30, 41, 59, 0.4)', padding: '20px', borderRadius: '12px', border: '1px solid rgba(148, 163, 184, 0.1)' }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '12px' }}>
          <h4 style={{ margin: 0 }}>Global Hotkey</h4>
          {!recording && (
            <button onClick={startRecording} style={ghostBtnStyle}>Change</button>
          )}
        </div>

        {/* Current binding */}
        {!recording && (
          <div style={{ display: 'flex', gap: '6px', alignItems: 'center', flexWrap: 'wrap' }}>
            {displayChips(currentBinding.keys)}
            {saved && <span style={{ fontSize: '0.78rem', color: '#22c55e', marginLeft: '6px' }}>Saved ✓</span>}
          </div>
        )}

        {/* Capture mode */}
        {recording && (
          <>
            <p style={{ margin: '0 0 10px', fontSize: '0.82rem', color: '#94a3b8', lineHeight: 1.5 }}>
              Hold up to <strong style={{ color: '#e2e8f0' }}>2 keys</strong> (modifier or F-key), then click Save.
            </p>

            <div style={{
              minHeight: '44px',
              background: 'rgba(255,255,255,0.03)',
              border: '1px dashed rgba(255,255,255,0.15)',
              borderRadius: '8px',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
              gap: '6px', padding: '8px 12px', marginBottom: '12px', flexWrap: 'wrap',
            }}>
              {captureChips.length > 0
                ? captureChips.map((k, i) => (
                  <span key={i} style={{ ...chipStyle, background: 'rgba(99,179,237,0.15)', borderColor: 'rgba(99,179,237,0.4)', color: '#90cdf4' }}>
                    {KEY_LABELS[k] ?? k}
                  </span>
                ))
                : <span style={{ fontSize: '0.8rem', color: '#475569' }}>Waiting for input…</span>
              }
            </div>

            <div style={{ display: 'flex', gap: '8px' }}>
              <button
                onClick={saveBinding}
                disabled={!pendingKeys.length}
                style={{
                  background: pendingKeys.length ? 'rgba(34,197,94,0.15)' : 'rgba(255,255,255,0.03)',
                  border: `1px solid ${pendingKeys.length ? 'rgba(34,197,94,0.4)' : 'rgba(255,255,255,0.08)'}`,
                  color: pendingKeys.length ? '#86efac' : '#475569',
                  borderRadius: '6px', padding: '5px 14px', fontSize: '0.8rem',
                  cursor: pendingKeys.length ? 'pointer' : 'not-allowed',
                }}
              >
                Save
              </button>
              <button onClick={cancelRecording} style={ghostBtnStyle}>Cancel</button>
            </div>

            <p style={{ marginTop: '10px', fontSize: '0.75rem', color: '#334155' }}>
              Supported: Ctrl, Shift, Alt, Win/Cmd, Caps Lock, Esc, Tab, F1–F12
            </p>
          </>
        )}
      </div>
    </div>
  );
}

const chipStyle: React.CSSProperties = {
  display: 'inline-block',
  padding: '3px 9px',
  borderRadius: '5px',
  background: 'rgba(255,255,255,0.07)',
  border: '1px solid rgba(255,255,255,0.13)',
  color: '#cbd5e1',
  fontSize: '0.8rem',
  fontFamily: 'DM Mono, monospace',
  letterSpacing: '0.03em',
};

const ghostBtnStyle: React.CSSProperties = {
  background: 'rgba(255,255,255,0.06)',
  border: '1px solid rgba(255,255,255,0.12)',
  color: '#94a3b8',
  borderRadius: '6px',
  padding: '5px 12px',
  fontSize: '0.8rem',
  cursor: 'pointer',
};
