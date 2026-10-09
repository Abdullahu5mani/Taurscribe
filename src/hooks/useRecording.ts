import type { TrayState } from "../App";
import { useState, useRef, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ModelInfo, GraniteModelInfo, Qwen3ModelInfo } from "./useModels";
import type { ASREngine } from "./useEngineSwitch";
import { applyDictionary, applySnippets } from "./usePersonalization";
import { ensureEngineModel } from "../utils/engineLoader";
import type { DictEntry, SnippetEntry } from "./usePersonalization";
import type { CommandResult, RecordingOutcome, SessionNotice } from "../types/session";

interface UseRecordingParams {
    activeEngineRef: React.RefObject<ASREngine>;
    models: ModelInfo[];
    graniteModels: GraniteModelInfo[];
    qwen3Models: Qwen3ModelInfo[];
    currentModel: string | null;
    currentGraniteModel: string | null;
    currentQwen3Model: string | null;
    asrBackend: "gpu" | "cpu";
    isFileTranscribingRef?: React.RefObject<boolean>;
    setCurrentModel: (id: string) => void;
    setLoadedEngine: (engine: ASREngine) => void;
    enableGrammarLMRef: React.RefObject<boolean>;
    enableDenoiseRef: React.RefObject<boolean>;
    enableOverlayRef: React.RefObject<boolean>;
    muteBackgroundAudioRef: React.RefObject<boolean>;
    transcriptionStyleRef: React.MutableRefObject<string>;
    setHeaderStatus: (msg: string, dur?: number, isProcessing?: boolean) => void;
    setTrayState: (state: TrayState, meeting?: undefined, detail?: string) => Promise<void>;
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
    graniteModels,
    qwen3Models,
    currentModel,
    currentGraniteModel,
    currentQwen3Model,
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
    const [isVoiceEnrolling, setIsVoiceEnrolling] = useState(false);
    const isVoiceEnrollingRef = useRef(false);
    const [isPaused, setIsPaused] = useState(false);
    const [isProcessingTranscript, setIsProcessingTranscript] = useState(false);
    const [latestLatency, setLatestLatency] = useState<number | null>(null);

    const isRecordingRef = useRef(false);
    /** True while the running session is a dual-channel meeting recording. */
    const meetingSessionRef = useRef(false);
    const isProcessingTranscriptRef = useRef(false);
    const isPausedRef = useRef(false);
    // Pause/resume commands may settle after Stop or after a new session begins.
    const recordingGenerationRef = useRef(0);
    const recordingEventVersionRef = useRef(0);
    const recordingSessionRef = useRef(0);
    // Only one Pause/Resume request per generation may be in flight.
    const pauseResumeOperationRef = useRef<number | null>(null);
    const recordingStartTimeRef = useRef(0);
    const hotkeySessionRef = useRef(false);
    const overlayHideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    const overlaySessionIdRef = useRef(0);
    const overlayTerminalRef = useRef(false);
    const overlayCommandChainRef = useRef<Promise<void>>(Promise.resolve());
    const liveTranscriptRef = useRef("");
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
        setIsDualChannelRecording(false);
        setIsInPersonRecording(false);
        setDualLevels({ mic: 0, system: 0 });
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

    const [isDualChannelRecording, setIsDualChannelRecording] = useState(false);
    // An in-person meeting: one microphone, saved to Meetings like a call.
    const [isInPersonRecording, setIsInPersonRecording] = useState(false);
    const [dualLevels, setDualLevels] = useState<{ mic: number; system: number }>({ mic: 0, system: 0 });

    useEffect(() => {
        let disposed = false;
        let receivedEvent = false;
        const update = (active: boolean) => {
            isVoiceEnrollingRef.current = active;
            setIsVoiceEnrolling(active);
        };
        const subscription = listen<{ active: boolean }>("voice-enrollment-state", ({ payload }) => {
            receivedEvent = true;
            if (!disposed) update(payload.active);
        });
        // Subscribe before hydrating so an old status response cannot overwrite
        // a newer transition from a setup running in another window.
        subscription.then(() => invoke<{ active?: boolean }>("get_my_voice"))
            .then(status => { if (!disposed && !receivedEvent) update(status?.active === true); })
            .catch(() => {});
        return () => { disposed = true; subscription.then(unlisten => unlisten()); };
    }, []);

