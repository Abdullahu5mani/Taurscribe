import { invoke } from '@tauri-apps/api/core';
import type { SessionNotice } from '../types/session';

export interface MeetingPermissions {
    microphone: string;
    system_audio: string;
}

export function meetingPermissionProblem(p: MeetingPermissions | null): { code: string; message: string } | null {
    if (!p || !['granted', 'denied', 'restricted', 'undetermined'].includes(p.microphone)
        || !['granted', 'not_required', 'denied', 'undetermined', 'unsupported'].includes(p.system_audio)) {
        return { code: 'meeting_permission_check_failed', message: 'Could not verify meeting recording permissions. Retry before recording the call.' };
    }
    if (p.system_audio === 'unsupported') {
        return { code: 'meeting_capture_unsupported', message: 'Meeting audio capture is unavailable on this OS. On Mac, update to macOS 14.2 or later.' };
    }
    if (p.microphone !== 'granted') {
        return { code: 'mic_permission_denied', message: p.microphone === 'restricted'
            ? 'Microphone access is restricted by device policy. Ask your administrator to allow recording.'
            : 'Allow Taurscribe in System Settings → Privacy & Security → Microphone before recording.' };
    }
    if (!['granted', 'not_required'].includes(p.system_audio)) {
        return { code: 'system_audio_permission_denied', message: 'Allow Taurscribe in System Settings → Privacy & Security → Screen & System Audio Recording, then quit and reopen Taurscribe. Microphone access alone cannot record the other participants.' };
    }
    return null;
}

/** A recording click may request undecided permissions; a denial is never
 * treated as success. Late permission answers cannot start a cancelled session. */
export async function ensureMeetingPermissions(isCurrent: () => boolean): Promise<void> {
    let p = await invoke<MeetingPermissions>('check_meeting_permissions');
    if (!isCurrent()) return;
    if (p?.microphone === 'undetermined') {
        await invoke('request_microphone_permission');
        if (!isCurrent()) return;
        p = await invoke<MeetingPermissions>('check_meeting_permissions');
    }
    if (p?.microphone === 'granted' && p.system_audio === 'undetermined') {
        await invoke('request_system_audio_permission');
        for (let attempt = 0; attempt < 20 && isCurrent(); attempt++) {
            p = await invoke<MeetingPermissions>('check_meeting_permissions');
            if (p.system_audio !== 'undetermined') break;
            await new Promise(resolve => setTimeout(resolve, 500));
        }
    }
    if (!isCurrent()) return;
    const problem = meetingPermissionProblem(p);
    if (problem) throw problem;
}

export function permissionNotice(error: { code?: string; message?: string }, fallbackTitle: string): SessionNotice {
    const code = error.code ?? 'unknown';
    const systemAudio = code === 'system_audio_permission_denied';
    const microphone = code === 'mic_permission_denied';
    return {
        level: 'error', code, title: systemAudio ? 'Meeting audio permission needed' : fallbackTitle,
        message: error.message ?? fallbackTitle, sticky: true,
        actions: systemAudio || microphone ? [{
            id: systemAudio ? 'system-audio-settings' : 'microphone-settings',
            label: systemAudio ? 'Open Audio Recording Settings' : 'Open Microphone Settings',
            onClick: () => { void invoke(systemAudio ? 'open_system_audio_settings' : 'open_microphone_settings').catch(() => {}); },
        }] : undefined,
    };
}
