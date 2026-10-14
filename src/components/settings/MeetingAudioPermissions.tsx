import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { MeetingPermissions } from '../../utils/meetingPermissions';

export function MeetingAudioPermissions() {
    const [isMac, setIsMac] = useState(false);
    const [permissions, setPermissions] = useState<MeetingPermissions | null>(null);
    const [error, setError] = useState('');
    const [requesting, setRequesting] = useState(false);
    const refresh = async () => {
        try { setPermissions(await invoke<MeetingPermissions>('check_meeting_permissions')); setError(''); }
        catch { setError('Could not verify audio permissions. Try again before recording.'); }
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
        return () => { disposed = true; clearInterval(timer); };
    }, []);
    if (!isMac) return null;
    const audio = permissions?.system_audio;
    const audioOk = audio === 'granted' || audio === 'not_required';
    const micOk = permissions?.microphone === 'granted';
    const request = async () => {
        setRequesting(true);
        try {
            if (!micOk) {
                if (permissions?.microphone === 'denied' || permissions?.microphone === 'restricted') await invoke('open_microphone_settings');
                else await invoke('request_microphone_permission');
            } else if (audio === 'denied') await invoke('open_system_audio_settings');
            else await invoke('request_system_audio_permission');
            await refresh();
        } catch (e) { setError(String(e)); }
        finally { setRequesting(false); }
    };
    return <div className="setting-card" data-testid="meeting-audio-permissions" style={{ marginBottom: 12 }}>
        <div className="setting-card-header">
            <div className="setting-card-label"><span>Meeting audio permissions</span></div>
            <span role="status">{audioOk && micOk ? 'Ready' : 'Needs attention'}</span>
        </div>
        <p className="setting-card-desc">Microphone: {micOk ? 'Granted' : permissions?.microphone ?? 'Checking…'} · Call audio: {audioOk ? 'Granted' : audio ?? 'Checking…'}</p>
        {audio === 'unsupported' ? <p className="setting-card-desc">Meeting audio requires macOS 14.2 or later.</p> :
            !(audioOk && micOk) && <>
                <p className="setting-card-desc">Allow Taurscribe in Privacy & Security → Microphone and Screen & System Audio Recording. After changing system audio access, quit and reopen Taurscribe.</p>
                <button type="button" className="about-open-btn" disabled={requesting || !permissions}
                    onClick={request}>{requesting ? 'Requesting…' : 'Enable meeting audio'}</button>
            </>}
        {error && <p role="alert" className="setting-card-desc">{error}</p>}
        <button type="button" className="ghost-btn" onClick={() => void refresh()}>Check again</button>
    </div>;
}