    // Recordings can start or stop outside the UI (control server, tray). Follow
    // the backend so the UI never keeps showing RECORDING for a stopped session.
    useEffect(() => {
        const unlistenPromise = listen<{ is_recording: boolean; is_dual_channel: boolean; is_in_person?: boolean }>("recording-state", (event) => {
            const { is_recording, is_dual_channel, is_in_person = false } = event.payload;
            recordingEventVersionRef.current += 1;
            if (is_recording && !isRecordingRef.current) {
                recordingGenerationRef.current += 1;
                recordingSessionRef.current += 1;
                if (isProcessingTranscriptRef.current) {
                    hotkeySessionRef.current = false;
                    setProcessingTranscript(false);
                }
                resetRecordingSession();
                recordingStartTimeRef.current = Date.now();
                showNotice(null);
                setIsRecording(true);
                isRecordingRef.current = true;
                setIsDualChannelRecording(is_dual_channel);
                setIsInPersonRecording(is_in_person);
                meetingSessionRef.current = is_dual_channel || is_in_person;
                setSessionPhase?.("recording");
                void emitOverlayState("recording").catch(() => {});
            } else if (!is_recording) {
                recordingGenerationRef.current += 1;
                if (isRecordingRef.current) {
                    setIsRecording(false);
                    isRecordingRef.current = false;
                    resetRecordingSession();
                }
                if (!isProcessingTranscriptRef.current) {
                    setSessionPhase?.("idle");
                    void setTrayState("ready");
                }
            }
        });
        return () => {
            unlistenPromise.then((unlisten) => unlisten());
        };
    }, []);

    useEffect(() => {
        const unlistenPromise = listen<{ mic: number; system: number }>("dual-audio-levels", (event) => {
            if (!isRecordingRef.current || !meetingSessionRef.current) return;
            setDualLevels(event.payload);
        });
        return () => {
            unlistenPromise.then((unlisten) => unlisten());
        };
    }, []);

