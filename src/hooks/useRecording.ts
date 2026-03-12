import { useState, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emitTo } from "@tauri-apps/api/event";
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
        hotkeySessionRef.current = fromHotkey && enableOverlayRef.current;
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
                emitTo("overlay", "overlay-state", { phase: "recording" }).catch(() => { });
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
        const isOverlay = hotkeySessionRef.current;
        console.log("[STOP] handleStopRecording called. GrammarLM:", enableGrammarLMRef.current);
        setIsRecording(false);
        isRecordingRef.current = false;
        setIsProcessingTranscript(true);
        if (isOverlay) {
            invoke("show_overlay").catch(() => { }); // Re-show in case it was hidden
            emitTo("overlay", "overlay-state", { phase: "transcribing" }).catch(() => { });
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
                if (isOverlay) {
                    invoke("show_overlay").catch(() => { });
                    await emitTo("overlay", "overlay-state", { phase: "too_short" }).catch(() => { });
                    await new Promise(resolve => setTimeout(resolve, 1000));
                    invoke("hide_overlay").catch(() => { });
                    emitTo("overlay", "overlay-state", { phase: "hidden" }).catch(() => { });
                }
                return;
            }


            if (enableGrammarLMRef.current) {
                if (isOverlay) {
                    invoke("show_overlay").catch(() => { });
                    emitTo("overlay", "overlay-state", { phase: "correcting" }).catch(() => { });
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

            await invoke("type_text", { text: finalTrans });

            if (muteBackgroundAudioRef.current) {
                await invoke("unmute_system_audio").catch(e => console.warn("unmute_system_audio failed:", e));
            }

            // Clear the "Processing transcription..." status that was set at recording stop.
            // The spell-check / grammar branches already set their own completion messages,
            // so only clear here when neither ran (plain Whisper with no post-processing).
            if (!enableGrammarLMRef.current) {
                setHeaderStatus("Done!", 900);
            }

            // Persist a lightweight history entry for this transcription.
            // We record which engine was used, how long the recording was,
            // and whether the grammar LLM was enabled for this run.
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

            playPaste?.();

            if (isOverlay) {
                invoke("show_overlay").catch(() => { });
                const preview = finalTrans.slice(0, 60) + (finalTrans.length > 60 ? "…" : "");
                await emitTo("overlay", "overlay-state", { phase: "done", text: preview, ms: totalMs });
                overlayHideTimerRef.current = setTimeout(() => {
                    overlayHideTimerRef.current = null;
                    invoke("hide_overlay").catch(() => { });
                    emitTo("overlay", "overlay-state", { phase: "hidden" }).catch(() => { });
                }, 1500);
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
                emitTo("overlay", "overlay-state", { phase: "hidden" }).catch(() => { });
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
