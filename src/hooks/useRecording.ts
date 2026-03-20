import { useState, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ModelInfo, ParakeetModelInfo, GraniteSpeechModelInfo } from "./useModels";
import type { ASREngine } from "./useEngineSwitch";
import { applyDictionary, applySnippets } from "./usePersonalization";
import type { DictEntry, SnippetEntry } from "./usePersonalization";

interface UseRecordingParams {
    activeEngineRef: React.RefObject<ASREngine>;
    models: ModelInfo[];
    parakeetModels: ParakeetModelInfo[];
    graniteModels: GraniteSpeechModelInfo[];
    currentModel: string | null;
    currentParakeetModel: string | null;
    setCurrentModel: (id: string) => void;
    setLoadedEngine: (engine: ASREngine) => void;
    enableGrammarLMRef: React.RefObject<boolean>;
    enableDenoiseRef: React.RefObject<boolean>;
    enableOverlayRef: React.RefObject<boolean>;
    muteBackgroundAudioRef: React.RefObject<boolean>;
    transcriptionStyleRef: React.MutableRefObject<string>;
    setHeaderStatus: (msg: string, dur?: number, isProcessing?: boolean) => void;
    setTrayState: (state: "ready" | "recording" | "processing") => Promise<void>;
    setIsSettingsOpen: (open: boolean) => void;
    playStart?: () => void;
    playPaste?: () => void;
    playError?: () => void;
    dictionaryRef: React.RefObject<DictEntry[]>;
    snippetsRef: React.RefObject<SnippetEntry[]>;
    /** Called after each successful save_transcript_history — lets the parent refresh the history UI. */
    onHistorySaved?: () => void;
}

const MIN_RECORDING_MS = 1500;

/**
 * Manages recording state and the start/stop recording handlers,
 * including post-processing (grammar LM).
 */