    const startRecordingOnce = async (generation: number, fromHotkey = false, audioSource?: string) => {
        const isCurrent = () => generation === recordingGenerationRef.current;
        hotkeySessionRef.current = fromHotkey; // tracks hotkey session independent of overlay toggle
        let effectiveAudioSource = audioSource;
        if (!effectiveAudioSource) {
            try {
                effectiveAudioSource = await invoke<string>("get_audio_source_mode");
            } catch {
                effectiveAudioSource = "mic";
            }
        }
        if (!isCurrent()) return;
        const isDual = effectiveAudioSource === "dual_channel";
        const isInPerson = effectiveAudioSource === "in_person";
        const isMeeting = isDual || isInPerson;
        setIsDualChannelRecording(isDual);
        setIsInPersonRecording(isInPerson);
        meetingSessionRef.current = isMeeting;
        if (fromHotkey) {
            overlaySessionIdRef.current += 1;
            overlayTerminalRef.current = false;
        }
        const currentEngine = activeEngineRef.current;

        // Make sure the active engine's model is loaded (utils/engineLoader.ts).
        const engineSetup = {
            whisper: {
                installed: models,
                selected: (currentModel ?? "").trim(),
                select: setCurrentModel as ((id: string) => void) | null,
                label: "Whisper model",
                missingHeader: "No Whisper models installed! Please download one.",
                missingNotice: {
                    title: "No Whisper model installed",
                    message: "Download a Whisper model or switch to another installed engine before recording.",
                } as { title: string; message: string } | null,
            },
            granite: {
                installed: graniteModels,
                selected: currentGraniteModel ?? "",
                select: null,
                label: "Granite",
                missingHeader: "No Granite models installed!",
                missingNotice: {
                    title: "Granite is not installed",
                    message: "Download Granite from Settings or switch to Whisper or Qwen3 before recording.",
                },
            },
            qwen3: {
                installed: qwen3Models,
                selected: currentQwen3Model ?? "",
                select: null,
                label: "Qwen3-ASR",
                missingHeader: "No Qwen3-ASR model installed. Download it from Settings.",
                missingNotice: null,
            },
        }[currentEngine];
        if (engineSetup.installed.length === 0) {
            setHeaderStatus(engineSetup.missingHeader, 5000);
            if (engineSetup.missingNotice) {
                showNotice({ level: "warning", code: "model_missing", sticky: true, ...engineSetup.missingNotice });
            }
            setIsSettingsOpen(true);
            return;
        }
        const targetModel = engineSetup.selected || engineSetup.installed[0].id;
        if (!engineSetup.selected) engineSetup.select?.(targetModel);
        try {
            const outcome = await ensureEngineModel(currentEngine, targetModel, asrBackend === "gpu", () => {
                if (!isCurrent()) throw new Error("Recording request superseded");
                setHeaderStatus(`Loading ${engineSetup.label}...`, 60_000);
                setSessionPhase?.("loading_model");
            });
            if (!isCurrent()) return;
            if (outcome === "loaded") {
                setLoadedEngine(currentEngine);
                setHeaderStatus(`${engineSetup.label} loaded`);
                showNotice(null);
            }
        } catch (e) {
            if (!isCurrent()) return;
            const error = e as { code?: string; message?: string };
            setHeaderStatus(`Failed to load ${engineSetup.label}: ${error.message}`, 5000);
            setSessionPhase?.("error");
            showNotice(commandErrorToNotice(error, `${engineSetup.label} failed to load`));
            return;
        }

        let mutedByStart = false;
        try {
            await setTrayState("recording");
            if (!isCurrent()) return;
            resetRecordingSession();
            // Play start sound before muting so the app's own audio isn't silenced.
            playStart?.();
            if (muteBackgroundAudioRef.current && !isMeeting) {
                await invoke("mute_system_audio").then(() => { mutedByStart = true; })
                    .catch(e => console.warn("mute_system_audio failed:", e));
            }
            if (!isCurrent()) return;
            const result = await invoke<CommandResult<string>>("start_recording", {
                denoise: enableDenoiseRef.current,
                audioSource: isDual ? "dual_channel" : isInPerson ? "in_person" : "mic",
            });
            if (!isCurrent()) return;
            if (!result.ok) throw result.error ?? new Error("Failed to start recording");
            // The command reports the temp file path; users only need to know it started.
            setHeaderStatus("Recording started");
            // A backend event may already have established this session's clock.
            if (!isRecordingRef.current) {
                recordingStartTimeRef.current = Date.now();
                recordingSessionRef.current += 1;
            }
            setIsRecording(true);
            isRecordingRef.current = true;
            setIsDualChannelRecording(isDual);
            setIsInPersonRecording(isInPerson);
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
            if (!isCurrent()) return;
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
            if (!isCurrent()) return;
            playError?.();
            await setTrayState("ready");
            if (!isCurrent()) return;
            setIsRecording(false);
            isRecordingRef.current = false;
            setIsDualChannelRecording(false);
            setIsInPersonRecording(false);
            setDualLevels({ mic: 0, system: 0 });
            setSessionPhase?.("error");
            showNotice(commandErrorToNotice(error, "Recording failed to start"));
            if (fromHotkey) hideOverlay();
        } finally {
            // A cancelled request can finish muting after Stop already restored
            // audio. Release that late mute unless a live dictation needs it.
            if (mutedByStart && (!isRecordingRef.current || meetingSessionRef.current)) {
                await invoke("unmute_system_audio").catch(() => {});
            }
        }
    };

    // One start at a time: a second start request (a duplicate tray/hotkey event
    // arriving while the first is still loading a model or opening devices) used
    // to start overlapping recordings.
    const startingRef = useRef(false);
    const handleStartRecording = async (fromHotkey = false, audioSource?: string) => {
        if (isVoiceEnrollingRef.current) {
            setHeaderStatus("Finish setting up your voice before recording", 4000);
            return;
        }
        if (startingRef.current || isRecordingRef.current || isProcessingTranscriptRef.current) return;
        startingRef.current = true;
        recordingGenerationRef.current += 1;
        try {
            await startRecordingOnce(recordingGenerationRef.current, fromHotkey, audioSource);
        } finally {
            startingRef.current = false;
        }
    };

