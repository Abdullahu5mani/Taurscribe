import { useEffect, useRef, type MutableRefObject } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import type { ASREngine } from "./useEngineSwitch";

interface UseHotkeyListenersParams {
    // Recording state refs
    isRecordingRef: React.RefObject<boolean>;
    isLoadingRef: React.RefObject<boolean>;
    activeEngineRef: React.RefObject<ASREngine>;
    /** True while FileTranscriptionPanel is transcribing — mirrors Record button BUSY. */
    isFileTranscribingRef: React.RefObject<boolean>;
    asrModelCountsRef: MutableRefObject<{
        whisper: number;
        parakeet: number;
        cohere: number;
    }>;

    // Stable handler refs (always point to latest closure)
    handleStartRecordingRef: React.RefObject<(fromHotkey?: boolean) => Promise<void>>;
    handleStopRecordingRef: React.RefObject<() => Promise<void>>;
    handlePauseRecordingRef: React.RefObject<() => Promise<void>>;
    handleResumeRecordingRef: React.RefObject<() => Promise<void>>;
    handleCancelRecordingRef: React.RefObject<() => Promise<void>>;
    handleTranscriptionChunkRef: React.RefObject<(text: string) => void>;
    playErrorRef: React.RefObject<() => void>;
    setHeaderStatusRef: React.RefObject<(msg: string, dur?: number) => void>;
    triggerNoModelAttentionRef: React.RefObject<() => void>;

    // Direct setters needed by some events
    setLoadedEngine: (engine: ASREngine | null) => void;
    silenceTimerRef: React.MutableRefObject<ReturnType<typeof setTimeout> | null>;
    setShowSilenceWarning: (v: boolean) => void;
    refreshMacPermissions: () => Promise<void>;
}

// Single window for start/stop: tight enough for quick dictation, loose enough to drop key chatter.
const HOTKEY_DEBOUNCE_MS = 350;

/**
 * Registers and manages all Tauri event listeners related to hotkeys and
 * recording lifecycle. All handlers are read via refs so this effect only
 * runs once (empty deps), avoiding listener churn on every render.
 */
