import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { MeetingPermissions } from '../../utils/meetingPermissions';

export function MeetingAudioPermissions() {
    const [isMac, setIsMac] = useState(false);
    const [permissions, setPermissions] = useState<MeetingPermissions | null>(null);
    const [error, setError] = useState('');
    const [requesting, setRequesting] = useState(false);
    const refreshVersion = useRef(0);
    const refresh = async () => {
        const version = ++refreshVersion.current;
        try {
            const result = await invoke<MeetingPermissions>('check_meeting_permissions');
            if (version !== refreshVersion.current) return;
            setPermissions(result); setError('');
        } catch {
            if (version !== refreshVersion.current) return;
            setPermissions(null);
            setError('Could not verify audio permissions. Try again before recording.');
        }
    };
    useEffect(() => {
        let disposed = false;
        let timer: ReturnType<typeof setInterval> | undefined;
        void invoke<string>('get_platform').then(platform => {
            if (disposed || platform !== 'macos') return;
            setIsMac(true);
            void refresh();
            timer = setInterval(refresh, 1500);
        }).catch(() => {});
        return () => { disposed = true; refreshVersion.current++; clearInterval(timer); };
    }, []);
    if (!isMac) return null;
    const audio = permissions?.system_audio;
    const audioOk = audio === 'granted' || audio === 'not_required';
    const micOk = permissions?.microphone === 'granted';
    const label = (status?: string) => ({
        granted: 'Granted', not_required: 'Granted', denied: 'Blocked',
        restricted: 'Restricted by policy', undetermined: 'Not enabled', unsupported: 'Unavailable',
    }[status ?? ''] ?? 'Checking…');
    const request = async () => {
        setRequesting(true);
        try {
            if (!micOk) {
                if (permissions?.microphone === 'denied' || permissions?.microphone === 'restricted') await invoke('open_microphone_settings');
                else await invoke('request_microphone_permission');
            } else if (audio === 'denied') await invoke('open_system_audio_settings');
            else await invoke('request_system_audio_permission');
            await refresh();
        } catch { setError('Could not request recording access. Open Privacy & Security in System Settings and enable Taurscribe.'); }
        finally { setRequesting(false); }
    };
    return <div className="setting-card" data-testid="meeting-audio-permissions" style={{ marginBottom: 12 }}>
        <div className="setting-card-header">
            <div className="setting-card-label"><span>Meeting audio permissions</span></div>
            <span id="meeting-audio-permissions-status" data-testid="meeting-audio-permissions-status" role="status">{audioOk && micOk ? 'Ready' : 'Needs attention'}</span>
        </div>
        <p className="setting-card-desc">Microphone: {label(permissions?.microphone)} · Call audio: {label(audio)}</p>
        {audio === 'unsupported' ? <p className="setting-card-desc">Meeting audio requires macOS 14.2 or later.</p> :
            !(audioOk && micOk) && <>
                <p className="setting-card-desc">Allow Taurscribe in Privacy & Security → Microphone and Screen & System Audio Recording. After changing system audio access, quit and reopen Taurscribe.</p>
                <button id="meeting-audio-enable-btn" data-testid="meeting-audio-enable-btn" type="button" className="about-open-btn" disabled={requesting || !permissions}
                    onClick={request}>{requesting ? 'Requesting…' : 'Enable meeting audio'}</button>
            </>}
        {error && <p id="meeting-audio-permissions-error" data-testid="meeting-audio-permissions-error" role="alert" className="setting-card-desc">{error}</p>}
        <button id="meeting-audio-check-again-btn" data-testid="meeting-audio-check-again-btn" type="button" className="ghost-btn" onClick={() => void refresh()}>Check again</button>
    </div>;
}