    const handlePauseRecording = async () => {
        if (!isRecordingRef.current || isPausedRef.current || isProcessingTranscriptRef.current) return;
        const generation = recordingGenerationRef.current;
        if (pauseResumeOperationRef.current === generation) return;
        pauseResumeOperationRef.current = generation;
        try {
            const result = await invoke<CommandResult<string>>("pause_recording");
            if (generation !== recordingGenerationRef.current) return;
            if (!result.ok) throw result.error?.message ?? "not paused";
            setIsPaused(true);
            isPausedRef.current = true;
            pausedAtRef.current = Date.now();
            setHeaderStatus("Recording paused", 1500);
            setSessionPhase?.("paused");
            await setTrayState("paused");
            if (generation !== recordingGenerationRef.current) return;
            await emitOverlayState("paused", liveTranscriptRef.current);
        } catch (e) {
            if (generation !== recordingGenerationRef.current) return;
            setHeaderStatus("Couldn't pause recording: " + e, 4000);
        } finally {
            if (pauseResumeOperationRef.current === generation) pauseResumeOperationRef.current = null;
        }
    };

    const handleResumeRecording = async () => {
        if (!isRecordingRef.current || !isPausedRef.current || isProcessingTranscriptRef.current) return;
        const generation = recordingGenerationRef.current;
        if (pauseResumeOperationRef.current === generation) return;
        pauseResumeOperationRef.current = generation;
        try {
            const result = await invoke<CommandResult<string>>("resume_recording");
            if (generation !== recordingGenerationRef.current) return;
            if (!result.ok) throw result.error?.message ?? "not resumed";
            settlePauseWindow();
            setIsPaused(false);
            isPausedRef.current = false;
            setHeaderStatus("Recording resumed", 1500);
            setSessionPhase?.("recording");
            await setTrayState("recording");
            if (generation !== recordingGenerationRef.current) return;
            await emitOverlayState("recording", liveTranscriptRef.current);
        } catch (e) {
            if (generation !== recordingGenerationRef.current) return;
            setHeaderStatus("Couldn't resume recording: " + e, 4000);
        } finally {
            if (pauseResumeOperationRef.current === generation) pauseResumeOperationRef.current = null;
        }
    };