export function useHotkeyListeners({
    isRecordingRef,
    isLoadingRef,
    activeEngineRef,
    isFileTranscribingRef,
    asrModelCountsRef,
    handleStartRecordingRef,
    handleStopRecordingRef,
    handlePauseRecordingRef,
    handleResumeRecordingRef,
    handleCancelRecordingRef,
    handleTranscriptionChunkRef,
    playErrorRef,
    setHeaderStatusRef,
    triggerNoModelAttentionRef,
    setLoadedEngine,
    silenceTimerRef,
    setShowSilenceWarning,
    refreshMacPermissions,
}: UseHotkeyListenersParams) {
    // Debounce / sequencing guards live inside the hook, not App.tsx
    const startingRecordingRef = useRef(false);
    const pendingStopRef = useRef(false);
    const stopInProgressRef = useRef(false);
    const lastStartTime = useRef(0);
    const lastStopTime = useRef(0);
    const overlayFeedbackHideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    useEffect(() => {
        let active = true;
        let unlistenStart: (() => void) | undefined;
        let unlistenStop: (() => void) | undefined;
        let unlistenChunk: (() => void) | undefined;
        let unlistenAccessibility: (() => void) | undefined;
        let unlistenAudioFallback: (() => void) | undefined;
        let unlistenAudioDisconnect: (() => void) | undefined;
        let unlistenOverlayAction: (() => void) | undefined;
        let unlistenModelUnloaded: (() => void) | undefined;
        let unlistenAudioLevel: (() => void) | undefined;

        const SILENCE_THRESHOLD = 0.02;
        const SILENCE_DELAY_MS = 3000;

        const clearOverlayFeedbackHide = () => {
            if (overlayFeedbackHideTimerRef.current !== null) {
                clearTimeout(overlayFeedbackHideTimerRef.current);
                overlayFeedbackHideTimerRef.current = null;
            }
        };

        const showTimedOverlayFeedback = (phase: "model_loading" | "no_model") => {
            clearOverlayFeedbackHide();
            invoke("show_overlay").catch(() => {});
            invoke("set_overlay_state", {
                phase,
                engine: activeEngineRef.current,
            }).catch(() => {});
            overlayFeedbackHideTimerRef.current = setTimeout(() => {
                overlayFeedbackHideTimerRef.current = null;
                invoke("hide_overlay").catch(() => {});
                invoke("set_overlay_state", {
                    phase: "hidden",
                    engine: activeEngineRef.current,
                }).catch(() => {});
            }, 2500);
        };

        const setup = async () => {
            const unsub1 = await listen("hotkey-start-recording", async () => {
                const now = Date.now();
                if (now - lastStartTime.current < HOTKEY_DEBOUNCE_MS) return;
                lastStartTime.current = now;

                clearOverlayFeedbackHide();

                // Don't start if already recording, starting, or processing a previous stop
                if (
                    isRecordingRef.current ||
                    startingRecordingRef.current ||
                    stopInProgressRef.current
                ) return;

                // Block hotkey while model is loading
                if (isLoadingRef.current) {
                    playErrorRef.current?.();
                    showTimedOverlayFeedback("model_loading");
                    return;
                }

                if (isFileTranscribingRef.current) {
                    playErrorRef.current?.();
                    setHeaderStatusRef.current?.(
                        "Cannot record while a file is being transcribed",
                        4000
                    );
                    return;
                }

                const eng = activeEngineRef.current;
                const counts = asrModelCountsRef.current;
                const noModelsForEngine =
                    (eng === "whisper" && counts.whisper === 0) ||
                    (eng === "parakeet" && counts.parakeet === 0) ||
                    (eng === "cohere" && counts.cohere === 0);
                if (noModelsForEngine) {
                    playErrorRef.current?.();
                    triggerNoModelAttentionRef.current?.();
                    showTimedOverlayFeedback("no_model");
                    return;
                }

                startingRecordingRef.current = true;
                pendingStopRef.current = false;
                await handleStartRecordingRef.current?.(true);
                startingRecordingRef.current = false;
                if (pendingStopRef.current) {
                    pendingStopRef.current = false;
                    setTimeout(async () => {
                        await handleStopRecordingRef.current?.();
                    }, 250);
                }
            });

            const unsub2 = await listen("hotkey-stop-recording", async () => {
                if (startingRecordingRef.current) {
                    pendingStopRef.current = true;
                    return;
                }
                if (stopInProgressRef.current) return;
                if (!isRecordingRef.current) return;

                stopInProgressRef.current = true;
                const now = Date.now();
                if (now - lastStopTime.current < HOTKEY_DEBOUNCE_MS) {
                    stopInProgressRef.current = false;
                    return;
                }
                lastStopTime.current = now;

                try {
                    await handleStopRecordingRef.current?.();
                } finally {
                    stopInProgressRef.current = false;
                }
            });

            const unsub3 = await listen<{ text: string }>("transcription-chunk", (event) => {
                handleTranscriptionChunkRef.current?.(event.payload.text);
            });

            // Re-check macOS permissions if the backend notices the hotkey listener
            // cannot receive events.
            const unsub4 = await listen("accessibility-missing", () => {
                void refreshMacPermissions();
            });

            const unsub5 = await listen("audio-fallback", (event) => {
                const deviceName = event.payload as string;
                setHeaderStatusRef.current?.(`Mic lost, using fallback: ${deviceName}`, 6000);
            });

            const unsub6 = await listen("audio-disconnected", (_event) => {
                setHeaderStatusRef.current?.(
                    "Microphone disconnected! Recording stopped.",
                    6000
                );
                if (isRecordingRef.current && !stopInProgressRef.current) {
                    stopInProgressRef.current = true;
                    handleStopRecordingRef.current?.().finally(() => {
                        stopInProgressRef.current = false;
                    });
                }
            });

            const unsub7 = await listen<string>("overlay-action", async (event) => {
                const action = String(event.payload);
                if (action === "pause") {
                    await handlePauseRecordingRef.current?.();
                    return;
                }
                if (action === "resume") {
                    await handleResumeRecordingRef.current?.();
                    return;
                }
                if (action === "cancel") {
                    if (stopInProgressRef.current) return;
                    stopInProgressRef.current = true;
                    try {
                        await handleCancelRecordingRef.current?.();
                    } finally {
                        stopInProgressRef.current = false;
                    }
                }
            });

            const unsub8 = await listen("model-unloaded", () => {
                setLoadedEngine(null);
                setHeaderStatusRef.current?.(
                    "Model unloaded — VRAM freed. Next dictation will load the model again.",
                    5000
                );
            });

            // Silence detection: show a hint when audio level stays near-zero for 3 s
            const unsub9 = await listen<number>("audio-level", (event) => {
                if (!isRecordingRef.current) return;
                const level = event.payload;
                if (level > SILENCE_THRESHOLD) {
                    if (silenceTimerRef.current) {
                        clearTimeout(silenceTimerRef.current);
                        silenceTimerRef.current = null;
                    }
                    setShowSilenceWarning(false);
                } else {
                    if (!silenceTimerRef.current) {
                        silenceTimerRef.current = setTimeout(() => {
                            if (isRecordingRef.current) setShowSilenceWarning(true);
                            silenceTimerRef.current = null;
                        }, SILENCE_DELAY_MS);
                    }
                }
            });

            if (active) {
                unlistenStart = unsub1;
                unlistenStop = unsub2;
                unlistenChunk = unsub3;
                unlistenAccessibility = unsub4;
                unlistenAudioFallback = unsub5;
                unlistenAudioDisconnect = unsub6;
                unlistenOverlayAction = unsub7;
                unlistenModelUnloaded = unsub8;
                unlistenAudioLevel = unsub9;
            } else {
                unsub1(); unsub2(); unsub3(); unsub4();
                unsub5(); unsub6(); unsub7(); unsub8(); unsub9();
            }
        };

        setup();
        return () => {
            active = false;
            clearOverlayFeedbackHide();
            unlistenStart?.();
            unlistenStop?.();
            unlistenChunk?.();
            unlistenAccessibility?.();
            unlistenAudioFallback?.();
            unlistenAudioDisconnect?.();
            unlistenOverlayAction?.();
            unlistenModelUnloaded?.();
            unlistenAudioLevel?.();
        };
    }, []); // eslint-disable-line react-hooks/exhaustive-deps
}
