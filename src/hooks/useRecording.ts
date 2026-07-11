import { useState, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ModelInfo, ParakeetModelInfo, CohereModelInfo } from "./useModels";
import type { ASREngine } from "./useEngineSwitch";
import { applyDictionary, applySnippets } from "./usePersonalization";
import type { DictEntry, SnippetEntry } from "./usePersonalization";
import type { CommandResult, SessionNotice } from "../types/session";

interface UseRecordingParams {
    activeEngineRef: React.RefObject<ASREngine>;
    models: ModelInfo[];
    parakeetModels: ParakeetModelInfo[];
    cohereModels: CohereModelInfo[];
    currentModel: string | null;
    currentParakeetModel: string | null;
    currentCohereModel: string | null;
    asrBackend: "gpu" | "cpu";
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
    setSessionPhase?: (phase: "idle" | "loading_model" | "recording" | "paused" | "processing" | "success" | "warning" | "error") => void;
    setSessionNotice?: (notice: SessionNotice | null) => void;
    setSessionTranscript?: (transcript: string) => void;
    setSessionLatency?: (latency: number | null) => void;
}

/** Minimum live mic time before stop; enforced in this hook only (not Rust). */
const MIN_RECORDING_MS = 300;
type OverlayPhase =
    | "recording"
    | "paused"
    | "transcribing"
    | "correcting"
    | "done"
    | "too_short"
    | "nothing_heard"
    | "paste_failed"
    | "cancelled"
    | "hidden";

const TERMINAL_OVERLAY_PHASES = new Set<OverlayPhase>([
    "done",
    "too_short",
    "nothing_heard",
    "paste_failed",
    "cancelled",
    "hidden",
]);

/**
 * Manages recording state and the start/stop recording handlers,
 * including post-processing (grammar LM).
 */
