import { useState, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emitTo } from "@tauri-apps/api/event";
import type { ModelInfo, ParakeetModelInfo } from "./useModels";
import type { ASREngine } from "./useEngineSwitch";

interface UseRecordingParams {
    activeEngineRef: React.RefObject<ASREngine>;
    models: ModelInfo[];
    parakeetModels: ParakeetModelInfo[];
    currentModel: string | null;
    currentParakeetModel: string | null;
    setCurrentModel: (id: string) => void;
    setLoadedEngine: (engine: ASREngine) => void;
    enableGrammarLMRef: React.RefObject<boolean>;
    enableSpellCheckRef: React.RefObject<boolean>;
    enableDenoiseRef: React.RefObject<boolean>;
    enableOverlayRef: React.RefObject<boolean>;
    transcriptionStyleRef: React.MutableRefObject<string>;
    setHeaderStatus: (msg: string, dur?: number, isProcessing?: boolean) => void;
    setTrayState: (state: "ready" | "recording" | "processing") => Promise<void>;
    setIsSettingsOpen: (open: boolean) => void;
    playStart?: () => void;
    playPaste?: () => void;
    playError?: () => void;
}

const MIN_RECORDING_MS = 1500;

/**
 * Manages recording state and the start/stop recording handlers,
 * including post-processing (spell check, grammar LM).
 */
export function useRecording({
    activeEngineRef,
    models,
    parakeetModels,
    currentModel,
    currentParakeetModel,
    setCurrentModel,
    setLoadedEngine,
    enableGrammarLMRef,
    enableSpellCheckRef,
    enableDenoiseRef,
    enableOverlayRef,
    transcriptionStyleRef,
    setHeaderStatus,
    setTrayState,
    setIsSettingsOpen,
    playStart,
    playPaste,
    playError,
}: UseRecordingParams) {
    const [isRecording, setIsRecording] = useState(false);
    const [isProcessingTranscript, setIsProcessingTranscript] = useState(false);
    const [isCorrecting, setIsCorrecting] = useState(false);
    const [liveTranscript, setLiveTranscript] = useState("");
    const [latestLatency, setLatestLatency] = useState<number | null>(null);

    const isRecordingRef = useRef(false);
    const recordingStartTimeRef = useRef(0);
    const hotkeySessionRef = useRef(false);

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

        try {
            await setTrayState("recording");
            setLiveTranscript("");
            setLatestLatency(null);
            const res = await invoke("start_recording", { denoise: enableDenoiseRef.current });
            setHeaderStatus(res as string);
            recordingStartTimeRef.current = Date.now();
            setIsRecording(true);
            isRecordingRef.current = true;
            if (fromHotkey) {
                await invoke("show_overlay").catch(() => { });
                await new Promise(r => setTimeout(r, 50));
                emitTo("overlay", "overlay-state", { phase: "recording" }).catch(() => { });
            }
            playStart?.();
        } catch (e) {
            console.error("Start recording failed:", e);
            setHeaderStatus("Error: " + e, 5000);
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
        console.log("[STOP] handleStopRecording called. GrammarLM:", enableGrammarLMRef.current, "SpellCheck:", enableSpellCheckRef.current);
        setIsRecording(false);
        isRecordingRef.current = false;
        setIsProcessingTranscript(true);
        if (isOverlay) emitTo("overlay", "overlay-state", { phase: "transcribing" }).catch(() => { });

        try {
            await setTrayState("processing");
            if (currentEngine === "whisper") setHeaderStatus("Processing transcription...", 60_000, true);

            let finalTrans = await invoke("stop_recording") as string;

            const recordingDurationMs = Date.now() - recordingStartTimeRef.current;
            if (recordingDurationMs < MIN_RECORDING_MS) {
                setHeaderStatus("Recording too short — hold for at least 1.5 seconds", 5000);
                playError?.();
                setLiveTranscript("");
                setIsProcessingTranscript(false);
                await setTrayState("ready");
                if (isOverlay) {
                    invoke("hide_overlay").catch(() => { });
                    emitTo("overlay", "overlay-state", { phase: "hidden" }).catch(() => { });
                }
                return;
            }

            // Treat empty / silence / "Recording saved." as no speech detected
            const trimmed = finalTrans.trim();
            if (!trimmed || trimmed === "[silence]" || trimmed === "Recording saved.") {
                setHeaderStatus("No speech detected", 4000);
                playError?.();
                setLiveTranscript("");
                setIsProcessingTranscript(false);
                await setTrayState("ready");
                if (isOverlay) {
                    invoke("hide_overlay").catch(() => { });
                    emitTo("overlay", "overlay-state", { phase: "hidden" }).catch(() => { });
                }
                return;
            }

            if (enableSpellCheckRef.current) {
                setIsCorrecting(true);
                if (isOverlay) emitTo("overlay", "overlay-state", { phase: "correcting" }).catch(() => { });
                setHeaderStatus("Fixing spelling...", 60_000, true);
                try {
                    finalTrans = await invoke("correct_spelling", { text: finalTrans });
                    if (!enableGrammarLMRef.current) {
                        setHeaderStatus("Spelling corrected!");
                    }
                } catch (e) {
                    setHeaderStatus("Spell check failed: " + e, 5000);
                }
            }

            if (enableGrammarLMRef.current) {
                if (isOverlay) emitTo("overlay", "overlay-state", { phase: "correcting" }).catch(() => { });
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

            const totalMs = Date.now() - processingStartMs;
            setLatestLatency(totalMs);
            setLiveTranscript(finalTrans);

            await invoke("type_text", { text: finalTrans });
            playPaste?.();

            if (isOverlay) {
                const preview = finalTrans.slice(0, 60) + (finalTrans.length > 60 ? "…" : "");
                await emitTo("overlay", "overlay-state", { phase: "done", text: preview });
                setTimeout(() => {
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
