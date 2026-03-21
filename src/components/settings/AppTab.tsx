import { invoke } from '@tauri-apps/api/core';
import { Store } from '@tauri-apps/plugin-store';

interface AppTabProps {
    closeBehavior: 'tray' | 'quit';
    setCloseBehavior: (val: 'tray' | 'quit') => void;
    soundVolume: number;
    soundMuted: boolean;
    setSoundVolume: (v: number) => void;
    setSoundMuted: (m: boolean) => void;
}

export function AppTab({
    closeBehavior, setCloseBehavior,
    soundVolume, soundMuted, setSoundVolume, setSoundMuted,
}: AppTabProps) {

    const handleCloseBehavior = async (val: 'tray' | 'quit') => {
        setCloseBehavior(val);
        const store = await Store.load('settings.json');
        await store.set('close_behavior', val);
        await store.save();
        await invoke('set_close_behavior', { behavior: val });
    };

    return (
        <div className="app-tab">

            {/* ── Window ──────────────────────────────────────────── */}
            <h3 className="settings-section-title">Window</h3>

            <div className="setting-card">
                <div className="setting-card-header">
                    <span className="setting-card-label">Close button behavior</span>
                </div>
                <p className="setting-card-desc">
                    What happens when you click the window's close (×) button.
                </p>
                <div className="close-behavior-options">
                    <label className={`close-behavior-option${closeBehavior === 'tray' ? ' close-behavior-option--active' : ''}`}>
                        <input
                            type="radio"
                            name="close_behavior"
                            value="tray"
                            checked={closeBehavior === 'tray'}
                            onChange={() => handleCloseBehavior('tray')}
                        />
                        <div className="close-behavior-option-content">
                            <span className="close-behavior-option-title">Minimise to tray</span>
                            <span className="close-behavior-option-desc">
                                The app stays running in the system tray. Click the tray icon to reopen.
                            </span>
                        </div>
                    </label>
                    <label className={`close-behavior-option${closeBehavior === 'quit' ? ' close-behavior-option--active' : ''}`}>
                        <input
                            type="radio"
                            name="close_behavior"
                            value="quit"
                            checked={closeBehavior === 'quit'}
                            onChange={() => handleCloseBehavior('quit')}
                        />
                        <div className="close-behavior-option-content">
                            <span className="close-behavior-option-title">Quit app</span>
                            <span className="close-behavior-option-desc">
                                The process exits completely. Use the tray menu to reopen.
                            </span>
                        </div>
                    </label>
                </div>
            </div>

            {/* ── Sounds ──────────────────────────────────────────── */}
            <h3 className="settings-section-title" style={{ marginTop: '36px' }}>Sound Effects</h3>

            <div className="setting-card">
                <div className="setting-card-header">
                    <span className="setting-card-label-plain">Playback</span>
                    <button
                        className={`ghost-btn ${soundMuted ? 'ghost-btn--danger' : 'ghost-btn--ok'}`}
                        onClick={() => setSoundMuted(!soundMuted)}
                        title={soundMuted ? 'Unmute sounds' : 'Mute sounds'}
                    >
                        {soundMuted ? (
                            <>
                                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                                    <line x1="1" y1="1" x2="23" y2="23" />
                                    <path d="M9 9v3a3 3 0 0 0 5.12 2.12M15 9.34V4a3 3 0 0 0-5.94-.6" />
                                    <path d="M17 16.95A7 7 0 0 1 5 12v-2m14 0v2a7 7 0 0 1-.11 1.23" />
                                    <line x1="12" y1="19" x2="12" y2="23" />
                                    <line x1="8" y1="23" x2="16" y2="23" />
                                </svg>
                                Muted
                            </>
                        ) : (
                            <>
                                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                                    <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
                                    <path d="M19.07 4.93a10 10 0 0 1 0 14.14" />
                                    <path d="M15.54 8.46a5 5 0 0 1 0 7.07" />
                                </svg>
                                On
                            </>
                        )}
                    </button>
                </div>

                <p className="setting-card-desc">
                    Audio cues on recording start, paste complete, and error.
                </p>

                <div className="volume-row" style={{ opacity: soundMuted ? 0.35 : 1, transition: 'opacity 0.2s' }}>
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={{ color: 'var(--text-muted)', flexShrink: 0 }}>
                        <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
                    </svg>
                    <input
                        type="range"
                        min={0} max={1} step={0.01}
                        value={soundVolume}
                        disabled={soundMuted}
                        onChange={e => setSoundVolume(Number(e.target.value))}
                        className="volume-slider"
                    />
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={{ color: 'var(--text-muted)', flexShrink: 0 }}>
                        <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
                        <path d="M19.07 4.93a10 10 0 0 1 0 14.14" />
                        <path d="M15.54 8.46a5 5 0 0 1 0 7.07" />
                    </svg>
                    <span className="volume-pct">{Math.round(soundVolume * 100)}%</span>
                </div>
            </div>

            <div className="setting-card" style={{ marginTop: '12px' }}>
                <h4 className="setting-card-label-plain">Sound events</h4>
                <div className="sound-events">
                    {[
                        { event: 'Recording start', file: 'recStart.wav' },
                        { event: 'Transcript pasted', file: 'paste.wav' },
                        { event: 'Error', file: 'error.wav' },
                    ].map(({ event, file }) => (
                        <div className="sound-event-row" key={file}>
                            <span className="sound-event-name">{event}</span>
                            <span className="sound-event-file">{file}</span>
                        </div>
                    ))}
                </div>
            </div>

        </div>
    );
}
