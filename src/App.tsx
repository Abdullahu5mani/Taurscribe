import { useEffect, useRef, useCallback, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Store } from "@tauri-apps/plugin-store";
import { SettingsModal } from "./components/SettingsModal";
import { SetupWizard } from "./components/SetupWizard";
import { TitleBar } from "./components/TitleBar";
import { useState } from "react";
import { useHeaderStatus } from "./hooks/useHeaderStatus";
import { useModels } from "./hooks/useModels";
import { usePostProcessing } from "./hooks/usePostProcessing";
import { useEngineSwitch } from "./hooks/useEngineSwitch";
import type { ASREngine } from "./hooks/useEngineSwitch";
import { useSessionState } from "./hooks/useSessionState";
import { useRecording } from "./hooks/useRecording";
import { useSounds } from "./hooks/useSounds";
import { usePersonalization } from "./hooks/usePersonalization";
import { TranscriptFeed } from "./components/TranscriptFeed";
import { FileTranscriptionPanel } from "./components/FileTranscriptionPanel";
import { QuickSettings } from "./components/QuickSettings";
import { EnginePicker } from "./components/EnginePicker";
import { SessionNoticeCard } from "./components/SessionNoticeCard";
import { useDownloads } from "./hooks/useDownloads";
import { useInitialLoad } from "./hooks/useInitialLoad";
import { useHotkeyListeners } from "./hooks/useHotkeyListeners";
import { useModelsWatcher } from "./hooks/useModelsWatcher";
import { useSyncedRef } from "./utils/useSyncedRef";
import { MODELS } from "./components/settings/types";
import type { DownloadableModel } from "./components/settings/types";
import { beautifyModelName } from "./utils/modelDisplay";
import type { OnboardingUseCase } from "./modelRecommendations";
import "./components/TitleBar.css";
import "./App.css";
import { IconFileText, IconBolt, IconEject, IconDownload, IconMic, IconLightbulb, IconSettings } from "./components/Icons";
import { getEngineForModelId } from "./utils/engineUtils";
import { OverlayScrollbarsComponent } from "overlayscrollbars-react";
import type { CommandResult } from "./types/session";

const ANIMATED_LOGOS = [
  "animated_logo_breathe.svg",
  "animated_logo_scan_reveal.svg",
  "animated_logo_focus.svg",
  "animated_logo_crt.svg",
  "animated_logo_pulse_reveal.svg",
  "animated_logo_stomp.svg",
];

type EngineSelectionState = {
  active_engine: string;
  selected_model_id: string | null;
  loaded_engine: string | null;
  loaded_model_id: string | null;
  backend: string;
  engine_loading: boolean;
};




const setTrayState = async (newState: "ready" | "recording" | "processing") => {
  try {
    await invoke("set_tray_state", { newState });
  } catch (e) {
    console.error("Failed to set tray state:", e);
  }
};