    const handleStopRecording = async () => {
        if (isProcessingTranscriptRef.current) return;
        recordingGenerationRef.current += 1;
        const wasRecording = isRecordingRef.current;
        const wasPaused = isPausedRef.current;
        const pausedAt = pausedAtRef.current;
        const pausedTotal = totalPausedMsRef.current;
        const eventVersion = recordingEventVersionRef.current;
        const session = recordingSessionRef.current;
        const superseded = () => session !== recordingSessionRef.current;
        const setStopProcessing = (value: boolean) => {
            if (!superseded()) setProcessingTranscript(value);
        };
        let stopAccepted = false;
        const currentEngine = activeEngineRef.current;
        const processingStartMs = Date.now();
        const isOverlay = hotkeySessionRef.current;              // true for any hotkey session
        const showOverlay = isOverlay && enableOverlayRef.current; // true only when overlay is enabled
        console.log("[STOP] handleStopRecording called. GrammarLM:", enableGrammarLMRef.current);
        setIsRecording(false);
        isRecordingRef.current = false;
        settlePauseWindow();
        // Taken now: the time spent processing after Stop is not recording time.
        const recordedMsAtStop = getEffectiveRecordingMs();
        setIsPaused(false);
        isPausedRef.current = false;
        setStopProcessing(true);
        setSessionPhase?.("processing");
        if (showOverlay) {
            emitOverlayState("transcribing", liveTranscriptRef.current).catch(() => { });
        }

        try {
            await setTrayState(meetingSessionRef.current ? "processing_meeting" : "processing_speech");
            if (superseded()) return;
            if (currentEngine === "whisper") setHeaderStatus("Processing transcription...", 15_000, true);

            const stopResult = await invoke<CommandResult<RecordingOutcome>>("stop_recording");
            if (superseded()) return;
            if (!stopResult.ok || !stopResult.data) {
                throw stopResult.error ?? new Error("Failed to stop recording");
            }
            stopAccepted = true;
            const outcome = stopResult.data;
            const captureWarning = (outcome.warnings ?? []).join(" ");
            const recovery = outcome.recording_path ? ` Recording retained at ${outcome.recording_path}.` : "";
            const captureNotice: SessionNotice | null = captureWarning
                ? { level: "warning", code: "recording_incomplete", title: "Recording needs attention", message: captureWarning + recovery, sticky: true }
                : null;
            let finalTrans = outcome.transcript;
            let grammarNotice: SessionNotice | null = null;
            let grammarApplied = false;

            // Meeting recordings belong to the meetings area only: the backend
            // saves and processes them there, and says which kind this was.
            // Never paste a meeting transcript or add it to dictation history.
            meetingSessionRef.current = false;
            if (outcome.kind === "meeting") {
                if (muteBackgroundAudioRef.current) {
                    await invoke("unmute_system_audio").catch(() => {});
                }
                if (superseded()) return;
                resetRecordingSession();
                setStopProcessing(false);
                await setTrayState("done", undefined, "Meeting saved to Meetings");
                if (superseded()) return;
                setSessionPhase?.(captureNotice ? "warning" : "idle");
                showNotice(captureNotice);
                setHeaderStatus(captureNotice ? "Meeting saved with incomplete audio — check the warning" : "Meeting recording saved to Meetings", 6000);
                if (isOverlay) {
                    hideOverlay();
                }
                return;
            }

            // Apply custom dictionary substitutions (before grammar LLM)
            finalTrans = applyDictionary(finalTrans, dictionaryRef.current ?? []);

            // Measured by the backend from the audio captured.
            const recordingDurationMs = outcome.duration_ms ?? recordedMsAtStop;
            if (outcome.too_short) {
                setHeaderStatus("Recording too short — try holding a little longer", 5000);
                if (muteBackgroundAudioRef.current) {
                    await invoke("unmute_system_audio").catch(() => {});
                }
                if (superseded()) return;
                playError?.();
                resetRecordingSession();
                setStopProcessing(false);
                await setTrayState("nothing_heard", undefined, "Recording too short");
                if (superseded()) return;
                setSessionPhase?.("warning");
                showNotice(captureNotice ?? {
                    level: "warning",
                    code: "recording_too_short",
                    title: "Recording too short",
                    message: "Hold the hotkey a little longer so the app has enough speech to process.",
                    sticky: true,
                });
                if (showOverlay) {
                    overlayTerminalRef.current = true;
                    await emitOverlayState("too_short");
                    if (superseded()) return;
                    await new Promise(resolve => setTimeout(resolve, 1000));
                }
                if (superseded()) return;
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
                if (superseded()) return;
                playError?.();
                resetRecordingSession();
                setStopProcessing(false);
                await setTrayState("nothing_heard", undefined, "Nothing heard — check your microphone");
                if (superseded()) return;
                setSessionPhase?.("warning");
                showNotice(captureNotice ?? {
                    level: "warning",
                    code: "nothing_heard",
                    title: "Nothing was heard",
                    message: "Check the selected microphone, input level, or background noise and try again.",
                    sticky: true,
                });
                if (showOverlay) {
                    overlayTerminalRef.current = true;
                    await emitOverlayState("nothing_heard");
                    if (superseded()) return;
                    await new Promise(resolve => setTimeout(resolve, 1100));
                }
                if (superseded()) return;
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
                void setTrayState("grammar");
                try {
                    const activeStyle = transcriptionStyleRef.current;
                    finalTrans = await invoke("correct_text", { text: finalTrans, style: activeStyle });
                    if (superseded()) return;
                    grammarApplied = true;
                    setHeaderStatus("Transcribed & Corrected!");
                } catch (e) {
                    if (superseded()) return;
                    setHeaderStatus("Transcript ready — grammar step failed: " + e, 5000);
                    grammarNotice = {
                        level: "warning",
                        code: "unknown",
                        title: "Grammar correction failed",
                        message: "The transcript is ready, but the grammar correction step failed.",
                        sticky: true,
                    };
                    showNotice(grammarNotice);
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

            if (superseded()) return;
            if (muteBackgroundAudioRef.current) {
                await invoke("unmute_system_audio").catch(e => console.warn("unmute_system_audio failed:", e));
            }

            // The backend saved the recognized text before deleting the
            // recording; store the final text (dictionary, grammar, snippets)
            // in the same entry. If the backend could not save it, try once
            // more, and otherwise say so: the recording was kept.
            let historyError: string | null = null;
            if (outcome.history.status === "saved") {
                try {
                    await invoke("update_transcript_history", {
                        id: outcome.history.id,
                        transcript: finalTrans,
                        grammarLlmUsed: grammarApplied,
                        processingTimeMs: totalMs,
                    });
                } catch (e) {
                    console.warn("Failed to update transcript history (the recognized text is saved):", e);
                    historyError = "The recognized text is saved, but the final edits could not be stored in history.";
                }
                onHistorySaved?.();
            } else if (outcome.history.status === "not_saved") {
                try {
                    const activeModelId =
                        currentEngine === "whisper" ? currentModel :
                        currentEngine === "granite" ? currentGraniteModel :
                        currentEngine === "qwen3" ? currentQwen3Model : null;
                    await invoke("save_transcript_history", {
                        transcript: finalTrans,
                        engine: currentEngine,
                        durationMs: recordingDurationMs,
                        grammarLlmUsed: grammarApplied,
                        processingTimeMs: totalMs,
                        modelId: activeModelId ?? null,
                        audioSource: "microphone",
                    });
                    onHistorySaved?.();
                } catch (e) {
                    console.warn("Failed to save transcript history:", e);
                    const kept = outcome.history.recording_path ? ` The recording is kept at ${outcome.history.recording_path}.` : "";
                    historyError = `It couldn't be saved to history (${outcome.history.error}).${kept}`;
                }
            }
            if (superseded()) return;
            const unsavedNotice: SessionNotice | null = historyError
                ? { level: "warning", code: "history_not_saved", title: outcome.history.status === "saved" ? "Final edits not saved to history" : "Transcript not saved to history", message: [historyError, captureNotice?.message, grammarNotice?.message].filter(Boolean).join(" "), sticky: true }
                : captureNotice
                    ? { ...captureNotice, message: [captureNotice.message, grammarNotice?.message].filter(Boolean).join(" ") }
                    : grammarNotice;

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
                await setTrayState("paste_failed", undefined, headerMsg);
                if (superseded()) return;
                setSessionPhase?.("warning");
                showNotice({
                    level: "warning",
                    code: pasteError.includes("secure_input") ? "paste_blocked_secure_input" : pasteError.includes("console") ? "paste_blocked_console" : "paste_failed",
                    title: "Transcript ready but paste was blocked",
                    message: [headerMsg, unsavedNotice?.message].filter(Boolean).join(" "),
                    sticky: true,
                });
                if (showOverlay) {
                    overlayTerminalRef.current = true;
                    await emitOverlayState("paste_failed", finalTrans);
                }
                if (superseded()) return;
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
                    setHeaderStatus(unsavedNotice ? unsavedNotice.title : "Done!", unsavedNotice ? 6000 : 900);
                }
                playPaste?.();
                await setTrayState("done", undefined, finalTrans.slice(0, 60) + (finalTrans.length > 60 ? "…" : ""));
                if (superseded()) return;
                setSessionPhase?.(unsavedNotice ? "warning" : "success");
                showNotice(unsavedNotice);
                if (showOverlay) {
                    const preview = finalTrans.slice(0, 60) + (finalTrans.length > 60 ? "…" : "");
                    overlayTerminalRef.current = true;
                    await emitOverlayState("done", preview, totalMs);
                }
                if (superseded()) return;
                if (isOverlay) {
                    clearPendingOverlayHide();
                    overlayHideTimerRef.current = setTimeout(() => {
                        overlayHideTimerRef.current = null;
                        hideOverlay();
                    }, 1500);
                }
            }

            setStopProcessing(false);
            await setTrayState("ready");
        } catch (e) {
            if (superseded()) return;
            console.error("Stop recording failed:", e);
            const error = e as { code?: string; message?: string };
            const errStr = String(error.message ?? e);
            // A native stop-state event confirms teardown even when final processing fails.
            // Without that event a gate/IPC rejection may have left capture running.
            if (!stopAccepted && error.code !== "not_recording" && !errStr.includes("Not recording")
                && eventVersion === recordingEventVersionRef.current && wasRecording) {
                setIsRecording(true);
                isRecordingRef.current = true;
                setIsPaused(wasPaused);
                isPausedRef.current = wasPaused;
                pausedAtRef.current = pausedAt;
                totalPausedMsRef.current = pausedTotal;
            }
            if (!errStr.includes("Not recording")) {
                setHeaderStatus("Error: " + errStr, 5000);
                if (muteBackgroundAudioRef.current && !isRecordingRef.current) {
                    await invoke("unmute_system_audio").catch(() => {});
                }
                if (superseded()) return;
                playError?.();
                setSessionPhase?.("error");
                showNotice(commandErrorToNotice(error, "Recording failed to stop cleanly"));
            }
            setStopProcessing(false);
            await setTrayState(isRecordingRef.current ? (isPausedRef.current ? "paused" : "recording") : "ready");
            if (superseded()) return;
            if (isRecordingRef.current) setSessionPhase?.(isPausedRef.current ? "paused" : "recording");
            if (isOverlay && !isRecordingRef.current) {
                overlayTerminalRef.current = true;
                hideOverlay();
            }
        } finally {
            if (session === recordingSessionRef.current) {
                if (muteBackgroundAudioRef.current && !isRecordingRef.current) {
                    invoke("unmute_system_audio").catch(e => console.warn("unmute_system_audio failed:", e));
                }
                if (!startingRef.current) hotkeySessionRef.current = false;
            }
        }
    };

