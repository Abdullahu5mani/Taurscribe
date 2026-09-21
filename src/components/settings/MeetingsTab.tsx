import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Store } from '@tauri-apps/plugin-store';
import { MEETING_KEYS, DEFAULT_AUTORECORD_DELAY, DEFAULT_MATCH_THRESHOLD, DEFAULT_CONTINUE_MINUTES } from './types';

const AUTORECORD_DELAYS = [0, 3, 5, 10];
const CONTINUE_WINDOWS = [0, 5, 10, 30];

interface MeetingsTabProps {
    speakerModelDownloaded: boolean;
    diarizationModelDownloaded: boolean;
    onOpenModels: () => void;
}

async function persist(key: string, value: unknown) {
    const store = await Store.load('settings.json');
    await store.set(key, value);
    await store.save();
}

export function MeetingsTab({ speakerModelDownloaded, diarizationModelDownloaded, onOpenModels }: MeetingsTabProps) {
    const [audioSourceMode, setAudioSourceMode] = useState<string>('mic');
    const [detectionEnabled, setDetectionEnabled] = useState(true);
    const [autoRecord, setAutoRecord] = useState(false);
    const [autoRecordDelay, setAutoRecordDelay] = useState(DEFAULT_AUTORECORD_DELAY);
    const [showBanner, setShowBanner] = useState(true);
    const [matchThreshold, setMatchThreshold] = useState(DEFAULT_MATCH_THRESHOLD);
    const [continueMinutes, setContinueMinutes] = useState(DEFAULT_CONTINUE_MINUTES);
    const [activeMeetingCount, setActiveMeetingCount] = useState(0);
    const [saved, setSaved] = useState(false);

    // Backend state is authoritative for what is running; the store supplies
    // the UI-only preferences.
    useEffect(() => {
        (async () => {
            try {
                const store = await Store.load('settings.json');
                setAudioSourceMode(await invoke<string>('get_audio_source_mode').catch(() => 'mic'));
                setAutoRecord(await invoke<boolean>('get_auto_record_meetings').catch(() => false));
                setMatchThreshold(await invoke<number>('get_speaker_match_threshold').catch(() => DEFAULT_MATCH_THRESHOLD));
                setContinueMinutes(await invoke<number>('get_meeting_continue_minutes').catch(() => DEFAULT_CONTINUE_MINUTES));
                setAutoRecordDelay((await store.get<number>(MEETING_KEYS.autoRecordDelay)) ?? DEFAULT_AUTORECORD_DELAY);
                setShowBanner((await store.get<boolean>(MEETING_KEYS.showBanner)) ?? true);
                const status = await invoke<{ is_watching: boolean; active_meetings_count: number }>('get_meeting_detection_status').catch(() => null);
                if (status) {
                    setDetectionEnabled(status.is_watching);
                    setActiveMeetingCount(status.active_meetings_count);
                }
            } catch (e) {
                console.error('Failed to load meeting settings:', e);
            }
        })();
    }, []);

    const flashSaved = () => {
        setSaved(true);
        setTimeout(() => setSaved(false), 2000);
    };

    const save = async (label: string, apply: () => Promise<unknown>, key: string, value: unknown) => {
        try {
            await apply();
            await persist(key, value);
            flashSaved();
        } catch (e) {
            console.error(`Failed to save ${label}:`, e);
        }
    };

    const handleSourceMode = (mode: string) => {
        setAudioSourceMode(mode);
        void save('audio source mode', () => invoke('set_audio_source_mode', { mode }), MEETING_KEYS.sourceMode, mode);
    };

    const handleDetection = (enabled: boolean) => {
        setDetectionEnabled(enabled);
        void save('meeting detection', () => invoke(enabled ? 'start_meeting_detection' : 'stop_meeting_detection'), MEETING_KEYS.detection, enabled);
    };

    const handleAutoRecord = (enabled: boolean) => {
        setAutoRecord(enabled);
        void save('auto-record', () => invoke('set_auto_record_meetings', { enabled }), MEETING_KEYS.autoRecord, enabled);
    };

    const handleDelay = (seconds: number) => {
        setAutoRecordDelay(seconds);
        void save('auto-record delay', async () => {}, MEETING_KEYS.autoRecordDelay, seconds);
    };

    const handleShowBanner = (enabled: boolean) => {
        setShowBanner(enabled);
        void save('meeting banner', async () => {}, MEETING_KEYS.showBanner, enabled);
    };

    const handleContinue = (minutes: number) => {
        setContinueMinutes(minutes);
        void save('continue window', () => invoke('set_meeting_continue_minutes', { minutes }), MEETING_KEYS.continueMinutes, minutes);
    };

    const handleThreshold = (value: number) => {
        setMatchThreshold(value);
        void save('speaker match threshold', () => invoke('set_speaker_match_threshold', { value }), MEETING_KEYS.matchThreshold, value);
    };

    const toggle = (id: string, label: string, checked: boolean, onChange: (v: boolean) => void, disabled = false) => (
        <label className="switch" htmlFor={id}>
            <input
                id={id}
                data-testid={id}
                role="switch"
                aria-checked={checked}
                aria-label={label}
                type="checkbox"
                checked={checked}
                disabled={disabled}
                onChange={e => onChange(e.target.checked)}
            />
            <span className="slider round" />
        </label>
    );

    const strictness = matchThreshold >= 0.7 ? 'Strict' : matchThreshold <= 0.52 ? 'Loose' : 'Balanced';

    return (
        <div className="meetings-tab">

            {/* ── Detection ───────────────────────────────────────── */}
            <div className="setting-card-header">
                <h3 className="settings-section-title">Detection</h3>
                {saved && <span className="saved-confirm">Saved ✓</span>}
            </div>

            <div className="setting-card">
                <div className="setting-card-header">
                    <div className="setting-card-label">
                        <span className="status-dot" style={{ background: detectionEnabled ? 'var(--success)' : 'var(--text-muted)' }} />
                        <span>Auto-detect meetings</span>
                        <span className="setting-card-meta">Zoom · Teams · Meet · Slack · Discord · Webex</span>
                    </div>
                    {toggle('meeting-detection-toggle', 'Auto-detect meetings', detectionEnabled, handleDetection)}
                </div>
                <p className="setting-card-desc">
                    Watches meeting apps and browser calls on this machine. No bot joins the call.
                </p>
                <div className="info-row">
                    <span className="info-row-label">Active calls</span>
                    <span className="info-row-value">{activeMeetingCount > 0 ? `${activeMeetingCount} in progress` : 'None detected'}</span>
                </div>
            </div>

            <div className="setting-card" style={{ marginTop: '12px' }}>
                <div className="setting-card-header">
                    <span className="setting-card-label">Show meeting banner</span>
                    {toggle('meeting-banner-toggle', 'Show a banner when a meeting is detected', showBanner, handleShowBanner, !detectionEnabled)}
                </div>
                <p className="setting-card-desc">
                    Shows a one-click “Record meeting” banner when a call starts.
                </p>
            </div>

            <div className="setting-card" style={{ marginTop: '12px' }}>
                <div className="setting-card-header">
                    <div className="setting-card-label">
                        <span className="status-dot" style={{ background: autoRecord ? 'var(--success)' : 'var(--text-muted)' }} />
                        <span>Auto-record meetings</span>
                    </div>
                    {toggle('auto-record-meetings-toggle', 'Auto-record detected meetings', autoRecord, handleAutoRecord, !detectionEnabled)}
                </div>
                <p className="setting-card-desc">
                    Starts a dual-channel recording when a call is detected. You can cancel during the countdown.
                </p>
                <div
                    id="auto-record-delay-group"
                    data-testid="auto-record-delay-group"
                    className="recording-mode-seg"
                    role="radiogroup"
                    aria-label="Auto-record countdown"
                    style={{ opacity: autoRecord && detectionEnabled ? 1 : 0.4 }}
                >
                    {AUTORECORD_DELAYS.map(s => (
                        <button
                            type="button"
                            key={s}
                            id={`auto-record-delay-${s}`}
                            data-testid={`auto-record-delay-${s}`}
                            role="radio"
                            aria-checked={autoRecordDelay === s}
                            className={autoRecordDelay === s ? 'active' : ''}
                            disabled={!autoRecord || !detectionEnabled}
                            onClick={() => handleDelay(s)}
                        >{s === 0 ? 'Immediately' : `${s}s countdown`}</button>
                    ))}
                </div>
            </div>

            <div className="setting-card" style={{ marginTop: '12px' }}>
                <div className="setting-card-header">
                    <span className="setting-card-label">Continue a call after a restart</span>
                </div>
                <p className="setting-card-desc">
                    If you stop recording a call and record the same call again within this time, the new part is
                    added to the same meeting (speakers and names carry over) instead of saving a second meeting.
                </p>
                <div
                    id="continue-window-group"
                    data-testid="continue-window-group"
                    className="recording-mode-seg"
                    role="radiogroup"
                    aria-label="Continue a call after a restart"
                >
                    {CONTINUE_WINDOWS.map(m => (
                        <button
                            type="button"
                            key={m}
                            id={`continue-window-${m}`}
                            data-testid={`continue-window-${m}`}
                            role="radio"
                            aria-checked={continueMinutes === m}
                            className={continueMinutes === m ? 'active' : ''}
                            onClick={() => handleContinue(m)}
                        >{m === 0 ? 'Off' : `Within ${m} min`}</button>
                    ))}
                </div>
            </div>

            {/* ── Audio ───────────────────────────────────────────── */}
            <h3 className="settings-section-title" style={{ marginTop: '36px' }}>Audio</h3>

            <div className="setting-card">
                <div className="setting-card-header">
                    <span className="setting-card-label">Hotkey recording source</span>
                </div>
                <p className="setting-card-desc">
                    What the global hotkey records. Meeting recordings are always dual-channel.
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
                        onClick={() => handleSourceMode('mic')}
                    >Microphone only</button>
                    <button
                        type="button"
                        id="audio-source-dual-btn"
                        data-testid="audio-source-dual-btn"
                        role="radio"
                        aria-checked={audioSourceMode === 'dual_channel'}
                        className={audioSourceMode === 'dual_channel' ? 'active' : ''}
                        onClick={() => handleSourceMode('dual_channel')}
                    >Mic + call audio</button>
                </div>
                <div className="info-row">
                    <span className="info-row-label">Left channel</span>
                    <span className="info-row-value">Microphone (you)</span>
                </div>
                <div className="info-row">
                    <span className="info-row-label">Right channel</span>
                    <span className="info-row-value">System audio (other participants)</span>
                </div>
            </div>

            {/* ── Speakers ────────────────────────────────────────── */}
            <h3 className="settings-section-title" style={{ marginTop: '36px' }}>Speakers</h3>

            <div className="setting-card">
                <div className="setting-card-header">
                    <div className="setting-card-label">
                        <span className="status-dot" style={{ background: diarizationModelDownloaded ? 'var(--success)' : 'var(--text-muted)' }} />
                        <span>Speaker separation</span>
                        <span className="setting-card-meta">{diarizationModelDownloaded ? 'Nemotron 3 · up to 8 people' : 'Built-in voice grouping'}</span>
                    </div>
                    {!diarizationModelDownloaded && (
                        <button
                            type="button"
                            id="meetings-download-diarization-model-btn"
                            data-testid="meetings-download-diarization-model-btn"
                            className="about-open-btn"
                            onClick={onOpenModels}
                        >Download ↗</button>
                    )}
                </div>
                <p className="setting-card-desc">
                    You are always the mic channel. The call audio is split into the other people on the call.
                    {diarizationModelDownloaded
                        ? ' Runs once when a meeting ends.'
                        : ' Without the Nemotron model, similar-sounding people can be merged into one speaker.'}
                </p>
            </div>

            <div className="setting-card" style={{ marginTop: '12px' }}>
                <div className="setting-card-header">
                    <div className="setting-card-label">
                        <span className="status-dot" style={{ background: speakerModelDownloaded ? 'var(--success)' : 'var(--text-muted)' }} />
                        <span>Speaker recognition</span>
                        <span className="setting-card-meta">CAM++ · 192-d voiceprints</span>
                    </div>
                    {!speakerModelDownloaded && (
                        <button
                            type="button"
                            id="meetings-download-speaker-model-btn"
                            data-testid="meetings-download-speaker-model-btn"
                            className="about-open-btn"
                            onClick={onOpenModels}
                        >Download ↗</button>
                    )}
                </div>
                <p className="setting-card-desc">
                    Matches each speaker's voice against the Speaker Vault so people you have named are recognised in later meetings.
                    {!speakerModelDownloaded && ' Without this model, people are not recognised across meetings.'}
                </p>

                <div className="setting-card-header" style={{ marginTop: '12px' }}>
                    <span className="setting-card-label-plain">Match strictness</span>
                    <span className="setting-card-meta">{strictness} · {matchThreshold.toFixed(2)}</span>
                </div>
                <div className="volume-row" style={{ opacity: speakerModelDownloaded ? 1 : 0.4 }}>
                    <span className="setting-card-meta">Looser</span>
                    <input
                        type="range"
                        id="speaker-match-threshold-slider"
                        data-testid="speaker-match-threshold-slider"
                        aria-label="Speaker match strictness"
                        min={0.45} max={0.85} step={0.01}
                        value={matchThreshold}
                        disabled={!speakerModelDownloaded}
                        onChange={e => setMatchThreshold(Number(e.target.value))}
                        onPointerUp={e => handleThreshold(Number((e.target as HTMLInputElement).value))}
                        onKeyUp={e => handleThreshold(Number((e.target as HTMLInputElement).value))}
                        className="volume-slider"
                    />
                    <span className="setting-card-meta">Stricter</span>
                </div>
                <p className="setting-card-desc" style={{ marginTop: '8px' }}>
                    Looser recognises people more often but may confuse similar voices. Default 0.60.
                    {Math.abs(matchThreshold - DEFAULT_MATCH_THRESHOLD) > 0.001 && (
                        <>
                            {' '}
                            <button
                                type="button"
                                id="speaker-match-threshold-reset"
                                data-testid="speaker-match-threshold-reset"
                                className="ghost-btn"
                                onClick={() => handleThreshold(DEFAULT_MATCH_THRESHOLD)}
                            >Reset</button>
                        </>
                    )}
                </p>
            </div>

            {/* ── Storage ─────────────────────────────────────────── */}
            <h3 className="settings-section-title" style={{ marginTop: '36px' }}>Storage</h3>

            <div className="setting-card">
                <div className="setting-card-header">
                    <span className="setting-card-label">Meeting audio and voice snippets</span>
                    <button
                        type="button"
                        id="open-folder-meetings-btn"
                        data-testid="open-folder-meetings-btn"
                        className="about-open-btn"
                        onClick={() => invoke('open_app_folder', { folder: 'meetings' }).catch(err => console.warn('open_app_folder failed:', err))}
                        aria-label="Open meetings storage folder"
                    >Open ↗</button>
                </div>
                <p className="setting-card-desc">
                    Recordings are kept on this machine. Deleting a meeting in the Meetings view removes its audio.
                </p>
            </div>

        </div>
    );
}