export function useRecording({
    activeEngineRef,
    models,
    parakeetModels,
    cohereModels,
    currentModel,
    currentParakeetModel,
    currentCohereModel,
    asrBackend,
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
    setSessionPhase,
    setSessionNotice,
    setSessionTranscript,
    setSessionLatency,
}: UseRecordingParams) {
    const [isRecording, setIsRecording] = useState(false);
    const [isPaused, setIsPaused] = useState(false);
    const [isProcessingTranscript, setIsProcessingTranscript] = useState(false);
    const [latestLatency, setLatestLatency] = useState<number | null>(null);

    const isRecordingRef = useRef(false);
    const isProcessingTranscriptRef = useRef(false);
    const isPausedRef = useRef(false);
    const recordingStartTimeRef = useRef(0);
    const hotkeySessionRef = useRef(false);
    const overlayHideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    const overlaySessionIdRef = useRef(0);
    const overlayTerminalRef = useRef(false);
    const overlayCommandChainRef = useRef<Promise<void>>(Promise.resolve());
    const liveTranscriptRef = useRef("");
    /** Base transcript snapshot taken when the first Cohere partial arrives for a chunk. */
    const coherePartialBaseRef = useRef("");
    /** True while streaming partials for the current Cohere VAD chunk. */
    const coherePartialActiveRef = useRef(false);
    const pausedAtRef = useRef<number | null>(null);
    const totalPausedMsRef = useRef(0);

    const clearPendingOverlayHide = () => {
        if (overlayHideTimerRef.current !== null) {
            clearTimeout(overlayHideTimerRef.current);
            overlayHideTimerRef.current = null;
        }
    };

    const getEffectiveRecordingMs = () => {
        const pauseMs = isPausedRef.current && pausedAtRef.current
            ? Date.now() - pausedAtRef.current
            : 0;
        return Date.now() - recordingStartTimeRef.current - totalPausedMsRef.current - pauseMs;
    };

    const settlePauseWindow = () => {
        if (isPausedRef.current && pausedAtRef.current) {
            totalPausedMsRef.current += Date.now() - pausedAtRef.current;
            pausedAtRef.current = null;
        }
    };

    const setProcessingTranscript = (processing: boolean) => {
        isProcessingTranscriptRef.current = processing;
        setIsProcessingTranscript(processing);
    };

    const queueOverlayCommand = (sessionId: number, command: () => Promise<void>) => {
        const queued = overlayCommandChainRef.current
            .catch(() => { })
            .then(async () => {
                if (sessionId !== overlaySessionIdRef.current) return;
                await command();
            });
        overlayCommandChainRef.current = queued.catch(() => { });
        return queued;
    };

    const emitOverlayState = async (phase: OverlayPhase, text?: string, ms?: number) => {
        if (!hotkeySessionRef.current || !enableOverlayRef.current) return;
        const isTerminal = TERMINAL_OVERLAY_PHASES.has(phase);
        if (overlayTerminalRef.current && !isTerminal) return;
        const sessionId = overlaySessionIdRef.current;
        const engine = activeEngineRef.current;
        return queueOverlayCommand(sessionId, async () => {
            await invoke("show_overlay").catch(() => { });
            await invoke("set_overlay_state", { phase, text, ms, engine }).catch(() => { });
        });
    };

    const hideOverlay = () => {
        const sessionId = overlaySessionIdRef.current;
        const engine = activeEngineRef.current;
        return queueOverlayCommand(sessionId, async () => {
            await invoke("hide_overlay").catch(() => { });
            await invoke("set_overlay_state", { phase: "hidden", engine }).catch(() => { });
        });
    };

    const resetRecordingSession = () => {
        liveTranscriptRef.current = "";
        setLatestLatency(null);
        setSessionTranscript?.("");
        setSessionLatency?.(null);
        setIsPaused(false);
        isPausedRef.current = false;
        pausedAtRef.current = null;
        totalPausedMsRef.current = 0;
    };

    const showNotice = (notice: SessionNotice | null) => {
        setSessionNotice?.(notice);
    };

    const commandErrorToNotice = (error: { code?: string; message?: string }, fallbackTitle: string): SessionNotice => ({
        level: "error",
        code: error.code ?? "unknown",
        title: fallbackTitle,
        message: error.message ?? fallbackTitle,
        sticky: true,
    });

    const handleStartRecording = async (fromHotkey = false) => {
        hotkeySessionRef.current = fromHotkey; // tracks hotkey session independent of overlay toggle
        if (fromHotkey) {
            overlaySessionIdRef.current += 1;
            overlayTerminalRef.current = false;
        }
        const currentEngine = activeEngineRef.current;

        if (currentEngine === "whisper") {
            if (models.length === 0) {
                setHeaderStatus("No Whisper models installed! Please download one.", 5000);
                showNotice({
                    level: "warning",
                    code: "model_missing",
                    title: "No Whisper model installed",
                    message: "Download a Whisper model or switch to another installed engine before recording.",
                    sticky: true,
                });
                setIsSettingsOpen(true);
                return;
            }
            const useGpu = asrBackend === "gpu";
            let targetWhisperId = (currentModel ?? "").trim();
            if (!targetWhisperId) {
                setHeaderStatus("Auto-selecting model...", 60_000);
                setSessionPhase?.("loading_model");
                targetWhisperId = models[0].id;
                setCurrentModel(targetWhisperId);
                try {
                    const result = await invoke<CommandResult<string>>("switch_model", { modelId: targetWhisperId, useGpu });
                    if (!result.ok) throw result.error ?? new Error("Failed to auto-select model");
                    setLoadedEngine("whisper");
                    setHeaderStatus("Model selected: " + targetWhisperId);
                    showNotice(null);
                } catch (e) {
                    const error = e as { code?: string; message?: string };
                    setHeaderStatus("Failed to auto-select model: " + error.message, 5000);
                    setSessionPhase?.("error");
                    showNotice(commandErrorToNotice(error, "Whisper model failed to load"));
                    return;
                }
            } else {
                try {
                    const loadedId = ((await invoke("get_current_model")) ?? null) as string | null;
                    const normalizedLoaded = (loadedId ?? "").trim();
                    if (!normalizedLoaded || normalizedLoaded !== targetWhisperId) {
                        setHeaderStatus("Loading Whisper model...", 60_000);
                        setSessionPhase?.("loading_model");
                        const result = await invoke<CommandResult<string>>("switch_model", { modelId: targetWhisperId, useGpu });
                        if (!result.ok) throw result.error ?? new Error("Failed to load Whisper model");
                        setLoadedEngine("whisper");
                        setHeaderStatus("Whisper model loaded");
                        showNotice(null);
                    }
                } catch (e) {
                    const error = e as { code?: string; message?: string };
                    setHeaderStatus("Failed to load Whisper model: " + error.message, 5000);
                    setSessionPhase?.("error");
                    showNotice(commandErrorToNotice(error, "Whisper model failed to load"));
                    return;
                }
            }
        }

        if (currentEngine === "parakeet") {
            if (parakeetModels.length === 0) {
                setHeaderStatus("No Parakeet models installed!", 5000);
                showNotice({
                    level: "warning",
                    code: "model_missing",
                    title: "Parakeet is not installed",
                    message: "Download Parakeet from Settings or switch to Whisper/Granite before recording.",
                    sticky: true,
                });
                setIsSettingsOpen(true);
                return;
            }
            try {
                const targetModel = currentParakeetModel || parakeetModels[0].id;
                const pStatus = await invoke("get_parakeet_status") as { loaded: boolean; model_id?: string | null };
                if (!pStatus.loaded || pStatus.model_id !== targetModel) {
                    setHeaderStatus("Loading Parakeet...", 60_000);
                    setSessionPhase?.("loading_model");
                    const result = await invoke<CommandResult<string>>("init_parakeet", { modelId: targetModel, useGpu: asrBackend === "gpu" });
                    if (!result.ok) throw result.error ?? new Error("Failed to initialize Parakeet");
                    setLoadedEngine("parakeet");
                    setHeaderStatus("Parakeet model loaded");
                    showNotice(null);
                }
            } catch (e) {
                const error = e as { code?: string; message?: string };
                setHeaderStatus("Failed to initialize Parakeet: " + error.message, 5000);
                setSessionPhase?.("error");
                showNotice(commandErrorToNotice(error, "Parakeet failed to load"));
                return;
            }
        }

        if (currentEngine === "granite") {
            if (cohereModels.length === 0) {
                setHeaderStatus("No Granite Speech model installed! Download it from Settings.", 5000);
                showNotice({
                    level: "warning",
                    code: "model_missing",
                    title: "Granite Speech is not installed",
                    message: "Download the Granite Speech bundle from Settings or switch to another engine.",
                    sticky: true,
                });
                setIsSettingsOpen(true);
                return;
            }
            try {
                const gStatus = await invoke("get_granite_status") as { loaded: boolean };
                if (!gStatus.loaded) {
                    setHeaderStatus("Loading Granite Speech...", 60_000);
                    setSessionPhase?.("loading_model");
                    const gid = currentCohereModel || cohereModels[0]?.id;
                    const result = await invoke<CommandResult<string>>("init_granite", {
                        modelId: gid,
                        forceCpu: asrBackend === "cpu",
                    });
                    if (!result.ok) throw result.error ?? new Error("Failed to initialize Granite Speech");
                    setLoadedEngine("granite");
                    setHeaderStatus("Granite Speech loaded");
                    showNotice(null);
                }
            } catch (e) {
                const error = e as { code?: string; message?: string };
                setHeaderStatus("Failed to initialize Granite Speech: " + error.message, 5000);
                setSessionPhase?.("error");
                showNotice(commandErrorToNotice(error, "Granite Speech failed to load"));
                return;
            }
        }

        try {
            await setTrayState("recording");
            resetRecordingSession();
            // Play start sound before muting so the app's own audio isn't silenced.
            playStart?.();
            if (muteBackgroundAudioRef.current) {
                await invoke("mute_system_audio").catch(e => console.warn("mute_system_audio failed:", e));
            }
            const result = await invoke<CommandResult<string>>("start_recording", { denoise: enableDenoiseRef.current });
            if (!result.ok) throw result.error ?? new Error("Failed to start recording");
            setHeaderStatus(result.data ?? "Recording started");
            recordingStartTimeRef.current = Date.now();
            setIsRecording(true);
            isRecordingRef.current = true;
            setSessionPhase?.("recording");
            showNotice(null);
            if (fromHotkey) {
                clearPendingOverlayHide();
                if (enableOverlayRef.current) {
                    for (let attempt = 0; attempt < 2; attempt++) {
                        try {
                            await invoke("show_overlay");
                            break;
                        } catch {
                            if (attempt === 1) console.warn("show_overlay failed after retry");
                        }
                    }
                    await new Promise(r => setTimeout(r, 80));
                    emitOverlayState("recording").catch(() => { });
                }
            }
        } catch (e) {
            const error = e as { code?: string; message?: string };
            const errStr = String(error.message ?? e);
            if ((error.code ?? "").includes("already_recording") || errStr.includes("Already recording")) {
                setHeaderStatus("Recording already in progress", 2000);
                return;
            }
            console.error("Start recording failed:", e);
            setHeaderStatus("Error: " + errStr, 5000);
            if (muteBackgroundAudioRef.current) {
                await invoke("unmute_system_audio").catch(() => {});
            }
            playError?.();
            await setTrayState("ready");
            setIsRecording(false);
            isRecordingRef.current = false;
            setSessionPhase?.("error");
            showNotice(commandErrorToNotice(error, "Recording failed to start"));
            if (fromHotkey) hideOverlay();
        }
    };

    const handlePauseRecording = async () => {
        if (!isRecordingRef.current || isPausedRef.current || isProcessingTranscriptRef.current) return;
        try {
            await invoke("pause_recording");
            setIsPaused(true);
            isPausedRef.current = true;
            pausedAtRef.current = Date.now();
            setHeaderStatus("Recording paused", 1500);
            setSessionPhase?.("paused");
            await emitOverlayState("paused", liveTranscriptRef.current);
        } catch (e) {
            setHeaderStatus("Couldn't pause recording: " + e, 4000);
        }
    };

    const handleResumeRecording = async () => {
        if (!isRecordingRef.current || !isPausedRef.current || isProcessingTranscriptRef.current) return;
        try {
            await invoke("resume_recording");
            settlePauseWindow();
            setIsPaused(false);
            isPausedRef.current = false;
            setHeaderStatus("Recording resumed", 1500);
            setSessionPhase?.("recording");
            await emitOverlayState("recording", liveTranscriptRef.current);
        } catch (e) {
            setHeaderStatus("Couldn't resume recording: " + e, 4000);
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
        settlePauseWindow();
        setIsPaused(false);
        isPausedRef.current = false;
        setProcessingTranscript(true);
        setSessionPhase?.("processing");
        if (showOverlay) {
            emitOverlayState("transcribing", liveTranscriptRef.current).catch(() => { });
        }

        try {
            await setTrayState("processing");
            if (currentEngine === "whisper") setHeaderStatus("Processing transcription...", 15_000, true);

            const stopResult = await invoke<CommandResult<string>>("stop_recording");
            if (!stopResult.ok) {
                throw stopResult.error ?? new Error("Failed to stop recording");
            }
            let finalTrans = stopResult.data ?? "";

            // Apply custom dictionary substitutions (before grammar LLM)
            finalTrans = applyDictionary(finalTrans, dictionaryRef.current ?? []);

            const recordingDurationMs = getEffectiveRecordingMs();
            if (recordingDurationMs < MIN_RECORDING_MS) {
                setHeaderStatus("Recording too short — try holding a little longer", 5000);
                if (muteBackgroundAudioRef.current) {
                    await invoke("unmute_system_audio").catch(() => {});
                }
                playError?.();
                resetRecordingSession();
                setProcessingTranscript(false);
                await setTrayState("ready");
                setSessionPhase?.("warning");
                showNotice({
                    level: "warning",
                    code: "recording_too_short",
                    title: "Recording too short",
                    message: "Hold the hotkey a little longer so the app has enough speech to process.",
                    sticky: true,
                });
                if (showOverlay) {
                    overlayTerminalRef.current = true;
                    await emitOverlayState("too_short");
                    await new Promise(resolve => setTimeout(resolve, 1000));
                }
                if (isOverlay) {
                    hideOverlay();
                }
                return;
            }

            if (!finalTrans.trim()) {
                setHeaderStatus("Nothing was heard — check your mic input or try again", 5500);
                if (muteBackgroundAudioRef.current) {
                    await invoke("unmute_system_audio").catch(() => {});
                }
                playError?.();
                resetRecordingSession();
                setProcessingTranscript(false);
                await setTrayState("ready");
                setSessionPhase?.("warning");
                showNotice({
                    level: "warning",
                    code: "nothing_heard",
                    title: "Nothing was heard",
                    message: "Check the selected microphone, input level, or background noise and try again.",
                    sticky: true,
                });
                if (showOverlay) {
                    overlayTerminalRef.current = true;
                    await emitOverlayState("nothing_heard");
                    await new Promise(resolve => setTimeout(resolve, 1100));
                }
                if (isOverlay) {
                    hideOverlay();
                }
                return;
            }

            if (enableGrammarLMRef.current) {
                if (showOverlay) {
                    emitOverlayState("correcting", liveTranscriptRef.current).catch(() => { });
                }
                setHeaderStatus("Correcting grammar...", 60_000, true);
                try {
                    const activeStyle = transcriptionStyleRef.current;
                    finalTrans = await invoke("correct_text", { text: finalTrans, style: activeStyle });
                    setHeaderStatus("Transcribed & Corrected!");
                } catch (e) {
                    setHeaderStatus("Transcript ready — grammar step failed: " + e, 5000);
                    showNotice({
                        level: "warning",
                        code: "unknown",
                        title: "Grammar correction failed",
                        message: "The transcript is ready, but the grammar correction step failed.",
                        sticky: true,
                    });
                }
            }

            // Apply text snippets last (after grammar LLM so expansions aren't mangled)
            finalTrans = applySnippets(finalTrans, snippetsRef.current ?? []);

            const totalMs = Date.now() - processingStartMs;
            setLatestLatency(totalMs);
            liveTranscriptRef.current = finalTrans;
            setSessionTranscript?.(finalTrans);
            setSessionLatency?.(totalMs);

            // Capture paste result without blocking history/unmute — a failed
            // paste means the transcript is still shown in the UI, just not
            // inserted into the target app.
            let pasteError: string | null = null;
            try {
                const typeResult = await invoke<CommandResult<null>>("type_text", { text: finalTrans });
                if (!typeResult.ok) {
                    pasteError = typeResult.error?.code ?? typeResult.error?.message ?? "paste_failed";
                }
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
                const activeModelId =
                    currentEngine === "whisper" ? currentModel :
                    currentEngine === "parakeet" ? currentParakeetModel :
                    currentEngine === "granite" ? currentCohereModel : null;
                await invoke("save_transcript_history", {
                    transcript: finalTrans,
                    engine: currentEngine,
                    durationMs: recordingDurationMs,
                    grammarLlmUsed: enableGrammarLMRef.current,
                    processingTimeMs: totalMs,
                    modelId: activeModelId ?? null,
                    audioSource: "microphone",
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
                setSessionPhase?.("warning");
                showNotice({
                    level: "warning",
                    code: pasteError.includes("secure_input") ? "paste_blocked_secure_input" : pasteError.includes("console") ? "paste_blocked_console" : "paste_failed",
                    title: "Transcript ready but paste was blocked",
                    message: headerMsg,
                    sticky: true,
                });
                if (showOverlay) {
                    overlayTerminalRef.current = true;
                    await emitOverlayState("paste_failed", finalTrans);
                }
                if (isOverlay) {
                    clearPendingOverlayHide();
                    overlayHideTimerRef.current = setTimeout(() => {
                        overlayHideTimerRef.current = null;
                        hideOverlay();
                    }, 2000);
                }
            } else {
                // Clear the "Processing transcription..." status only when no
                // grammar LM ran (the grammar branch already set its own message).
                if (!enableGrammarLMRef.current) {
                    setHeaderStatus("Done!", 900);
                }
                playPaste?.();
                setSessionPhase?.("success");
                showNotice(null);
                if (showOverlay) {
                    const preview = finalTrans.slice(0, 60) + (finalTrans.length > 60 ? "…" : "");
                    overlayTerminalRef.current = true;
                    await emitOverlayState("done", preview, totalMs);
                }
                if (isOverlay) {
                    clearPendingOverlayHide();
                    overlayHideTimerRef.current = setTimeout(() => {
                        overlayHideTimerRef.current = null;
                        hideOverlay();
                    }, 1500);
                }
            }

            setProcessingTranscript(false);
            await setTrayState("ready");
        } catch (e) {
            console.error("Stop recording failed:", e);
            const error = e as { code?: string; message?: string };
            const errStr = String(error.message ?? e);
            if (!errStr.includes("Not recording")) {
                setHeaderStatus("Error: " + errStr, 5000);
                if (muteBackgroundAudioRef.current) {
                    await invoke("unmute_system_audio").catch(() => {});
                }
                playError?.();
                setSessionPhase?.("error");
                showNotice(commandErrorToNotice(error, "Recording failed to stop cleanly"));
            }
            isRecordingRef.current = false;
            setProcessingTranscript(false);
            await setTrayState("ready");
            if (isOverlay) {
                overlayTerminalRef.current = true;
                hideOverlay();
            }
        } finally {
            if (muteBackgroundAudioRef.current) {
                invoke("unmute_system_audio").catch(e => console.warn("unmute_system_audio failed:", e));
            }
        }
        hotkeySessionRef.current = false;
    };

    const handleCancelRecording = async () => {
        const isOverlay = hotkeySessionRef.current;
        const showOverlay = isOverlay && enableOverlayRef.current;
        if (!isRecordingRef.current && !isPausedRef.current) return;

        clearPendingOverlayHide();
        try {
            await invoke("cancel_recording");
            if (muteBackgroundAudioRef.current) {
                await invoke("unmute_system_audio").catch(() => {});
            }
            setIsRecording(false);
            isRecordingRef.current = false;
            setProcessingTranscript(false);
            await setTrayState("ready");
            resetRecordingSession();
            setHeaderStatus("Recording discarded", 1800);
            playError?.();
            setSessionPhase?.("warning");
            showNotice({
                level: "warning",
                code: "unknown",
                title: "Recording discarded",
                message: "The current recording was cancelled before transcription finished.",
                sticky: false,
            });

            if (showOverlay) {
                overlayTerminalRef.current = true;
                await emitOverlayState("cancelled");
                await new Promise((resolve) => setTimeout(resolve, 900));
            }
        } catch (e) {
            setHeaderStatus("Couldn't cancel recording: " + e, 4000);
        } finally {
            if (isOverlay) {
                hideOverlay();
            }
            hotkeySessionRef.current = false;
        }
    };

    const handlePartialChunk = (word: string) => {
        // Reject partials when not actively recording or when paused.
        if ((!isRecordingRef.current && !isProcessingTranscriptRef.current) || isPausedRef.current) return;
        const trimmed = word.trim();
        if (!trimmed) return;

        if (!coherePartialActiveRef.current) {
            // First partial for this VAD chunk — snapshot the committed transcript.
            coherePartialBaseRef.current = liveTranscriptRef.current;
            coherePartialActiveRef.current = true;
        }
        const base = coherePartialBaseRef.current;
        const next = base ? `${base} ${trimmed}` : trimmed;
        liveTranscriptRef.current = next;
        if (hotkeySessionRef.current && enableOverlayRef.current && !overlayTerminalRef.current) {
            const phase = isRecordingRef.current ? "recording" : "transcribing";
            emitOverlayState(phase, next)?.catch(() => {});
        }
    };

    const handleTranscriptionChunk = (chunkText: string) => {
        // Accept chunks when actively recording OR during the processing window
        // (tail flush events arrive while stop_recording is awaited). Reject when
        // the session is fully idle or paused — that would be stale/cross-session audio.
        if ((!isRecordingRef.current && !isProcessingTranscriptRef.current) || isPausedRef.current) return;
        const cleanChunk = chunkText.trim();
        if (!cleanChunk) return;

        // If provisional partials were streaming, reset live transcript to the snapshot
        // taken before partials started so the authoritative decoded text can replace
        // them without duplication.
        if (coherePartialActiveRef.current) {
            liveTranscriptRef.current = coherePartialBaseRef.current;
            coherePartialActiveRef.current = false;
            coherePartialBaseRef.current = "";
        }

        // Leading space in chunkText = SentencePiece word boundary marker.
        // Only join with a space when the chunk starts a new word; a chunk that
        // continues the previous word (e.g. "ed" after "nest") has no leading space
        // and must be concatenated directly so "nested" doesn't become "nest ed".
        const startsNewWord = !liveTranscriptRef.current || chunkText.startsWith(" ");
        const sep = startsNewWord ? " " : "";
        const nextTranscript = `${liveTranscriptRef.current}${sep}${cleanChunk}`.replace(/\s+/g, " ").trim();
        liveTranscriptRef.current = nextTranscript;
        setSessionTranscript?.(nextTranscript);

        if (hotkeySessionRef.current && enableOverlayRef.current && !overlayTerminalRef.current) {
            const phase = isRecordingRef.current ? "recording" : "transcribing";
            emitOverlayState(phase, nextTranscript)?.catch(() => { });
        }
    };

    return {
        isRecording,
        isRecordingRef,
        isPaused,
        isProcessingTranscript,
        latestLatency,
        handleStartRecording,
        handlePauseRecording,
        handleResumeRecording,
        handleStopRecording,
        handleCancelRecording,
        handleTranscriptionChunk,
        handlePartialChunk,
    };
}