    const handleCancelRecording = async () => {
        const isOverlay = hotkeySessionRef.current;
        const showOverlay = isOverlay && enableOverlayRef.current;
        if ((!isRecordingRef.current && !isPausedRef.current) || isProcessingTranscriptRef.current) return;

        clearPendingOverlayHide();
        const session = recordingSessionRef.current;
        const isCurrent = () => session === recordingSessionRef.current;
        setProcessingTranscript(true);
        try {
            const result = await invoke<CommandResult<null>>("cancel_recording");
            if (!isCurrent()) return;
            if (!result.ok) throw result.error ?? new Error("Failed to cancel recording");
            recordingGenerationRef.current += 1;
            if (muteBackgroundAudioRef.current) {
                await invoke("unmute_system_audio").catch(() => {});
            }
            if (!isCurrent()) return;
            setIsRecording(false);
            isRecordingRef.current = false;
            setProcessingTranscript(false);
            await setTrayState("cancelled");
            if (!isCurrent()) return;
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
            if (!isCurrent()) return;
            const error = e as { code?: string; message?: string };
            if (error.code === "not_recording") {
                setIsRecording(false);
                isRecordingRef.current = false;
                resetRecordingSession();
            }
            setHeaderStatus("Couldn't cancel recording: " + (error.message ?? e), 4000);
        } finally {
            if (isCurrent()) {
                setProcessingTranscript(false);
                if (isOverlay) hideOverlay();
                hotkeySessionRef.current = false;
            }
        }
    };