export function useRecording({
    activeEngineRef,
    models,
    parakeetModels,
    graniteModels,
    currentModel,
    currentParakeetModel,
    setCurrentModel,
    setLoadedEngine,
    enableGrammarLMRef,
    enableDenoiseRef,
    enableOverlayRef,
    muteBackgroundAudioRef,
    transcriptionStyleRef,
    setHeaderStatus,
    setTrayState,
    setIsSettingsOpen,
    playStart,
    playPaste,
    playError,
    dictionaryRef,
    snippetsRef,
    onHistorySaved,
}: UseRecordingParams) {
    const [isRecording, setIsRecording] = useState(false);
    const [isProcessingTranscript, setIsProcessingTranscript] = useState(false);
    const [isCorrecting, setIsCorrecting] = useState(false);
    const [liveTranscript, setLiveTranscript] = useState("");
    const [latestLatency, setLatestLatency] = useState<number | null>(null);

    const isRecordingRef = useRef(false);
    const recordingStartTimeRef = useRef(0);
    const hotkeySessionRef = useRef(false);
    const overlayHideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    const handleStartRecording = async (fromHotkey = false) => {
        hotkeySessionRef.current = fromHotkey; // tracks hotkey session independent of overlay toggle
        const currentEngine = activeEngineRef.current;

        if (currentEngine === "whisper") {
            if (models.length === 0) {
                setHeaderStatus("No Whisper models installed! Please download one.", 5000);
                setIsSettingsOpen(true);
                return;
            }
            if (!currentModel) {
                setHeaderStatus("Auto-selecting model...", 60_000);
                const first = models[0].id;
                setCurrentModel(first);
                try {
                    await invoke("switch_model", { modelId: first });
                    setLoadedEngine("whisper");
                    setHeaderStatus("Model selected: " + first);
                } catch (e) {
                    setHeaderStatus("Failed to auto-select model: " + e, 5000);
                    return;
                }
            }
        }

        if (currentEngine === "parakeet") {
            if (parakeetModels.length === 0) {
                setHeaderStatus("No Parakeet models installed!", 5000);
                setIsSettingsOpen(true);
                return;
            }
            try {
                const pStatus = await invoke("get_parakeet_status") as { loaded: boolean };
                if (!pStatus.loaded) {
                    setHeaderStatus("Loading Parakeet...", 60_000);
                    const targetModel = currentParakeetModel || parakeetModels[0].id;
                    await invoke("init_parakeet", { modelId: targetModel });
                    setLoadedEngine("parakeet");
                    setHeaderStatus("Parakeet model loaded");
                }
            } catch (e) {
                setHeaderStatus("Failed to initialize Parakeet: " + e, 5000);
                return;
            }
        }

        if (currentEngine === "granite_speech") {
            if (graniteModels.length === 0) {
                setHeaderStatus("No Granite Speech model installed! Download it from Settings.", 5000);
                setIsSettingsOpen(true);
                return;
            }
            try {
                const gStatus = await invoke("get_granite_speech_status") as { loaded: boolean };
                if (!gStatus.loaded) {
                    setHeaderStatus("Loading Granite Speech...", 60_000);
                    await invoke("init_granite_speech", {});
                    setLoadedEngine("granite_speech");
                    setHeaderStatus("Granite Speech loaded");
                }
            } catch (e) {
                setHeaderStatus("Failed to initialize Granite Speech: " + e, 5000);
                return;
            }
        }

        try {
            await setTrayState("recording");
            setLiveTranscript("");
            setLatestLatency(null);
            // Play start sound before muting so the app's own audio isn't silenced.
            playStart?.();
            if (muteBackgroundAudioRef.current) {
                await invoke("mute_system_audio").catch(e => console.warn("mute_system_audio failed:", e));
            }
            const res = await invoke("start_recording", { denoise: enableDenoiseRef.current });
            setHeaderStatus(res as string);
            recordingStartTimeRef.current = Date.now();
            setIsRecording(true);
            isRecordingRef.current = true;
            if (fromHotkey) {
                // Cancel any pending hide from the previous session
                if (overlayHideTimerRef.current !== null) {
                    clearTimeout(overlayHideTimerRef.current);
                    overlayHideTimerRef.current = null;
                }
                if (enableOverlayRef.current) {
                    // Show overlay reliably: retry once if needed, then emit state
                    for (let attempt = 0; attempt < 2; attempt++) {
                        try {
                            await invoke("show_overlay");
                            break;
                        } catch {
                            if (attempt === 1) console.warn("show_overlay failed after retry");
                        }
                    }
                    await new Promise(r => setTimeout(r, 80));
                    invoke("set_overlay_state", { phase: "recording" }).catch(() => { });
                }
            }
        } catch (e) {
            const errStr = String(e);
            if (errStr.includes("Already recording")) {
                setHeaderStatus("Recording already in progress", 2000);
                return;
            }
            console.error("Start recording failed:", e);
            setHeaderStatus("Error: " + e, 5000);
            if (muteBackgroundAudioRef.current) {
                await invoke("unmute_system_audio").catch(() => {});
            }
            playError?.();
            await setTrayState("ready");
            setIsRecording(false);
            isRecordingRef.current = false;
            if (fromHotkey) invoke("hide_overlay").catch(() => { });
        }
    };

    const handleStopRecording = async () => {
        const currentEngine = activeEngineRef.current;
        const processingStartMs = Date.now();
        const isOverlay = hotkeySessionRef.current;              // true for any hotkey session
        const showOverlay = isOverlay && enableOverlayRef.current; // true only when overlay is enabled
        console.log("[STOP] handleStopRecording called. GrammarLM:", enableGrammarLMRef.current);
        setIsRecording(false);
        isRecordingRef.current = false;
        setIsProcessingTranscript(true);
        if (showOverlay) {
            invoke("show_overlay").catch(() => { }); // Re-show in case it was hidden
            invoke("set_overlay_state", { phase: "transcribing" }).catch(() => { });
        }

        try {
            await setTrayState("processing");
            if (currentEngine === "whisper") setHeaderStatus("Processing transcription...", 15_000, true);

            let finalTrans = await invoke("stop_recording") as string;

            // Apply custom dictionary substitutions (before grammar LLM)
            finalTrans = applyDictionary(finalTrans, dictionaryRef.current ?? []);

            const recordingDurationMs = Date.now() - recordingStartTimeRef.current;
            if (recordingDurationMs < MIN_RECORDING_MS) {
                setHeaderStatus("Recording too short — try at least 1.5 seconds", 5000);
                if (muteBackgroundAudioRef.current) {
                    await invoke("unmute_system_audio").catch(() => {});
                }
                playError?.();
                setLiveTranscript("");
                setIsProcessingTranscript(false);
                await setTrayState("ready");
                if (showOverlay) {
                    invoke("show_overlay").catch(() => { });
                    await invoke("set_overlay_state", { phase: "too_short" }).catch(() => { });
                    await new Promise(resolve => setTimeout(resolve, 1000));
                }
                if (isOverlay) {
                    invoke("hide_overlay").catch(() => { });
                    invoke("set_overlay_state", { phase: "hidden" }).catch(() => { });
                }
                return;
            }


            if (enableGrammarLMRef.current) {
                if (showOverlay) {
                    invoke("show_overlay").catch(() => { });
                    invoke("set_overlay_state", { phase: "correcting" }).catch(() => { });
                }
                setHeaderStatus("Correcting grammar...", 60_000, true);
                try {
                    const activeStyle = transcriptionStyleRef.current;
                    finalTrans = await invoke("correct_text", { text: finalTrans, style: activeStyle });
                    setHeaderStatus("Transcribed & Corrected!");
                } catch (e) {
                    setHeaderStatus("Grammar correction failed: " + e, 5000);
                }
            }

            setIsCorrecting(false);

            // Apply text snippets last (after grammar LLM so expansions aren't mangled)
            finalTrans = applySnippets(finalTrans, snippetsRef.current ?? []);

            const totalMs = Date.now() - processingStartMs;
            setLatestLatency(totalMs);
            setLiveTranscript(finalTrans);

            // Capture paste result without blocking history/unmute — a failed
            // paste means the transcript is still shown in the UI, just not
            // inserted into the target app.
            let pasteError: string | null = null;
            try {
                await invoke("type_text", { text: finalTrans });
            } catch (e) {
                pasteError = String(e);
                console.warn("[INSERT] type_text failed:", pasteError);
            }

            if (muteBackgroundAudioRef.current) {
                await invoke("unmute_system_audio").catch(e => console.warn("unmute_system_audio failed:", e));
            }

            // Persist a lightweight history entry regardless of paste outcome —
            // the transcript was generated successfully and is visible in the UI.
            try {
                await invoke("save_transcript_history", {
                    transcript: finalTrans,
                    engine: currentEngine,
                    durationMs: recordingDurationMs,
                    grammarLlmUsed: enableGrammarLMRef.current,
                    processingTimeMs: totalMs,
                });
                onHistorySaved?.();
            } catch (e) {
                console.warn("Failed to save transcript history:", e);
            }

            if (pasteError) {
                let headerMsg: string;
                if (pasteError.includes("secure_input")) {
                    headerMsg = "Couldn't paste — a password field has locked keyboard input";
                } else if (pasteError.includes("console")) {
                    headerMsg = "Couldn't paste — right-click → Paste in console windows";
                } else {
                    headerMsg = "Couldn't paste — transcript is shown above";
                }
                setHeaderStatus(headerMsg, 5000);
                playError?.();
                if (showOverlay) {
                    invoke("show_overlay").catch(() => { });
                    await invoke("set_overlay_state", { phase: "paste_failed" }).catch(() => { });
                }
                if (isOverlay) {
                    overlayHideTimerRef.current = setTimeout(() => {
                        overlayHideTimerRef.current = null;
                        invoke("hide_overlay").catch(() => { });
                        invoke("set_overlay_state", { phase: "hidden" }).catch(() => { });
                    }, 2000);
                }
            } else {
                // Clear the "Processing transcription..." status only when no
                // grammar LM ran (the grammar branch already set its own message).
                if (!enableGrammarLMRef.current) {
                    setHeaderStatus("Done!", 900);
                }
                playPaste?.();
                if (showOverlay) {
                    invoke("show_overlay").catch(() => { });
                    const preview = finalTrans.slice(0, 60) + (finalTrans.length > 60 ? "…" : "");
                    await invoke("set_overlay_state", { phase: "done", text: preview, ms: totalMs }).catch(() => { });
                }
                if (isOverlay) {
                    overlayHideTimerRef.current = setTimeout(() => {
                        overlayHideTimerRef.current = null;
                        invoke("hide_overlay").catch(() => { });
                        invoke("set_overlay_state", { phase: "hidden" }).catch(() => { });
                    }, 1500);
                }
            }

            setIsProcessingTranscript(false);
            await setTrayState("ready");
        } catch (e) {
            console.error("Stop recording failed:", e);
            const errStr = String(e);
            if (!errStr.includes("Not recording")) {
                setHeaderStatus("Error: " + e, 5000);
                if (muteBackgroundAudioRef.current) {
                    await invoke("unmute_system_audio").catch(() => {});
                }
                playError?.();
            }
            isRecordingRef.current = false;
            setIsCorrecting(false);
            setIsProcessingTranscript(false);
            await setTrayState("ready");
            if (isOverlay) {
                invoke("hide_overlay").catch(() => { });
                invoke("set_overlay_state", { phase: "hidden" }).catch(() => { });
            }
        } finally {
            if (muteBackgroundAudioRef.current) {
                invoke("unmute_system_audio").catch(e => console.warn("unmute_system_audio failed:", e));
            }
        }
        hotkeySessionRef.current = false;
    };

    return {
        isRecording,
        isRecordingRef,
        isProcessingTranscript,
        isCorrecting,
        liveTranscript,
        setLiveTranscript,
        latestLatency,
        handleStartRecording,
        handleStopRecording,
    };
}
