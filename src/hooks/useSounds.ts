import { useState, useRef, useEffect, useCallback } from 'react';
import { Store } from '@tauri-apps/plugin-store';
import recStartUrl from '../assets/sounds/recStart.wav';
import pasteUrl from '../assets/sounds/paste.wav';
import errorUrl from '../assets/sounds/error.wav';

export function useSounds() {
    const [volume, setVolumeState] = useState(0.7);
    const [muted, setMutedState] = useState(false);

    const volumeRef = useRef(0.7);
    const mutedRef = useRef(false);

    const recStartAudio = useRef<HTMLAudioElement | null>(null);
    const pasteAudio = useRef<HTMLAudioElement | null>(null);
    const errorAudio = useRef<HTMLAudioElement | null>(null);

    // Pre-load audio elements once
    useEffect(() => {
        recStartAudio.current = new Audio(recStartUrl);
        pasteAudio.current = new Audio(pasteUrl);
        errorAudio.current = new Audio(errorUrl);
    }, []);

    // Load persisted settings
    useEffect(() => {
        Store.load('settings.json').then(store => {
            store.get<number>('sound_volume').then(v => {
                if (v != null) {
                    volumeRef.current = v;
                    setVolumeState(v);
                }
            });
            store.get<boolean>('sound_muted').then(m => {
                if (m != null) {
                    mutedRef.current = m;
                    setMutedState(m);
                }
            });
        }).catch(() => {});
    }, []);

    const setVolume = useCallback((v: number) => {
        volumeRef.current = v;
        setVolumeState(v);
        Store.load('settings.json').then(store => {
            store.set('sound_volume', v);
            store.save();
        }).catch(() => {});
    }, []);

    const setMuted = useCallback((m: boolean) => {
        mutedRef.current = m;
        setMutedState(m);
        Store.load('settings.json').then(store => {
            store.set('sound_muted', m);
            store.save();
        }).catch(() => {});
    }, []);

    const play = useCallback((audio: HTMLAudioElement | null) => {
        if (!audio || mutedRef.current) return;
        audio.currentTime = 0;
        audio.volume = volumeRef.current;
        audio.play().catch(() => {});
    }, []);

    const playStart = useCallback(() => play(recStartAudio.current), [play]);
    const playPaste = useCallback(() => play(pasteAudio.current), [play]);
    const playError = useCallback(() => play(errorAudio.current), [play]);

    return { volume, muted, setVolume, setMuted, playStart, playPaste, playError };
}
