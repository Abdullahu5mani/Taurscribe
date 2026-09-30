import { invoke } from '@tauri-apps/api/core';
import { Store } from '@tauri-apps/plugin-store';

export type RecordingMode = 'hold' | 'toggle';
export interface HotkeyBinding { keys: string[]; mode: RecordingMode; }

export const isMac = navigator.platform.toLowerCase().includes('mac');
export const isLinux = navigator.platform.toLowerCase().includes('linux');

export const KEY_LABELS: Record<string, string> = {
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

export const DEFAULT_BINDING: HotkeyBinding = {
    keys: isMac ? ['ControlLeft', 'AltLeft'] : ['ControlLeft', 'MetaLeft'],
    mode: 'hold',
};

/** Dispatched on `window` after Settings saves a new hotkey or mode. */
export const HOTKEY_CHANGED_EVENT = 'taurscribe:hotkey-changed';

/** The saved hotkey, else the one the backend is using, else the default. */
export async function loadHotkeyBinding(): Promise<HotkeyBinding> {
    try {
        const store = await Store.load('settings.json');
        const saved = await store.get<Partial<HotkeyBinding>>('hotkey_binding');
        if (saved?.keys?.length) return { keys: saved.keys, mode: saved.mode ?? 'hold' };
    } catch { /* fall through */ }
    const fromRust = await invoke<HotkeyBinding>('get_hotkey').catch(() => null);
    if (fromRust?.keys?.length) return { keys: fromRust.keys, mode: fromRust.mode ?? 'hold' };
    return DEFAULT_BINDING;
}
