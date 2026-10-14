import { beforeEach, expect, mock, test } from 'bun:test';
import { renderHook } from './hookHarness';

let permissions: any;
let failCheck = false;
const calls: string[] = [];
const realCore = { ...(await import('@tauri-apps/api/core')) };
mock.module('@tauri-apps/api/core', () => ({ ...realCore, invoke: async (command: string) => {
    calls.push(command);
    if (command === 'get_platform') return 'macos';
    if (command === 'check_meeting_permissions') {
        if (failCheck) throw new Error('Unavailable');
        return permissions;
    }
    return null;
} }));
const { MeetingAudioPermissions } = await import('../../src/components/settings/MeetingAudioPermissions');
const nodes = (root: any): any[] => !root || typeof root !== 'object' ? [] : Array.isArray(root) ? root.flatMap(nodes) : [root, ...nodes(root.props?.children)];
const texts = (root: any): string => root == null ? '' : typeof root !== 'object' ? String(root) : Array.isArray(root) ? root.map(texts).join(' ') : texts(root.props?.children);
beforeEach(() => { permissions = { microphone: 'granted', system_audio: 'granted' }; failCheck = false; calls.length = 0; });

test('a microphone grant alone is not Ready and denied audio opens its own settings', async () => {
    permissions.system_audio = 'denied';
    const h = renderHook(MeetingAudioPermissions); await h.settle();
    expect(texts(h.handle.current)).toContain('Needs attention');
    expect(texts(h.handle.current)).toContain('Call audio:  Blocked');
    await nodes(h.handle.current).find(n => n.type === 'button' && texts(n) === 'Enable meeting audio').props.onClick();
    expect(calls).toContain('open_system_audio_settings');
    expect(calls).not.toContain('request_microphone_permission');
});

test('a failed recheck cannot keep displaying an old Ready grant', async () => {
    const h = renderHook(MeetingAudioPermissions); await h.settle();
    expect(texts(h.handle.current)).toContain('Ready');
    failCheck = true;
    nodes(h.handle.current).find(n => n.type === 'button' && texts(n) === 'Check again').props.onClick();
    await h.settle();
    expect(texts(h.handle.current)).not.toContain('Ready');
    expect(texts(h.handle.current)).toContain('Could not verify audio permissions');
});