function App() {
  const pickRandomLogo = useCallback(() => {
    return ANIMATED_LOGOS[Math.floor(Math.random() * ANIMATED_LOGOS.length)];
  }, []);

  const [randomLogo, setRandomLogo] = useState(pickRandomLogo);
  const [isLogoShuttering, setIsLogoShuttering] = useState(false);

  // M6 fix: containerBooting controls the CSS stagger class; cleared after
  // the boot animation completes so re-mounts don't re-trigger the stagger.
  const [containerBooting, setContainerBooting] = useState(true);

  useEffect(() => {
    // Container stagger: clear after all children finish (10 × 80ms + 500ms duration)
    const staggerTimer = setTimeout(() => setContainerBooting(false), 1400);

    return () => {
      clearTimeout(staggerTimer);
    };
  }, []);

  const handleLogoClick = useCallback(() => {
    if (isLogoShuttering) return;
    setIsLogoShuttering(true);
    // Sharp mechanical shutter timing: 150ms to close, swap, 150ms to open
    setTimeout(() => {
      setRandomLogo(pickRandomLogo());
      setTimeout(() => setIsLogoShuttering(false), 150);
    }, 150);
  }, [isLogoShuttering, pickRandomLogo]);

  useEffect(() => {
    let resizeRaf: number | null = null;
    let resizeDoneTimer: number | null = null;

    const markResizing = () => {
      if (!document.body.classList.contains("is-resizing")) {
        document.body.classList.add("is-resizing");
      }
      if (resizeDoneTimer !== null) {
        window.clearTimeout(resizeDoneTimer);
      }
      resizeDoneTimer = window.setTimeout(() => {
        document.body.classList.remove("is-resizing");
        resizeDoneTimer = null;
      }, 140);
    };

    const onResize = () => {
      if (resizeRaf !== null) {
        return;
      }
      resizeRaf = window.requestAnimationFrame(() => {
        resizeRaf = null;
        markResizing();
      });
    };

    window.addEventListener("resize", onResize, { passive: true });

    return () => {
      window.removeEventListener("resize", onResize);
      if (resizeRaf !== null) {
        window.cancelAnimationFrame(resizeRaf);
      }
      if (resizeDoneTimer !== null) {
        window.clearTimeout(resizeDoneTimer);
      }
      document.body.classList.remove("is-resizing");
    };
  }, []);

  // Close the settings modal when the window is hidden to tray so the hotkey
  // works immediately when the user restores the window.
  useEffect(() => {
    const unlisten = listen("window-hidden", () => {
      appHiddenRef.current = true;
      setIsSettingsOpen(false);
    });
    return () => { unlisten.then(fn => fn()); };
  }, []);

  const storeRef = useRef<Store | null>(null);
  const appHiddenRef = useRef(false);
  const pendingNoModelCtaPulseRef = useRef(false);
  const noModelCtaTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [backendInfo, setBackendInfo] = useState("Loading...");
  const [engineSelectionState, setEngineSelectionState] = useState<EngineSelectionState | null>(null);
  const [isInitialLoading, setIsInitialLoading] = useState(true);
  const [loadingMessage, setLoadingMessage] = useState("");
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [isEnginePickerOpen, setIsEnginePickerOpen] = useState(false);
  const [settingsInitialTab, setSettingsInitialTab] = useState<string | undefined>(undefined);
  const [settingsScrollTarget, setSettingsScrollTarget] = useState<'whisper' | 'parakeet' | 'granite' | null>(null);
  /** null = not yet loaded from store; true = show wizard (first run); false = show main app */
  const [showSetupWizard, setShowSetupWizard] = useState<boolean | null>(null);
  /** Incremented after each successful save_transcript_history; tells TranscriptFeed to reload. */
  const [historyRefreshKey, setHistoryRefreshKey] = useState(0);
  /** Whether the output area is in file-transcription mode vs mic-recording mode */
  const [fileMode, setFileMode] = useState(false);
  /** True while FileTranscriptionPanel has a file actively transcribing */
  const [isFileTranscribing, setIsFileTranscribing] = useState(false);
  const [noModelCtaAttention, setNoModelCtaAttention] = useState(false);

  // macOS fix: Detect the runtime platform so we can hide/adjust UI elements
  // that don't apply on macOS (e.g. GPU/CPU toggle, VRAM display).
  const [platform, setPlatform] = useState('');
  // macOS fix: Track the two separate permissions involved in the hotkey flow.
  // Accessibility is needed for text insertion into other apps; Input Monitoring
  // is needed for the global keyboard listener to receive events system-wide.
  const [accessibilityMissing, setAccessibilityMissing] = useState(false);
  const [inputMonitoringMissing, setInputMonitoringMissing] = useState(false);
  // macOS fix: Track microphone permission so we can show a banner when denied.
  const [micPermission, setMicPermission] = useState<'granted' | 'denied' | 'undetermined' | null>(null);
  // Silence warning: shown when recording is active but no audio comes through
  // (mic muted, wrong device, hardware issue, etc.).
  const [showSilenceWarning, setShowSilenceWarning] = useState(false);
  const silenceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Active microphone name and full device list — shown as a dropdown on the
  // home view so the user can switch mics without opening Settings.
  const [activeMic, setActiveMic] = useState<string | null>(null);
  const [inputDevices, setInputDevices] = useState<string[]>([]);
  const inputDevicesRefreshRef = useRef({ inFlight: false, lastFetchAt: 0 });
  // Close-button behavior: 'tray' = hide to tray (default), 'quit' = exit process
  const [closeBehavior, setCloseBehavior] = useState<'tray' | 'quit'>('tray');
  useEffect(() => {
    invoke<string>('get_platform').then(setPlatform).catch(() => {});
  }, []);
  const isMac = platform === 'macos';

  const refreshInputDevices = useCallback((force = false) => {
    const now = Date.now();
    if (!force && now - inputDevicesRefreshRef.current.lastFetchAt < 1200) {
      return;
    }
    if (inputDevicesRefreshRef.current.inFlight) {
      return;
    }

    inputDevicesRefreshRef.current.inFlight = true;
    inputDevicesRefreshRef.current.lastFetchAt = now;
    invoke<string[]>('list_input_devices')
      .then(setInputDevices)
      .catch(() => {})
      .finally(() => {
        inputDevicesRefreshRef.current.inFlight = false;
      });
  }, []);

  // Fetch the active mic and the full device list on launch (all platforms).
  useEffect(() => {
    invoke<string>('get_active_input_device').then(setActiveMic).catch(() => {});
    refreshInputDevices(true);
  }, [refreshInputDevices]);

  // Handle mic selection from the hardware bar dropdown.
  const handleMicChange = useCallback(async (name: string) => {
    const value = name || null; // empty string = system default
    setActiveMic(name || null);
    try {
      await invoke('set_input_device', { name: value });
      const store = await Store.load('settings.json');
      if (value) { await store.set('input_device', value); }
      else { await store.delete('input_device'); }
      await store.save();
      // Re-resolve the actual device name (in case "default" mapped to a real name)
      invoke<string>('get_active_input_device').then(setActiveMic).catch(() => {});
      refreshInputDevices(true);
    } catch (e) { console.error('Failed to set input device:', e); }
  }, [refreshInputDevices]);

  const refreshMacPermissions = useCallback(async () => {
    if (!isMac) {
      setAccessibilityMissing(false);
      setInputMonitoringMissing(false);
      return;
    }

    const [micStatus, accessibilityGranted, inputMonitoringGranted] = await Promise.all([
      invoke<string>('check_microphone_permission').catch(() => null),
      invoke<boolean>('check_accessibility_permission').catch(() => true),
      invoke<boolean>('check_input_monitoring_permission').catch(() => true),
    ]);

    if (micStatus) {
      setMicPermission(micStatus as 'granted' | 'denied' | 'undetermined');
    }
    setAccessibilityMissing(!accessibilityGranted);
    setInputMonitoringMissing(!inputMonitoringGranted);
  }, [isMac]);

  useEffect(() => {
    void refreshMacPermissions();
  }, [refreshMacPermissions]);

  const [settingsModels, setSettingsModels] = useState<DownloadableModel[]>(MODELS);

  // --- Custom Hooks ---
  const { headerStatusMessage, headerStatusIsProcessing, setHeaderStatus } = useHeaderStatus();
  const {
    sessionState,
    setSessionPhase,
    setSessionNotice,
    setLastTranscript,
    setLatestLatency: setSessionLatency,
  } = useSessionState();
  useEffect(() => {
    if (micPermission !== "denied") return;
    setSessionNotice({
      level: "error",
      code: "mic_permission_denied",
      title: "Microphone permission is blocked",
      message: "Taurscribe cannot start recording until microphone access is granted in system settings.",
      sticky: true,
      actions: isMac ? [{
        id: "open-mic-settings",
        label: "Open Microphone Settings",
        onClick: () => { void invoke("open_microphone_settings"); },
      }] : undefined,
    });
  }, [micPermission, isMac, setSessionNotice]);
  const {
    models, setModels, currentModel, setCurrentModel,
    parakeetModels, setParakeetModels, currentParakeetModel, setCurrentParakeetModel,
    cohereModels, setCohereModels, currentCohereModel, setCurrentCohereModel,
    refreshModels,
  } = useModels(setHeaderStatus);

  // Factory: refreshes model status after a download event. `fallbackDownloaded`
  // is what we assume if the status check fails — true on success, false on failure.
  const makeDownloadStatusHandler = useCallback((fallbackDownloaded: boolean) => async (id: string) => {
    const [statuses] = await Promise.all([
      invoke<{ id: string; downloaded: boolean; verified: boolean }[]>("get_download_status", { modelIds: [id] }).catch(() => null),
      refreshModels(false),
    ]);
    const s = statuses?.find(x => x.id === id);
    setSettingsModels(prev => prev.map(m =>
      m.id === id ? { ...m, downloaded: s?.downloaded ?? fallbackDownloaded, verified: s?.verified ?? false } : m
    ));
  }, [refreshModels]);

  // Keep stable references so useDownloads doesn't re-subscribe its event
  // listener on every render (which would cause missed events).
  // NOTE: the ref is updated again after useEngineSwitch to include auto-load logic.
  const pendingAutoLoadModelIdRef = useRef<string | null>(null);
  const onModelDownloadedImpl = makeDownloadStatusHandler(true);
  const onModelDownloadedRef = useRef(onModelDownloadedImpl);
  const onModelDownloaded = useCallback((id: string) => onModelDownloadedRef.current(id), []);

  const onDownloadFailedImpl = useCallback(async (id: string) => {
    if (pendingAutoLoadModelIdRef.current === id) {
      pendingAutoLoadModelIdRef.current = null;
    }
    await makeDownloadStatusHandler(false)(id);
  }, [makeDownloadStatusHandler]);
  const onDownloadFailedRef = useRef(onDownloadFailedImpl);
  useEffect(() => {
    onDownloadFailedRef.current = onDownloadFailedImpl;
  }, [onDownloadFailedImpl]);
  const onDownloadFailed = useCallback((id: string) => onDownloadFailedRef.current(id), []);

  const { downloadProgress, handleDownload, handleCancelDownload } = useDownloads(onModelDownloaded, onDownloadFailed);
  const downloadProgressRef = useRef(downloadProgress);
  useEffect(() => { downloadProgressRef.current = downloadProgress; }, [downloadProgress]);


  const handleDownloadWithCoreml = (id: string, name: string) => {
    const engineForModel = getEngineForModelId(id);
    if (engineForModel) {
      pendingAutoLoadModelIdRef.current = id;
    }
    handleDownload(id, name);
  };

  const handleCancelDownloadWithSelection = (id: string) => {
    if (pendingAutoLoadModelIdRef.current === id) {
      pendingAutoLoadModelIdRef.current = null;
    }
    handleCancelDownload(id);
  };


  const {
    llmStatus, enableGrammarLM, setEnableGrammarLM, enableGrammarLMRef,
    enableDenoise, setEnableDenoise, enableDenoiseRef,
    enableOverlay, setEnableOverlay, enableOverlayRef,
    muteBackgroundAudio, setMuteBackgroundAudio, muteBackgroundAudioRef,
    transcriptionStyle, setTranscriptionStyle, transcriptionStyleRef,
    llmBackend, setLlmBackend,
    asrBackend, setAsrBackend,
  } = usePostProcessing(setHeaderStatus, () => setIsSettingsOpen(true), storeRef);

  /** Granite loaded on a GPU backend — lock header ASR toggle while active. */
  const [cohereGpuOnlyLoaded, setCohereGpuOnlyLoaded] = useState(false);

  const { volume, muted, setVolume, setMuted, playStart, playPaste, playError } = useSounds();

  const {
    dictionary, dictionaryRef, addDictEntry, updateDictEntry, removeDictEntry,
    snippets, snippetsRef, addSnippet, updateSnippet, removeSnippet,
  } = usePersonalization();

  // useEngineSwitch must be declared before useRecording is *used* but after
  // useRecording is *called* (hooks cannot be moved past each other in call order).
  // We use a forwarded ref so useEngineSwitch can populate activeEngineRef and
  // setLoadedEngine before any handler runs — this is safe because React
  // guarantees handlers only fire after everything renders.
  const activeEngineForwarded = useRef<ASREngine>("whisper");
  const setLoadedEngineForwarded = useRef<(e: ASREngine | null) => void>(() => {});

  const {
    isRecording, isRecordingRef, isPaused, isProcessingTranscript,
    latestLatency,
    handleStartRecording, handlePauseRecording, handleResumeRecording, handleStopRecording, handleCancelRecording, handleTranscriptionChunk, handlePartialChunk,
  } = useRecording({
    activeEngineRef: activeEngineForwarded,
    models, parakeetModels, cohereModels, currentModel, currentParakeetModel, currentCohereModel,
    asrBackend,
    setCurrentModel, setLoadedEngine: (e) => setLoadedEngineForwarded.current(e), enableGrammarLMRef,
    enableDenoiseRef, enableOverlayRef, muteBackgroundAudioRef, transcriptionStyleRef, setHeaderStatus, setTrayState, setIsSettingsOpen,
    playStart, playPaste, playError,
    dictionaryRef, snippetsRef,
    onHistorySaved: () => setHistoryRefreshKey(k => k + 1),
    setSessionPhase,
    setSessionNotice,
    setSessionTranscript: setLastTranscript,
    setSessionLatency,
  });

  const {
    activeEngine, setActiveEngine, activeEngineRef,
    loadedEngine, setLoadedEngine,
    isLoading, setIsLoading, isLoadingRef,
    loadingTargetEngine,
    handleModelChange, handleSwitchToWhisper, handleSwitchToParakeet, handleSwitchToCohere,
    handleToggleAsrBackend,
  } = useEngineSwitch({
    models, parakeetModels, cohereModels,
    currentModel, currentParakeetModel, currentCohereModel,
    setCurrentModel, setCurrentParakeetModel, setCurrentCohereModel,
    setBackendInfo, storeRef, setHeaderStatus, setTrayState, asrBackend,
    setAsrBackend,
    cohereGpuOnlyLocked: cohereGpuOnlyLoaded,
    isRecordingRef,
    downloadProgressRef,
    setSessionPhase,
    setSessionNotice,
  });

  useEffect(() => {
    let cancelled = false;
    if (loadedEngine !== "granite") {
      setCohereGpuOnlyLoaded(false);
      return () => { cancelled = true; };
    }
    invoke<{ loaded?: boolean; gpu_only?: boolean; backend?: string }>("get_granite_status")
      .then((s) => {
        if (!cancelled) {
          const locked = (!!s.loaded && !!s.gpu_only) || s.backend === "Hybrid";
          setCohereGpuOnlyLoaded(locked);
        }
      })
      .catch(() => {
        if (!cancelled) setCohereGpuOnlyLoaded(false);
      });
    return () => { cancelled = true; };
  }, [loadedEngine]);

  // Wire the forwarded refs so useRecording's handlers use the real values
  activeEngineForwarded.current = activeEngineRef.current;
  setLoadedEngineForwarded.current = setLoadedEngine;

  useEffect(() => {
    if (isLoading) {
      setSessionPhase("loading_model");
      return;
    }
    if (isProcessingTranscript) {
      setSessionPhase("processing");
      return;
    }
    if (isRecording) {
      setSessionPhase(isPaused ? "paused" : "recording");
      return;
    }
    if (["loading_model", "recording", "paused", "processing"].includes(sessionState.phase)) {
      setSessionPhase("idle");
    }
  }, [isLoading, isProcessingTranscript, isRecording, isPaused, sessionState.phase, setSessionPhase]);

  // handleDeleteModel moved here so setLoadedEngine is in scope
  const handleDeleteModel = async (id: string, _name: string) => {
    const isActiveModel = id === currentModel || id === currentParakeetModel || id === currentCohereModel;
    if (isFileTranscribing && isActiveModel) {
      throw new Error("Cannot delete the active model while a file is being transcribed.");
    }
    try {
      const result = await invoke<CommandResult<string>>("delete_model", { modelId: id });
      if (!result.ok) {
        throw new Error(result.error?.message ?? "Failed to delete model");
      }
      setSettingsModels(prev => prev.map(m => m.id === id ? { ...m, downloaded: false, verified: false } : m));
      if (currentModel === id || currentParakeetModel === id || currentCohereModel === id) {
        setLoadedEngine(null);
        setSessionNotice({
          level: "warning",
          code: "model_missing",
          title: "Active model removed",
          message: "The active model was deleted. Choose another installed model or switch engines before recording again.",
          sticky: true,
        });
      }
      if (currentModel === id) setCurrentModel(null);
      if (currentParakeetModel === id) setCurrentParakeetModel(null);
      if (currentCohereModel === id) setCurrentCohereModel(null);
      await refreshModels(false);
    } catch (e) {
      console.error("Failed to delete model", e);
      throw e;
    }
  };

  // ── Stable handler refs for useHotkeyListeners ──
  const handleStartRecordingRef = useSyncedRef(handleStartRecording);
  const handleStopRecordingRef = useSyncedRef(handleStopRecording);
  const handlePauseRecordingRef = useSyncedRef(handlePauseRecording);
  const handleResumeRecordingRef = useSyncedRef(handleResumeRecording);
  const handleCancelRecordingRef = useSyncedRef(handleCancelRecording);
  const handleTranscriptionChunkRef = useSyncedRef(handleTranscriptionChunk);
  const handlePartialChunkRef = useSyncedRef(handlePartialChunk);
  const asrModelCountsRef = useRef({
    whisper: 0,
    parakeet: 0,
    granite: 0,
  });
  asrModelCountsRef.current = {
    whisper: models.length,
    parakeet: parakeetModels.length,
    granite: cohereModels.length,
  };
  const isFileTranscribingRef = useSyncedRef(isFileTranscribing);
  const playErrorRef = useSyncedRef(playError);
  const setHeaderStatusRef = useSyncedRef(setHeaderStatus);
  const startNoModelCtaAttention = useCallback(() => {
    if (noModelCtaTimerRef.current !== null) {
      clearTimeout(noModelCtaTimerRef.current);
      noModelCtaTimerRef.current = null;
    }
    setNoModelCtaAttention(true);
    noModelCtaTimerRef.current = setTimeout(() => {
      noModelCtaTimerRef.current = null;
      setNoModelCtaAttention(false);
    }, 2600);
  }, []);
  const triggerNoModelAttention = useCallback(() => {
    if (appHiddenRef.current) {
      pendingNoModelCtaPulseRef.current = true;
      return;
    }

    pendingNoModelCtaPulseRef.current = false;
    setFileMode(false);
    startNoModelCtaAttention();
  }, [startNoModelCtaAttention]);
  const triggerNoModelAttentionRef = useSyncedRef(triggerNoModelAttention);

  useEffect(() => {
    return () => {
      if (noModelCtaTimerRef.current !== null) {
        clearTimeout(noModelCtaTimerRef.current);
      }
    };
  }, []);

  // Re-randomize the logo animation when the window is restored from the tray
  useEffect(() => {
    const unlisten = listen("window-restored", () => {
      setRandomLogo(pickRandomLogo());
      appHiddenRef.current = false;
      if (pendingNoModelCtaPulseRef.current) {
        pendingNoModelCtaPulseRef.current = false;
        setFileMode(false);
        startNoModelCtaAttention();
      }
    });
    return () => { unlisten.then(fn => fn()); };
  }, [pickRandomLogo, startNoModelCtaAttention]);

  // ── Hooks extracted from App.tsx ──
  useInitialLoad({
    setModels, setCurrentModel,
    setParakeetModels, setCurrentParakeetModel,
    setCohereModels, setCurrentCohereModel,
    setSettingsModels,
    setLoadedEngine, setActiveEngine, activeEngineRef,
    isLoadingRef, setIsLoading, setLoadingMessage,
    setBackendInfo, setHeaderStatus,
    setShowSetupWizard, setIsInitialLoading,
    setCloseBehavior,
    storeRef,
  });

  useHotkeyListeners({
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
    handlePartialChunkRef,
    playErrorRef,
    setHeaderStatusRef,
    triggerNoModelAttentionRef,
    setLoadedEngine,
    silenceTimerRef,
    setShowSilenceWarning,
    refreshMacPermissions,
  });

  useModelsWatcher({ refreshModels, downloadProgressRef, setSettingsModels });

  // ── Small helpers (local, use hook outputs) ──
  const handleEjectModel = async () => {
    if (isLoading || isLoadingRef.current || isRecording) return;
    try {
      setHeaderStatus("Unloading model…", 10_000);
      const result = await invoke<CommandResult<string>>("unload_current_model");
      if (!result.ok) {
        throw new Error(result.error?.message ?? "Failed to unload model");
      }
      setLoadedEngine(null);
      setHeaderStatus("Model unloaded — VRAM freed");
      setSessionNotice({
        level: "warning",
        code: "model_missing",
        title: "Model unloaded",
        message: "VRAM was freed. The next dictation will reload the selected model before recording.",
        sticky: true,
      });
      try {
        const backend = await invoke<string>("get_backend_info");
        setBackendInfo(backend);
      } catch {
        /* keep previous hardware line */
      }
      await setTrayState("ready");
    } catch (e) {
      setHeaderStatus(`Failed to unload: ${e}`, 4000);
    }
  };

  const handleLoadActiveEngine = () => {
    if (activeEngine === "whisper") void handleSwitchToWhisper();
    else if (activeEngine === "parakeet") void handleSwitchToParakeet();
    else void handleSwitchToCohere();
  };

  const refreshEngineSelectionState = useCallback(() => {
    invoke<EngineSelectionState>("get_engine_selection_state")
      .then(setEngineSelectionState)
      .catch(() => {});
  }, []);

  useEffect(() => {
    refreshEngineSelectionState();
    const timer = window.setInterval(refreshEngineSelectionState, 4000);
    return () => window.clearInterval(timer);
  }, [refreshEngineSelectionState]);

  useEffect(() => {
    refreshEngineSelectionState();
  }, [
    refreshEngineSelectionState,
    activeEngine,
    loadedEngine,
    currentModel,
    currentParakeetModel,
    currentCohereModel,
    backendInfo,
    isLoading,
    isRecording,
    isProcessingTranscript,
  ]);

  // Auto-load newly downloaded model if it matches the active engine.
  // Status refresh is delegated to makeDownloadStatusHandler to avoid duplication.
  useEffect(() => {
    onModelDownloadedRef.current = async (id: string) => {
      // Reuse the factory for the status-refresh step (invoke + refreshModels + setSettingsModels)
      await makeDownloadStatusHandler(true)(id);

      const engineForModel = getEngineForModelId(id);
      const isExplicitSelection = pendingAutoLoadModelIdRef.current === id;
      if (isExplicitSelection) pendingAutoLoadModelIdRef.current = null;

      if (engineForModel && engineForModel === activeEngineRef.current && !isLoadingRef.current) {
        if (isExplicitSelection) {
          if (engineForModel === 'whisper') await handleModelChange(id);
          else if (engineForModel === 'parakeet') await handleSwitchToParakeet(id);
          else await handleSwitchToCohere(id);
          return;
        }
        if (loadedEngine) return;
        if (engineForModel === 'whisper') handleModelChange(id);
        else if (engineForModel === 'parakeet') handleSwitchToParakeet(id);
        else handleSwitchToCohere(id);
      }
    };
  }, [getEngineForModelId, handleModelChange, handleSwitchToCohere, handleSwitchToParakeet, loadedEngine, refreshModels]);






  // Clear silence warning + any pending timer when recording ends
  useEffect(() => {
    if (!isRecording) {
      if (silenceTimerRef.current) {
        clearTimeout(silenceTimerRef.current);
        silenceTimerRef.current = null;
      }
      setShowSilenceWarning(false);
    }
  }, [isRecording]);

  // --- Derived UI state ---
  const noWhisperModel = models.length === 0;
  const noParakeetModel = parakeetModels.length === 0;
  const noCohereModel = cohereModels.length === 0;
  const noAnyAsrModel = noWhisperModel && noParakeetModel && noCohereModel;
  const activeEngineHasNoModel =
    (activeEngine === "whisper" && noWhisperModel) ||
    (activeEngine === "parakeet" && noParakeetModel) ||
    (activeEngine === "granite" && noCohereModel);
  const noModel = activeEngineHasNoModel;
  const noLlm = llmStatus === "Not Downloaded";
  const downloadProgressKeys = useMemo(() => Object.keys(downloadProgress), [downloadProgress]);
  const isWhisperDownloading = useMemo(
    () => downloadProgressKeys.some((key) => key.startsWith("whisper-")),
    [downloadProgressKeys],
  );
  const isParakeetDownloading = useMemo(
    () => downloadProgressKeys.some((key) => key.startsWith("parakeet")),
    [downloadProgressKeys],
  );
  const isCohereDownloading = useMemo(
    () => downloadProgressKeys.some((key) => key.startsWith("granite") || key.startsWith("cohere")),
    [downloadProgressKeys],
  );
  const recordBtnBusy = isLoading || isProcessingTranscript;
  const recordBtnClass =
    noModel ? "record-btn disabled" :
      isFileTranscribing ? "record-btn disabled" :
        isRecording ? "record-btn recording" :
          recordBtnBusy ? "record-btn processing" :
            "record-btn idle";
  const recordBtnLabel =
    noModel ? "NO MODEL" :
      isFileTranscribing ? "BUSY" :
        isRecording ? "STOP" :
          recordBtnBusy ? "..." : "REC";
  const recordBtnDisabled = isFileTranscribing || (isLoading && !isRecording) || isProcessingTranscript;

  const onRecordClick = () => {
    if (noModel) { setIsSettingsOpen(true); return; }
    if (isRecording) handleStopRecording();
    else handleStartRecording();
  };

  const engineChipMeta = useMemo(() => {
    if (activeEngine === "whisper") {
      const label = "Whisper";
      const color = "var(--whisper-color)";
      if (isLoading && loadingTargetEngine === "whisper") return { label, color, model: "Loading…" };
      if (isWhisperDownloading) return { label, color, model: "Downloading…" };
      if (models.length === 0) return { label, color, model: "No model" };
      const m = models.find(x => x.id === currentModel);
      return { label, color, model: m ? beautifyModelName(m.display_name) : "None" };
    }
    if (activeEngine === "parakeet") {
      const label = "Parakeet";
      const color = "var(--parakeet-color)";
      if (isLoading && loadingTargetEngine === "parakeet") return { label, color, model: "Loading…" };
      if (isParakeetDownloading) return { label, color, model: "Downloading…" };
      if (parakeetModels.length === 0) return { label, color, model: "No model" };
      const m = parakeetModels.find(x => x.id === currentParakeetModel) ?? parakeetModels[0];
      return { label, color, model: beautifyModelName(m.display_name) };
    }
    const label = "Granite";
    const color = "var(--cohere-color)";
    if (isLoading && loadingTargetEngine === "granite") return { label, color, model: "Loading…" };
    if (isCohereDownloading) return { label, color, model: "Downloading…" };
    if (cohereModels.length === 0) return { label, color, model: "No model" };
    const m = cohereModels.find(x => x.id === currentCohereModel) ?? cohereModels[0];
    return { label, color, model: m.display_name };
  }, [activeEngine, isLoading, loadingTargetEngine, isWhisperDownloading, isParakeetDownloading, isCohereDownloading,
      models, currentModel, parakeetModels, currentParakeetModel, cohereModels, currentCohereModel]);

  const recordReadinessMeta = useMemo(() => {
    const loadedEngineName = engineSelectionState?.loaded_engine as ASREngine | null | undefined;
    const activeModelLoaded = loadedEngineName === activeEngine && !!engineSelectionState?.loaded_model_id;
    const selectedOrLoadedModelId =
      activeModelLoaded
        ? engineSelectionState?.loaded_model_id
        : engineSelectionState?.selected_model_id;
    const findModelName = () => {
      if (activeEngine === "whisper") {
        const m = models.find(x => x.id === selectedOrLoadedModelId) ?? models.find(x => x.id === currentModel);
        return m ? beautifyModelName(m.display_name) : engineChipMeta.model;
      }
      if (activeEngine === "parakeet") {
        const m = parakeetModels.find(x => x.id === selectedOrLoadedModelId) ?? parakeetModels.find(x => x.id === currentParakeetModel) ?? parakeetModels[0];
        return m ? beautifyModelName(m.display_name) : engineChipMeta.model;
      }
      const m = cohereModels.find(x => x.id === selectedOrLoadedModelId) ?? cohereModels.find(x => x.id === currentCohereModel) ?? cohereModels[0];
      return m ? beautifyModelName(m.display_name) : engineChipMeta.model;
    };

    const backend = activeModelLoaded
      ? (engineSelectionState?.backend || backendInfo || "Unknown")
      : asrBackend === "gpu"
        ? "GPU pref"
        : "CPU pref";
    const backendKey = backend.toLowerCase().replace(/[^a-z0-9]+/g, "-");
    const phase = isLoading || engineSelectionState?.engine_loading
      ? "LOADING"
      : isRecording
        ? "RECORDING"
        : isProcessingTranscript
          ? "PROCESSING"
          : noModel
            ? "NO MODEL"
            : activeModelLoaded
              ? "READY"
              : "LOAD REQUIRED";

    return {
      phase,
      backend,
      backendKey,
      model: findModelName(),
      activeModelLoaded,
    };
  }, [
    engineSelectionState,
    activeEngine,
    models,
    currentModel,
    parakeetModels,
    currentParakeetModel,
    cohereModels,
    currentCohereModel,
    engineChipMeta.model,
    backendInfo,
    asrBackend,
    isLoading,
    isRecording,
    isProcessingTranscript,
    noModel,
  ]);

  const handleOpenSettingsTab = useCallback((tab?: string) => {
    setSettingsInitialTab(tab);
    setIsSettingsOpen(true);
  }, []);

  const openModelSettingsForEngine = useCallback((engine: 'whisper' | 'parakeet' | 'granite') => {
    setSettingsInitialTab('models');
    setSettingsScrollTarget(engine);
    setIsSettingsOpen(true);
  }, []);

  const handleCloseSettings = useCallback(() => {
    setIsSettingsOpen(false);
    // Refresh the mic dropdown in case the user changed the device in Settings.
    invoke<string>('get_active_input_device').then(setActiveMic).catch(() => {});
    refreshInputDevices(true);
  }, [refreshInputDevices]);

  useEffect(() => {
    if (!activeEngineHasNoModel && sessionState.notice?.code === "model_missing") {
      setSessionNotice(null);
    }
  }, [activeEngineHasNoModel, sessionState.notice?.code, setSessionNotice]);

  const colorizedStatus = useMemo(() => {
    const msg = headerStatusMessage ?? "";
    const parts = msg.split(/(Granite Speech|Granite|Whisper|Parakeet|OpenAI|NVIDIA)/g);
    return parts.map((part, i) => {
      if (part === "Whisper" || part === "OpenAI") return <span key={i} style={{ color: 'var(--whisper-color)' }}>{part}</span>;
      if (part === "Parakeet" || part === "NVIDIA") return <span key={i} style={{ color: 'var(--parakeet-color)' }}>{part}</span>;
      if (part === "Granite Speech" || part === "Granite") return <span key={i} style={{ color: 'var(--cohere-color)' }}>{part}</span>;
      return part;
    });
  }, [headerStatusMessage]);

  const handleSetupComplete = useCallback(({ openSettings, useCase }: { openSettings: boolean; useCase: OnboardingUseCase }) => {
    storeRef.current?.set("setup_complete", true);
    storeRef.current?.set("onboarding_use_case", useCase);
    storeRef.current?.save().catch(console.error);
    setShowSetupWizard(false);
    if (openSettings) {
      setSettingsInitialTab("models");
      setIsSettingsOpen(true);
    }
  }, []);

  if (showSetupWizard === null) {
    return (
      <div className="app-loading" style={{ minHeight: "100vh", display: "flex", alignItems: "center", justifyContent: "center", background: "var(--bg-primary, #000000)", color: "var(--text-secondary)" }}>
        Loading…
      </div>
    );
  }

  if (showSetupWizard === true) {
    return (
      <SetupWizard
        onComplete={handleSetupComplete}
        handleDownload={handleDownloadWithCoreml}
        handleCancelDownload={handleCancelDownloadWithSelection}
        downloadProgress={downloadProgress}
        settingsModels={settingsModels}
        enableDenoise={enableDenoise}
        setEnableDenoise={setEnableDenoise}
        enableOverlay={enableOverlay}
        setEnableOverlay={setEnableOverlay}
        muteBackgroundAudio={muteBackgroundAudio}
        setMuteBackgroundAudio={setMuteBackgroundAudio}
      />
    );
  }

  return (
    <>
      <TitleBar
        logoSrc={`/logos/${randomLogo}`}
        isLogoShuttering={isLogoShuttering}
        onLogoClick={handleLogoClick}
      />
      <div className={`app-body ${isRecording ? "app-body--recording" : ""} theme-${activeEngine}`}>
        <main className={`container${containerBooting ? " container--booting" : ""}`}>
          <div>
            <div className="app-header">
              <div className="header-status">
                {headerStatusMessage !== null && (
                  <span
                    className={`header-status-message ${headerStatusIsProcessing ? "header-status-message--processing" : ""}`}
                    key={headerStatusMessage}
                  >
                    {colorizedStatus}
                  </span>
                )}
              </div>
            </div>
            {/* macOS fix: Show a warning banner when the hotkey pipeline is missing
                Input Monitoring and/or Accessibility permission. */}
            {isMac && (accessibilityMissing || inputMonitoringMissing) && (
              <div className="accessibility-banner">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={{ flexShrink: 0 }}>
                  <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
                  <line x1="12" y1="9" x2="12" y2="13" />
                  <line x1="12" y1="17" x2="12.01" y2="17" />
                </svg>
                <span>
                  {inputMonitoringMissing && accessibilityMissing ? (
                    <>Use the buttons below: <strong>Input Monitoring</strong> for the global shortcut, then <strong>Accessibility</strong> to paste into other apps.</>
                  ) : inputMonitoringMissing ? (
                    <>Use <strong>Input Monitoring</strong> below — the shortcut will not work until this is enabled for Taurscribe.</>
                  ) : (
                    <>Use <strong>Accessibility</strong> below — otherwise text cannot be inserted into other apps.</>
                  )}
                </span>
                <div className="accessibility-banner-actions">
                  {inputMonitoringMissing && (
                    <>
                      <button
                        type="button"
                        className="accessibility-banner-action"
                        onClick={async () => {
                          await invoke<boolean>('request_input_monitoring_permission').catch(() => false);
                          await invoke('open_input_monitoring_settings').catch(() => {});
                          setTimeout(() => { void refreshMacPermissions(); }, 700);
                        }}
                      >
                        Enable Input Monitoring
                      </button>
                    </>
                  )}
                  {accessibilityMissing && (
                    <button
                      type="button"
                      className="accessibility-banner-action"
                      onClick={async () => {
                        await invoke<boolean>('request_accessibility_permission').catch(() => false);
                        await invoke('open_accessibility_settings').catch(() => {});
                        setTimeout(() => { void refreshMacPermissions(); }, 700);
                      }}
                    >
                      Enable Accessibility
                    </button>
                  )}
                  <button
                    type="button"
                    className="accessibility-banner-action"
                    onClick={async () => {
                      await invoke('relaunch_app').catch(() => {});
                    }}
                  >
                    Restart App
                  </button>
                </div>
                <button
                  type="button"
                  className="accessibility-banner-dismiss"
                  onClick={() => {
                    setAccessibilityMissing(false);
                    setInputMonitoringMissing(false);
                  }}
                  aria-label="Dismiss"
                >
                  ✕
                </button>
              </div>
            )}

            {/* macOS fix: Show a banner when microphone permission is not granted.
                "undetermined" → prompt the user to grant access (triggers the OS dialog).
                "denied" → direct the user to System Settings. */}
            {isMac && micPermission && micPermission !== 'granted' && (
              <div className="mic-banner">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={{ flexShrink: 0 }}>
                  <path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z" />
                  <path d="M19 10v2a7 7 0 0 1-14 0v-2" />
                  <line x1="12" y1="19" x2="12" y2="23" />
                  <line x1="8" y1="23" x2="16" y2="23" />
                </svg>
                {micPermission === 'undetermined' ? (
                  <span>
                    Microphone access is required for recording.{' '}
                    <button
                      type="button"
                      className="mic-banner-action"
                      onClick={async () => {
                        await invoke<string>('request_microphone_permission');
                        // Re-check with a fresh AVFoundation status query — the
                        // request call triggers the dialog but its return value
                        // can race with the OS updating the authorization status.
                        const status = await invoke<string>('check_microphone_permission');
                        setMicPermission(status as 'granted' | 'denied' | 'undetermined');
                      }}
                    >
                      Grant Access
                    </button>
                  </span>
                ) : (
                  <span>
                    Microphone access denied — open <strong>System Settings → Privacy &amp; Security → Microphone</strong> and enable Taurscribe, then restart the app.
                    {' '}
                    <button
                      type="button"
                      className="mic-banner-action"
                      onClick={async () => {
                        await invoke('open_microphone_settings').catch(() => {});
                      }}
                    >
                      Open Settings
                    </button>
                  </span>
                )}
                <button type="button" className="mic-banner-dismiss" onClick={() => setMicPermission(null)} aria-label="Dismiss">✕</button>
              </div>
            )}

            {showSilenceWarning && isRecording && !isPaused && (
              <div className="silence-banner" role="alert">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={{ flexShrink: 0 }}>
                  <line x1="1" y1="1" x2="23" y2="23" />
                  <path d="M9 9v3a3 3 0 0 0 5.12 2.12M15 9.34V4a3 3 0 0 0-5.94-.6" />
                  <path d="M17 16.95A7 7 0 0 1 5 12v-2m14 0v2a7 7 0 0 1-.11 1.23" />
                  <line x1="12" y1="19" x2="12" y2="23" />
                  <line x1="8" y1="23" x2="16" y2="23" />
                </svg>
                <span>No audio detected — is your mic muted or the wrong device selected?</span>
                <button type="button" className="silence-banner-dismiss" onClick={() => setShowSilenceWarning(false)} aria-label="Dismiss">✕</button>
              </div>
            )}
          </div>

          {/* Mic / File mode toggle — top-left, directly under the header */}
          <div className="mode-toggle">
            <button
              type="button"
              className={`mode-toggle-btn${!fileMode ? " mode-toggle-btn--active" : ""}`}
              onClick={() => setFileMode(false)}
              disabled={fileMode && isFileTranscribing}
              title={fileMode && isFileTranscribing ? "Wait for file transcription to finish" : undefined}
            >
              <IconMic size={13} /> Mic
            </button>
            <button
              type="button"
              className={`mode-toggle-btn${fileMode ? " mode-toggle-btn--active" : ""}`}
              onClick={() => setFileMode(true)}
            >
              <IconFileText size={13} /> Files
            </button>
          </div>

          {sessionState.notice && (
            <SessionNoticeCard notice={sessionState.notice} />
          )}

          {isInitialLoading && (
            <div className="loading-overlay-backdrop" aria-busy="true" aria-live="polite">
              <div className="loading-overlay">
                <div className="loading-spinner" />
                <span className="loading-text">{loadingMessage || "Loading model…"}</span>
              </div>
            </div>
          )}

          <OverlayScrollbarsComponent
            className="output-area output-area--feed"
            options={{
              scrollbars: { theme: "os-theme-pure", autoHide: "move", autoHideDelay: 400 },
              overflow: { x: "hidden" },
            }}
            defer
          >
            <div style={fileMode ? undefined : { display: 'none' }}>
              <FileTranscriptionPanel
                activeEngine={activeEngine}
                currentModel={currentModel}
                currentParakeetModel={currentParakeetModel}
                currentCohereModel={currentCohereModel}
                isModelLoading={isLoading}
                onFileProcessingChange={setIsFileTranscribing}
              />
            </div>
            {!fileMode && (activeEngineHasNoModel ? (
              <div className="empty-state">
                <div className="empty-state-icon" aria-hidden="true">
                  {noAnyAsrModel ? <IconDownload size={32} /> : activeEngine === "whisper" ? <IconMic size={32} /> : <IconBolt size={32} style={{ color: '#facc15' }} />}
                </div>
                <h2 className="empty-state-title">
                  {noAnyAsrModel
                    ? "No speech model downloaded"
                    : activeEngine === "whisper"
                      ? "No Whisper model downloaded"
                      : activeEngine === "parakeet"
                        ? "Parakeet not downloaded"
                        : "Granite not downloaded"}
                </h2>
                <p className="empty-state-body">
                  {noAnyAsrModel ? (
                    <>Download a <strong>Whisper</strong>, <strong>Parakeet</strong>, or <strong>Granite</strong> model to start transcribing. Whisper Base is a good starting point — it's fast and accurate.</>
                  ) : activeEngine === "whisper" ? (
                    <>You're on the <strong>Whisper</strong> engine but haven't downloaded a model yet. Try <strong>Whisper Base</strong> — it's small and accurate. Or switch to Parakeet if you already have it.</>
                  ) : activeEngine === "parakeet" ? (
                    <>You're on the <strong>Parakeet</strong> engine but the Nemotron Streaming model isn't downloaded yet. Switch to Whisper if you already have a model, or download Parakeet from Settings.</>
                  ) : (
                    <>You're on the <strong>Granite</strong> engine but the model isn't downloaded yet. Switch to Whisper or Parakeet if you already have a model, or download Granite from Settings.</>
                  )}
                </p>
                {!noAnyAsrModel && (
                  <p className="empty-state-hint">
                    {activeEngine === "whisper" && !noParakeetModel
                      ? <><IconLightbulb size={14} /> You already have a Parakeet model — click the Parakeet card above to switch.</>
                      : activeEngine === "parakeet" && !noWhisperModel
                        ? <><IconLightbulb size={14} /> You already have a Whisper model — click the Whisper card above to switch.</>
                        : activeEngine === "granite" && !noWhisperModel
                          ? <><IconLightbulb size={14} /> You already have a Whisper model — click the Whisper card above to switch.</>
                          : null}
                  </p>
                )}
                <button
                  type="button"
                  className={`empty-state-cta${noModelCtaAttention ? " empty-state-cta--attention" : ""}`}
                  onClick={() => {
                    setNoModelCtaAttention(false);
                    openModelSettingsForEngine(activeEngine as 'whisper' | 'parakeet' | 'granite');
                  }}
                >
                  Open Settings → Download Models
                </button>
                {noLlm && (
                  <p className="empty-state-llm-hint">
                    <span className="empty-state-llm-dot" />FlowScribe grammar LLM also not downloaded — optional but improves quality.
                  </p>
                )}
              </div>
            ) : (
              <TranscriptFeed
                refreshKey={historyRefreshKey}
                isRecording={isRecording}
                isPaused={isPaused}
                isProcessingTranscript={isProcessingTranscript}
                latestLatency={sessionState.latestLatency ?? latestLatency}
              />
            ))}
          </OverlayScrollbarsComponent>

          <div className="bottom-bar">
            <div className="bottom-left">
              {/* Microphone selector — lists all available input devices;
                  selecting one persists the choice to settings.json. */}
              <div className="bottom-left-status-row">
                <div className="mic-selector-bar">
                  <svg className="mic-selector-icon" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z" />
                    <path d="M19 10v2a7 7 0 0 1-14 0v-2" />
                    <line x1="12" y1="19" x2="12" y2="23" />
                    <line x1="8" y1="23" x2="16" y2="23" />
                  </svg>
                  {/* H5 fix: aria-label names the control for screen readers */}
                  <select
                    className="mic-selector-dropdown"
                    aria-label="Input device"
                    value={activeMic ?? ''}
                    onChange={(e) => handleMicChange(e.target.value)}
                    onFocus={() => refreshInputDevices(false)}
                    onMouseEnter={() => refreshInputDevices(false)}
                  >
                    <option value="">System Default</option>
                    {inputDevices.map((d) => (
                      <option key={d} value={d}>{d}</option>
                    ))}
                  </select>
                </div>

              </div>

              <button
                type="button"
                className="engine-chip"
                onClick={() => setIsEnginePickerOpen(o => !o)}
                aria-label="Switch engine or model"
                aria-expanded={isEnginePickerOpen}
              >
                <span
                  className={`eng-status-dot eng-status-dot--${
                    isLoading && loadingTargetEngine === activeEngine ? "loading" :
                      isProcessingTranscript ? "processing" :
                        loadedEngine === activeEngine ? "loaded" : "unloaded"
                  }`}
                  aria-hidden="true"
                />
                <span className="eng-chip-text">{engineChipMeta.label} · {engineChipMeta.model}</span>
                <span className={`eng-chip-backend eng-chip-backend--${recordReadinessMeta.backendKey}`}>
                  {recordReadinessMeta.backend}
                </span>
                <span className="eng-chip-caret" aria-hidden="true">▾</span>
              </button>

              {/* Load / unload toggle — hidden while busy, or while the active
                  engine has no installed model to load. */}
              {!isLoading && !isRecording && !isProcessingTranscript && (
                loadedEngine === activeEngine ? (
                  <button
                    type="button"
                    className="load-eject-btn"
                    onClick={handleEjectModel}
                    title="Unload model (free VRAM)"
                    aria-label="Unload model"
                  >
                    <IconEject size={14} />
                  </button>
                ) : (
                  (activeEngine === "whisper" ? !noWhisperModel :
                   activeEngine === "parakeet" ? !noParakeetModel :
                   !noCohereModel) && (
                    <button
                      type="button"
                      className="load-eject-btn load-eject-btn--load"
                      onClick={handleLoadActiveEngine}
                      title="Load model"
                      aria-label="Load model"
                    >
                      <IconBolt size={14} />
                    </button>
                  )
                )
              )}

              {isEnginePickerOpen && (
                <EnginePicker
                  activeEngine={activeEngine}
                  loadedEngine={loadedEngine}
                  loadingTargetEngine={loadingTargetEngine}
                  models={models}
                  currentModel={currentModel}
                  parakeetModels={parakeetModels}
                  currentParakeetModel={currentParakeetModel}
                  cohereModels={cohereModels}
                  currentCohereModel={currentCohereModel}
                  downloadProgress={downloadProgress}
                  isWhisperDownloading={isWhisperDownloading}
                  isParakeetDownloading={isParakeetDownloading}
                  isCohereDownloading={isCohereDownloading}
                  disabled={isRecording || isFileTranscribing}
                  onSelectWhisperModel={(id) => handleModelChange(id)}
                  onSelectParakeetModel={(id) => { void handleSwitchToParakeet(id); }}
                  onSelectCohereModel={(id) => { void handleSwitchToCohere(id); }}
                  onUnload={handleEjectModel}
                  onOpenDownloads={openModelSettingsForEngine}
                  onClose={() => setIsEnginePickerOpen(false)}
                />
              )}
            </div>

            <div className="record-btn-wrap">
              <button
                type="button"
                className={recordBtnClass}
                disabled={!noModel && recordBtnDisabled}
                onClick={onRecordClick}
                title={noModel ? "Download a model first in Settings" : isFileTranscribing ? "Cannot record while a file is being transcribed" : recordBtnBusy ? "Please wait…" : isRecording ? "Stop recording" : "Start recording"}
              >
                {recordBtnLabel}
              </button>
            </div>

            <button
              type="button"
              className="settings-btn"
              onClick={() => setIsSettingsOpen(true)}
              title="Settings"
              aria-label="Settings"
            >
              <IconSettings size={20} />
            </button>
          </div>

          <SettingsModal
            isOpen={isSettingsOpen}
            onClose={handleCloseSettings}
            initialTab={settingsInitialTab as Parameters<typeof SettingsModal>[0]['initialTab']}
            scrollTarget={settingsScrollTarget ?? undefined}
            onScrollHandled={() => setSettingsScrollTarget(null)}
            enableGrammarLM={enableGrammarLM}
            setEnableGrammarLM={setEnableGrammarLM}
            llmStatus={llmStatus}

            enableDenoise={enableDenoise}
            setEnableDenoise={setEnableDenoise}
            muteBackgroundAudio={muteBackgroundAudio}
            setMuteBackgroundAudio={setMuteBackgroundAudio}
            enableOverlay={enableOverlay}
            setEnableOverlay={setEnableOverlay}
            transcriptionStyle={transcriptionStyle}
            setTranscriptionStyle={setTranscriptionStyle}
            llmBackend={llmBackend}
            setLlmBackend={setLlmBackend}
            soundVolume={volume}
            soundMuted={muted}
            setSoundVolume={setVolume}
            setSoundMuted={setMuted}
            dictionary={dictionary}
            addDictEntry={addDictEntry}
            updateDictEntry={updateDictEntry}
            removeDictEntry={removeDictEntry}
            snippets={snippets}
            addSnippet={addSnippet}
            updateSnippet={updateSnippet}
            removeSnippet={removeSnippet}
            settingsModels={settingsModels}
            downloadProgress={downloadProgress}
            onDownload={handleDownloadWithCoreml}
            onDelete={handleDeleteModel}
            onCancelDownload={handleCancelDownloadWithSelection}
            closeBehavior={closeBehavior}
            setCloseBehavior={setCloseBehavior}
          />
        </main>

        <QuickSettings
          enableGrammarLM={enableGrammarLM}
          setEnableGrammarLM={setEnableGrammarLM}
          llmStatus={llmStatus}
          enableDenoise={enableDenoise}
          setEnableDenoise={setEnableDenoise}
          enableOverlay={enableOverlay}
          setEnableOverlay={setEnableOverlay}
          muteBackgroundAudio={muteBackgroundAudio}
          setMuteBackgroundAudio={setMuteBackgroundAudio}
          llmBackend={llmBackend}
          setLlmBackend={setLlmBackend}
          transcriptionStyle={transcriptionStyle}
          setTranscriptionStyle={setTranscriptionStyle}
          backendInfo={backendInfo}
          asrBackend={asrBackend}
          onToggleAsrBackend={handleToggleAsrBackend}
          asrBackendLoading={isLoading}
          cohereGpuOnlyLoaded={cohereGpuOnlyLoaded}
          activeEngine={activeEngine}
          soundVolume={volume}
          soundMuted={muted}
          setSoundVolume={setVolume}
          setSoundMuted={setMuted}
          dictionaryCount={dictionary.length}
          snippetsCount={snippets.length}
          onOpenSettingsTab={handleOpenSettingsTab}
        />
      </div>
    </>
  );
}

export default App;