    const handleTranscriptionChunk = (chunkText: string) => {
        // Accept chunks when actively recording OR during the processing window
        // (tail flush events arrive while stop_recording is awaited). Reject when
        // the session is fully idle or paused — that would be stale/cross-session audio.
        if ((!isRecordingRef.current && !isProcessingTranscriptRef.current) || isPausedRef.current) return;
        const cleanChunk = chunkText.trim();
        if (!cleanChunk) return;

        // Each backend event contains a complete ASR chunk, not a token fragment.
        const nextTranscript = `${liveTranscriptRef.current} ${cleanChunk}`.replace(/\s+/g, " ").trim();
        liveTranscriptRef.current = nextTranscript;
        setSessionTranscript?.(nextTranscript);

        if (hotkeySessionRef.current && enableOverlayRef.current && !overlayTerminalRef.current) {
            const phase = isRecordingRef.current ? "recording" : "transcribing";
            emitOverlayState(phase, nextTranscript)?.catch(() => { });
        }
    };

    return {
        isVoiceEnrolling,
        isRecording,
        isRecordingRef,
        isPaused,
        isProcessingTranscript,
        latestLatency,
        isDualChannelRecording,
        isInPersonRecording,
        dualLevels,
        /** Recording time so far, not counting pauses. */
        getRecordingMs: getEffectiveRecordingMs,
        /** The live transcript of the recording so far. */
        getLiveTranscript: () => liveTranscriptRef.current,
        handleStartRecording,
        handlePauseRecording,
        handleResumeRecording,
        handleStopRecording,
        handleCancelRecording,
        handleTranscriptionChunk,
    };
}
