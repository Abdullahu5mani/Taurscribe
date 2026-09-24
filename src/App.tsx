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
import { MeetingBanner } from "./components/MeetingBanner";
import { MeetingsPanel } from "./components/MeetingsPanel";
import { useWindowLayout } from "./hooks/useWindowLayout";
import { levelToPercent } from "./utils/audioLevel";
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
import { IconFileText, IconBolt, IconEject, IconDownload, IconMic, IconVideo, IconLightbulb, IconSettings } from "./components/Icons";
import { getEngineForModelId } from "./utils/engineUtils";
import { useAutoUnload, AUTO_UNLOAD_OPTIONS, formatTimeoutLabel, formatRemaining } from "./hooks/useAutoUnload";
import { useSlidingIndicator } from "./hooks/useSlidingIndicator";
import { useDismissOnOutside } from "./hooks/useDismissOnOutside";
import { OverlayScrollbarsComponent } from "overlayscrollbars-react";
import type { CommandResult } from "./types/session";


type EngineSelectionState = {
  active_engine: string;
  selected_model_id: string | null;
  loaded_engine: string | null;
  loaded_model_id: string | null;
  backend: string;
  engine_loading: boolean;
};




import type { MeetingInfo } from "./components/MeetingHeaderPill";

let currentActiveMeeting: MeetingInfo | null = null;

/** Tray states; the processing ones say what is being processed. */
export type TrayState =
  | "ready" | "recording" | "paused" | "loading_model" | "downloading"
  | "processing_speech" | "processing_meeting" | "processing_file" | "grammar"
  | "done" | "nothing_heard" | "paste_failed" | "error" | "mic_blocked" | "cancelled";

/** Sets the tray icon. `detail` shows in its tooltip (and, for errors, the tray menu).
 *  Short-lived states (done, nothing heard, errors…) return to idle on their own. */
const setTrayState = async (
  newState: TrayState,
  meetingOverride?: MeetingInfo | null,
  detail?: string,
) => {
  try {
    const meeting = meetingOverride !== undefined ? meetingOverride : currentActiveMeeting;
    await invoke("set_tray_state", {
      newState,
      meetingPlatform: meeting?.platform || null,
      meetingProcess: meeting?.app_name || null,
      meetingPid: meeting?.pid || null,
      detail: detail ?? null,
    });
  } catch (e) {
    console.error("Failed to set tray state:", e);
  }
};


function App() {
  useWindowLayout();
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
  const autoUnloadMenuRef = useRef<HTMLDivElement>(null);
  const modeToggleRef = useRef<HTMLDivElement>(null);
  const [settingsInitialTab, setSettingsInitialTab] = useState<string | undefined>(undefined);
  const [settingsScrollTarget, setSettingsScrollTarget] = useState<ASREngine | null>(null);
  /** null = not yet loaded from store; true = show wizard (first run); false = show main app */
  const [showSetupWizard, setShowSetupWizard] = useState<boolean | null>(null);
  /** Incremented after each successful save_transcript_history; tells TranscriptFeed to reload. */
  const [historyRefreshKey, setHistoryRefreshKey] = useState(0);
  type NavMode = "mic" | "meetings" | "files";
  /** Whether the output area is in mic, meetings, or file-transcription mode */
  const [navMode, setNavMode] = useState<NavMode>("mic");
  const modeIndicator = useSlidingIndicator(modeToggleRef, ".mode-toggle-btn--active", navMode);
  const setFileMode = (files: boolean) => setNavMode(files ? "files" : "mic");
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
    graniteModels, setGraniteModels, currentGraniteModel, setCurrentGraniteModel,
    qwen3Models, setQwen3Models, currentQwen3Model, setCurrentQwen3Model,
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


  const { volume, muted, setVolume, setMuted, playStart, playPaste, playError } = useSounds();

  const {
    dictionary, dictionaryRef, addDictEntry, updateDictEntry, removeDictEntry,
    snippets, snippetsRef, addSnippet, updateSnippet, removeSnippet,
    customVocabulary, contextBiasEnabled,
    addVocabWord, removeVocabWord, addVocabPreset, clearVocab, setContextBiasEnabled,
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
    isDualChannelRecording, dualLevels,
    handleStartRecording, handlePauseRecording, handleResumeRecording, handleStopRecording, handleCancelRecording, handleTranscriptionChunk,
  } = useRecording({
    activeEngineRef: activeEngineForwarded,
    models, graniteModels, qwen3Models, currentModel, currentGraniteModel, currentQwen3Model,
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
    handleModelChange, handleSwitchToWhisper, handleSwitchToGranite, handleSwitchToQwen3,
    handleToggleAsrBackend,
  } = useEngineSwitch({
    models, graniteModels, qwen3Models,
    currentModel, currentGraniteModel, currentQwen3Model,
    setCurrentModel, setCurrentGraniteModel, setCurrentQwen3Model,
    setBackendInfo, storeRef, setHeaderStatus, setTrayState, asrBackend,
    setAsrBackend,
    isRecordingRef,
    downloadProgressRef,
    setSessionPhase,
    setSessionNotice,
  });

  const [activeMeeting, setActiveMeeting] = useState<MeetingInfo | null>(null);

  // Sync active meeting state with system tray & menu bar
  // While a stopped recording is processing, useRecording owns the tray (it shows
  // what is being processed); this sync resumes when processing ends.
  useEffect(() => {
    currentActiveMeeting = activeMeeting;
    if (isProcessingTranscript) return;
    if (!isRecording) {
      void setTrayState("ready", activeMeeting);
    } else {
      void setTrayState(isPaused ? "paused" : "recording", activeMeeting);
    }
  }, [activeMeeting, isRecording, isPaused, isProcessingTranscript]);

  // Errors reach the tray from the notice the app shows for them.
  const lastTrayNoticeRef = useRef<unknown>(null);
  useEffect(() => {
    const notice = sessionState.notice;
    if (!notice || notice === lastTrayNoticeRef.current || notice.level !== "error") return;
    lastTrayNoticeRef.current = notice;
    if (notice.code === "mic_permission_denied") {
      void setTrayState("mic_blocked");
    } else {
      void setTrayState("error", undefined, notice.title || notice.message);
    }
  }, [sessionState.notice]);

  // Model downloads: show progress in the tray (updated per whole percent).
  const trayDownloadRef = useRef<string | null>(null);
  useEffect(() => {
    const active = Object.entries(downloadProgress).filter(([, p]) => p.total > 0 && p.bytes < p.total && !p.error);
    if (active.length === 0) {
      if (trayDownloadRef.current !== null) {
        trayDownloadRef.current = null;
        if (!isRecording && !isProcessingTranscript) void setTrayState("ready", activeMeeting);
      }
      return;
    }
    const [id, p] = active[0];
    const name = settingsModels.find((m) => m.id === id)?.name ?? id;
    const label = `${name} · ${Math.floor((p.bytes / p.total) * 100)}%` + (active.length > 1 ? ` (+${active.length - 1} more)` : "");
    if (label === trayDownloadRef.current) return;
    trayDownloadRef.current = label;
    if (!isRecording && !isProcessingTranscript) void setTrayState("downloading", activeMeeting, label);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [downloadProgress]);

  // File transcription shows in the tray too (recording is blocked meanwhile).
  const fileTrayActiveRef = useRef(false);
  useEffect(() => {
    if (isFileTranscribing) {
      fileTrayActiveRef.current = true;
      void setTrayState("processing_file");
    } else if (fileTrayActiveRef.current) {
      fileTrayActiveRef.current = false;
      void setTrayState(isRecording ? "recording" : "ready", activeMeeting);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isFileTranscribing]);

  // Direct meeting detector event subscriptions and periodic reconciliation
  useEffect(() => {
    let isSubscribed = true;

    // 1. Initial scan on mount
    invoke<MeetingInfo[]>("scan_active_meetings")
      .then((meetings) => {
        if (!isSubscribed) return;
        const live = meetings.find((m) => m.should_record || m.confidence >= 70);
        setActiveMeeting(live || null);
      })
      .catch(() => {});

    // 2. Real-time events from Rust meeting detector
    const unlistenDetectedPromise = listen<MeetingInfo>("meeting-detected", (event) => {
      const m = event.payload;
      if (m.should_record || m.confidence >= 70) {
        setActiveMeeting(m);
      }
    });

    const unlistenChangedPromise = listen<MeetingInfo>("meeting-changed", (event) => {
      const m = event.payload;
      setActiveMeeting((curr) => (curr?.pid === m.pid ? m : curr));
    });

    const unlistenEndedPromise = listen<MeetingInfo>("meeting-ended", (event) => {
      const m = event.payload;
      setActiveMeeting((curr) => {
        if (!curr || curr.pid === m.pid || m.pid === 99999) {
          return null;
        }
        return curr;
      });
    });

    // 3. Periodic reconciliation every 3 seconds to guarantee no stuck meeting state
    const pollInterval = setInterval(() => {
      if (!isSubscribed) return;
      invoke<{ active_meetings: MeetingInfo[] }>("get_meeting_detection_status")
        .then((status) => {
          if (!isSubscribed) return;
          const live = status.active_meetings?.find((m) => m.should_record || m.confidence >= 70);
          setActiveMeeting((curr) => {
            if (!live && curr !== null) {
              return null;
            } else if (live && (!curr || curr.pid !== live.pid || curr.title !== live.title)) {
              return live;
            }
            return curr;
          });
        })
        .catch(() => {});
    }, 3000);

    return () => {
      isSubscribed = false;
      clearInterval(pollInterval);
      unlistenDetectedPromise.then((u) => u());
      unlistenChangedPromise.then((u) => u());
      unlistenEndedPromise.then((u) => u());
    };
  }, []);

  // Handle tray menu click "Record [Platform] Call"
  // Subscribed once: re-subscribing on every render (the handler changes each
  // render and unlisten is async) left several listeners live, and one click
  // started several recordings.
  useEffect(() => {
    const unlistenPromise = listen("start-meeting-recording", () => {
      void handleStartRecordingRef.current(false, "dual_channel");
    });
    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

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
    const isActiveModel = id === currentModel || id === currentGraniteModel || id === currentQwen3Model;
    if (isFileTranscribing && isActiveModel) {
      throw new Error("Cannot delete the active model while a file is being transcribed.");
    }
    try {
      const result = await invoke<CommandResult<string>>("delete_model", { modelId: id });
      if (!result.ok) {
        throw new Error(result.error?.message ?? "Failed to delete model");
      }
      setSettingsModels(prev => prev.map(m => m.id === id ? { ...m, downloaded: false, verified: false } : m));
      if (currentModel === id || currentGraniteModel === id || currentQwen3Model === id) {
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
      if (currentGraniteModel === id) setCurrentGraniteModel(null);
      if (currentQwen3Model === id) setCurrentQwen3Model(null);
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
  const asrModelCountsRef = useRef({
    whisper: 0,
    granite: 0,
    qwen3: 0,
  });
  asrModelCountsRef.current = {
    whisper: models.length,
    granite: graniteModels.length,
    qwen3: qwen3Models.length,
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

  // Tray "Load Model" runs the same load as the Load button.
  const loadActiveEngineRef = useRef<() => void>(() => {});
  useEffect(() => {
    const unlisten = listen("tray-load-model", () => loadActiveEngineRef.current());
    return () => { unlisten.then(fn => fn()); };
  }, []);

  // Window restored from the tray
  useEffect(() => {
    const unlisten = listen("window-restored", () => {
      appHiddenRef.current = false;
      if (pendingNoModelCtaPulseRef.current) {
        pendingNoModelCtaPulseRef.current = false;
        setFileMode(false);
        startNoModelCtaAttention();
      }
    });
    return () => { unlisten.then(fn => fn()); };
  }, [startNoModelCtaAttention]);

  // ── Hooks extracted from App.tsx ──
  useInitialLoad({
    setModels, setCurrentModel,
    setGraniteModels, setCurrentGraniteModel,
    setQwen3Models, setCurrentQwen3Model,
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
    enableOverlayRef,
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
  });

  useModelsWatcher({ isSettingsOpen, refreshModels, downloadProgressRef, setSettingsModels });

  const {
    timeoutSeconds: autoUnloadTimeout,
    remainingSeconds: autoUnloadRemaining,
    isMenuOpen: isAutoUnloadMenuOpen,
    setIsMenuOpen: setIsAutoUnloadMenuOpen,
    updateTimeout: handleUpdateAutoUnloadTimeout,
  } = useAutoUnload({
    loadedEngine,
    setLoadedEngine,
    setHeaderStatus,
  });
  const closeAutoUnloadMenu = useCallback(() => setIsAutoUnloadMenuOpen(false), [setIsAutoUnloadMenuOpen]);
  useDismissOnOutside(autoUnloadMenuRef, closeAutoUnloadMenu, {
    enabled: isAutoUnloadMenuOpen,
    ignoreSelector: "#auto-unload-timer-btn",
  });

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
    else if (activeEngine === "granite") void handleSwitchToGranite();
    else void handleSwitchToQwen3();
  };
  loadActiveEngineRef.current = handleLoadActiveEngine;

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
    currentGraniteModel,
    currentQwen3Model,
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
          else if (engineForModel === 'granite') await handleSwitchToGranite(id);
          else await handleSwitchToQwen3(id);
          return;
        }
        if (loadedEngine) return;
        if (engineForModel === 'whisper') handleModelChange(id);
        else if (engineForModel === 'granite') handleSwitchToGranite(id);
        else handleSwitchToQwen3(id);
      }
    };
  }, [handleModelChange, handleSwitchToGranite, handleSwitchToQwen3, loadedEngine, refreshModels]);






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
  const noGraniteModel = graniteModels.length === 0;
  const noQwen3Model = qwen3Models.length === 0;
  const noAnyAsrModel = noWhisperModel && noGraniteModel && noQwen3Model;
  const activeEngineHasNoModel =
    (activeEngine === "whisper" && noWhisperModel) ||
    (activeEngine === "granite" && noGraniteModel) ||
    (activeEngine === "qwen3" && noQwen3Model);
  const noModel = activeEngineHasNoModel;
  const noLlm = llmStatus === "Not Downloaded";
  const downloadProgressKeys = useMemo(() => Object.keys(downloadProgress), [downloadProgress]);
  const isWhisperDownloading = useMemo(
    () => downloadProgressKeys.some((key) => key.startsWith("whisper-")),
    [downloadProgressKeys],
  );
  const isGraniteDownloading = useMemo(
    () => downloadProgressKeys.some((key) => key.startsWith("granite")),
    [downloadProgressKeys],
  );
  const isQwen3Downloading = useMemo(
    () => downloadProgressKeys.some((key) => key.startsWith("qwen3")),
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
  const recordBtnAriaLabel =
    noModel ? "Download a model first in Settings" :
      isFileTranscribing ? "Cannot record while a file is being transcribed" :
        isRecording ? "Stop recording" :
          recordBtnBusy ? "Please wait…" :
            "Start recording (REC)";
  const recordBtnDisabled = isFileTranscribing || (isLoading && !isRecording) || isProcessingTranscript;

  const onRecordClick = () => {
    if (noModel) { setIsSettingsOpen(true); return; }
    if (isRecording) handleStopRecording();
    else if (activeMeeting) handleStartRecording(false, "dual_channel");
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
    if (activeEngine === "granite") {
      const label = "Granite";
      const color = "var(--granite-color)";
      if (isLoading && loadingTargetEngine === "granite") return { label, color, model: "Loading…" };
      if (isGraniteDownloading) return { label, color, model: "Downloading…" };
      if (graniteModels.length === 0) return { label, color, model: "No model" };
      const m = graniteModels.find(x => x.id === currentGraniteModel) ?? graniteModels[0];
      return { label, color, model: beautifyModelName(m.display_name) };
    }
    const label = "Qwen3-ASR";
    const color = "#a78bfa";
    if (isLoading && loadingTargetEngine === "qwen3") return { label, color, model: "Loading…" };
    if (isQwen3Downloading) return { label, color, model: "Downloading…" };
    if (qwen3Models.length === 0) return { label, color, model: "No model" };
    const m = qwen3Models.find(x => x.id === currentQwen3Model) ?? qwen3Models[0];
    return { label, color, model: m.display_name };
  }, [activeEngine, isLoading, loadingTargetEngine, isWhisperDownloading, isGraniteDownloading, isQwen3Downloading,
      models, currentModel, graniteModels, currentGraniteModel, qwen3Models, currentQwen3Model]);

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
      if (activeEngine === "granite") {
        const m = graniteModels.find(x => x.id === selectedOrLoadedModelId) ?? graniteModels.find(x => x.id === currentGraniteModel) ?? graniteModels[0];
        return m ? beautifyModelName(m.display_name) : engineChipMeta.model;
      }
      const m = qwen3Models.find(x => x.id === selectedOrLoadedModelId) ?? qwen3Models.find(x => x.id === currentQwen3Model) ?? qwen3Models[0];
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
    graniteModels,
    currentGraniteModel,
    qwen3Models,
    currentQwen3Model,
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

  const openModelSettingsForEngine = useCallback((engine: ASREngine) => {
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
    const parts = msg.split(/(Qwen3-ASR|Whisper|Granite|OpenAI|NVIDIA)/g);
    return parts.map((part, i) => {
      if (part === "Whisper" || part === "OpenAI") return <span key={i} style={{ color: 'var(--whisper-color)' }}>{part}</span>;
      if (part === "Granite" || part === "NVIDIA") return <span key={i} style={{ color: 'var(--granite-color)' }}>{part}</span>;
      if (part === "Qwen3-ASR") return <span key={i} style={{ color: '#a78bfa' }}>{part}</span>;
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

  useEffect(() => {
    (window as any).__TAURSCRIBE_TEST__ = {
      getActiveMeeting: () => activeMeeting,
      isRecording: () => isRecording,
      isDualChannel: () => isDualChannelRecording,
      getNavMode: () => navMode,
      setNavMode: (mode: NavMode) => setNavMode(mode),
      startDualRecording: () => handleStartRecording(false, "dual_channel"),
      stopRecording: () => handleStopRecording(),
      openSettings: () => setIsSettingsOpen(true),
    };
  }, [activeMeeting, isRecording, isDualChannelRecording, navMode, handleStartRecording, handleStopRecording]);

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
        meeting={activeMeeting}
        isRecording={isRecording}
        isDualChannelRecording={isDualChannelRecording}
        onStartDualRecording={() => handleStartRecording(false, "dual_channel")}
      />
      <div className={`app-body ${isRecording ? "app-body--recording" : ""} theme-${activeEngine}`}>
        <main className={`container${containerBooting ? " container--booting" : ""}`}>
          <div>
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
                        id="enable-input-monitoring-btn"
                        data-testid="enable-input-monitoring-btn"
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
                      id="enable-accessibility-btn"
                      data-testid="enable-accessibility-btn"
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
                    id="restart-app-btn"
                    data-testid="restart-app-btn"
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
                  id="dismiss-accessibility-banner-btn"
                  data-testid="dismiss-accessibility-banner-btn"
                  className="accessibility-banner-dismiss"
                  onClick={() => {
                    setAccessibilityMissing(false);
                    setInputMonitoringMissing(false);
                  }}
                  aria-label="Dismiss accessibility banner"
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
                      id="grant-mic-permission-btn"
                      data-testid="grant-mic-permission-btn"
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
                      id="open-mic-settings-btn"
                      data-testid="open-mic-settings-btn"
                      className="mic-banner-action"
                      onClick={async () => {
                        await invoke('open_microphone_settings').catch(() => {});
                      }}
                    >
                      Open Settings
                    </button>
                  </span>
                )}
                <button
                  type="button"
                  id="dismiss-mic-banner-btn"
                  data-testid="dismiss-mic-banner-btn"
                  className="mic-banner-dismiss"
                  onClick={() => setMicPermission(null)}
                  aria-label="Dismiss microphone banner"
                >
                  ✕
                </button>
              </div>
            )}

            {showSilenceWarning && isRecording && !isPaused && (
              <div
                id="silence-warning-banner"
                data-testid="silence-warning-banner"
                className="silence-banner"
                role="alert"
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={{ flexShrink: 0 }}>
                  <line x1="1" y1="1" x2="23" y2="23" />
                  <path d="M9 9v3a3 3 0 0 0 5.12 2.12M15 9.34V4a3 3 0 0 0-5.94-.6" />
                  <path d="M17 16.95A7 7 0 0 1 5 12v-2m14 0v2a7 7 0 0 1-.11 1.23" />
                  <line x1="12" y1="19" x2="12" y2="23" />
                  <line x1="8" y1="23" x2="16" y2="23" />
                </svg>
                <span>No audio detected — is your mic muted or the wrong device selected?</span>
                <button
                  type="button"
                  id="dismiss-silence-banner-btn"
                  data-testid="dismiss-silence-banner-btn"
                  className="silence-banner-dismiss"
                  onClick={() => setShowSilenceWarning(false)}
                  aria-label="Dismiss silence warning"
                >
                  ✕
                </button>
              </div>
            )}

            {/* Meeting Banner V8 */}
            <MeetingBanner
              key={activeMeeting ? `${activeMeeting.pid}:${activeMeeting.url}:${activeMeeting.title}` : "none"}
              meeting={activeMeeting}
              isRecording={isRecording}
              onStartDualRecording={() => handleStartRecording(false, "dual_channel")}
              suppressBanner={navMode === "meetings"}
            />
          </div>

          {/* Mode toggle (Mic / Meetings / Files) — top bar with Settings shortcut */}
          <div className="mode-toggle-row">
            <div
              id="mode-toggle-group"
              data-testid="mode-toggle-group"
              ref={modeToggleRef}
              className="mode-toggle"
              role="radiogroup"
              aria-label="Input mode"
            >
              <span
                className="mode-toggle-indicator"
                aria-hidden="true"
                style={{
                  transform: `translateX(${modeIndicator.left}px)`,
                  width: modeIndicator.width,
                  opacity: modeIndicator.ready ? 1 : 0,
                }}
              />
              <button
                type="button"
                id="mode-toggle-mic"
                data-testid="mode-toggle-mic"
                role="radio"
                aria-checked={navMode === "mic"}
                aria-label="Microphone dictation mode"
                className={`mode-toggle-btn${navMode === "mic" ? " mode-toggle-btn--active" : ""}`}
                onClick={() => setNavMode("mic")}
                disabled={navMode === "files" && isFileTranscribing}
                title={navMode === "files" && isFileTranscribing ? "Wait for file transcription to finish" : undefined}
              >
                <IconMic size={13} /> Mic
              </button>
              <button
                type="button"
                id="mode-toggle-meetings"
                data-testid="mode-toggle-meetings"
                role="radio"
                aria-checked={navMode === "meetings"}
                aria-label="Meeting detection and dual-channel recording mode"
                className={`mode-toggle-btn${navMode === "meetings" ? " mode-toggle-btn--active" : ""}`}
                onClick={() => setNavMode("meetings")}
                disabled={navMode === "files" && isFileTranscribing}
              >
                <IconVideo size={13} /> Meetings
                {activeMeeting && (
                  <span className="mode-toggle-meeting-dot" title="Active Meeting Detected" />
                )}
              </button>
              <button
                type="button"
                id="mode-toggle-files"
                data-testid="mode-toggle-files"
                role="radio"
                aria-checked={navMode === "files"}
                aria-label="File transcription mode"
                className={`mode-toggle-btn${navMode === "files" ? " mode-toggle-btn--active" : ""}`}
                onClick={() => setNavMode("files")}
              >
                <IconFileText size={13} /> Files
              </button>
            </div>

            <button
              type="button"
              id="settings-open-btn"
              data-testid="settings-open-btn"
              className="top-settings-btn"
              onClick={() => setIsSettingsOpen(true)}
              title="Settings"
              aria-label="Settings"
            >
              <IconSettings size={14} />
            </button>
          </div>

          <div className={`app-status-rail${headerStatusMessage ? " app-status-rail--visible" : ""}`}>
            {headerStatusMessage !== null && (
              <span
                className={`header-status-message ${headerStatusIsProcessing ? "header-status-message--processing" : ""}`}
                key={headerStatusMessage}
              >
                {colorizedStatus}
              </span>
            )}
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
            <div className="nav-view-enter" style={navMode === "files" ? undefined : { display: 'none' }}>
              <FileTranscriptionPanel
                activeEngine={activeEngine}
                currentModel={currentModel}
                currentGraniteModel={currentGraniteModel}
                currentQwen3Model={currentQwen3Model}
                isModelLoading={isLoading}
                onFileProcessingChange={setIsFileTranscribing}
              />
            </div>
            {navMode === "meetings" && (
              <MeetingsPanel
                activeMeeting={activeMeeting}
                isRecording={isRecording}
                isDualChannelRecording={isDualChannelRecording}
                dualLevels={dualLevels}
                onStartDualRecording={() => handleStartRecording(false, "dual_channel")}
                onStopRecording={handleStopRecording}
                onOpenSettings={() => handleOpenSettingsTab("meetings")}
                onOpenModelSettings={() => handleOpenSettingsTab("models")}
              />
            )}
            {navMode === "mic" && (activeEngineHasNoModel ? (
              <div className="empty-state">
                <div className="empty-state-icon" aria-hidden="true">
                  {noAnyAsrModel ? <IconDownload size={32} /> : activeEngine === "whisper" ? <IconMic size={32} /> : <IconBolt size={32} style={{ color: '#facc15' }} />}
                </div>
                <h2 className="empty-state-title">
                  {noAnyAsrModel
                    ? "No speech model downloaded"
                    : activeEngine === "whisper"
                      ? "No Whisper model downloaded"
                      : activeEngine === "granite"
                        ? "Granite not downloaded"
                        : "Qwen3-ASR not downloaded"}
                </h2>
                <p className="empty-state-body">
                  {noAnyAsrModel ? (
                    <>Download a <strong>Whisper</strong>, <strong>Granite</strong>, or <strong>Qwen3-ASR</strong> model to start transcribing. Whisper Base is a good starting point.</>
                  ) : activeEngine === "whisper" ? (
                    <>You're on the <strong>Whisper</strong> engine but haven't downloaded a model yet. Try <strong>Whisper Base</strong> — it's small and accurate. Or switch to Granite if you already have it.</>
                  ) : activeEngine === "granite" ? (
                    <>You're on the <strong>Granite</strong> engine but Granite Speech 5 isn't downloaded yet. Switch to Whisper if you already have a model, or download Granite from Settings.</>
                  ) : (
                    <>You're on the <strong>Qwen3-ASR</strong> engine but the model isn't downloaded yet. Download Qwen3-ASR from Settings or switch to another installed engine.</>
                  )}
                </p>
                {!noAnyAsrModel && (
                  <p className="empty-state-hint">
                    {activeEngine === "whisper" && !noGraniteModel
                      ? <><IconLightbulb size={14} /> You already have a Granite model — click the Granite card above to switch.</>
                      : activeEngine === "granite" && !noWhisperModel
                        ? <><IconLightbulb size={14} /> You already have a Whisper model — click the Whisper card above to switch.</>
                        : null}
                  </p>
                )}
                <button
                  type="button"
                  id="empty-state-download-cta"
                  data-testid="empty-state-download-cta"
                  className={`empty-state-cta${noModelCtaAttention ? " empty-state-cta--attention" : ""}`}
                  onClick={() => {
                    setNoModelCtaAttention(false);
                    openModelSettingsForEngine(activeEngine);
                  }}
                  aria-label="Open Settings to download models"
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

          {navMode !== "meetings" && (
            <div className={`bottom-bar${navMode === "files" ? " bottom-bar--files" : ""}`}>
              <div className="bottom-left">
              {/* Microphone selector — lists all available input devices;
                  selecting one persists the choice to settings.json.
                  Files mode keeps only the engine/model controls. */}
              {navMode === "mic" && (
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
                    id="mic-selector-dropdown"
                    data-testid="mic-selector-dropdown"
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

                {isDualChannelRecording && isRecording && (
                  <div
                    id="dual-channel-active-badge"
                    data-testid="dual-channel-active-badge"
                    className="dual-channel-active-badge"
                    title="Recording Dual-Channel: Mic (CH1) + System Loopback (CH2)"
                    role="status"
                    aria-label="Dual-Channel Audio Active"
                  >
                    <span className="dual-channel-dot" />
                    <span className="dual-channel-text">DUAL-CH</span>
                    <span className="dual-channel-levels">
                      <span className="dual-ch-tag">M</span>
                      <span className="dual-ch-val">{levelToPercent(dualLevels?.mic ?? 0)}%</span>
                      <span className="dual-ch-sep">·</span>
                      <span className="dual-ch-tag">S</span>
                      <span className="dual-ch-val">{levelToPercent(dualLevels?.system ?? 0)}%</span>
                    </span>
                  </div>
                )}
              </div>
              )}

              <button
                type="button"
                id="engine-chip-button"
                data-testid="engine-chip-button"
                className="engine-chip"
                onClick={() => setIsEnginePickerOpen(o => !o)}
                aria-label={`Switch engine or model; status ${recordReadinessMeta.phase}`}
                aria-expanded={isEnginePickerOpen}
                aria-haspopup="dialog"
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
              {!isLoading && !isRecording && !isProcessingTranscript && !isFileTranscribing && (
                loadedEngine === activeEngine ? (
                  <div className="load-eject-group">
                    <button
                      type="button"
                      id="model-eject-btn"
                      data-testid="model-eject-btn"
                      className="load-eject-btn"
                      onClick={handleEjectModel}
                      title="Unload model (free VRAM immediately)"
                      aria-label="Unload model"
                    >
                      <IconEject size={14} />
                    </button>

                    <button
                      type="button"
                      id="auto-unload-timer-btn"
                      data-testid="auto-unload-timer-btn"
                      className={`auto-unload-timer-btn${isAutoUnloadMenuOpen ? " auto-unload-timer-btn--active" : ""}`}
                      onClick={() => setIsAutoUnloadMenuOpen(!isAutoUnloadMenuOpen)}
                      title={
                        autoUnloadRemaining !== null
                          ? `Auto-unloads in ${formatRemaining(autoUnloadRemaining)} of inactivity (click to configure)`
                          : `Model auto-unload: ${formatTimeoutLabel(autoUnloadTimeout)} (click to configure)`
                      }
                      aria-label="Configure model memory auto-unload"
                    >
                      <span className="auto-unload-pill-icon">⏱️</span>
                      <span className="auto-unload-pill-text">
                        {autoUnloadRemaining !== null
                          ? formatRemaining(autoUnloadRemaining)
                          : formatTimeoutLabel(autoUnloadTimeout)}
                      </span>
                      <span className="auto-unload-pill-caret" aria-hidden="true">▾</span>
                    </button>

                    {isAutoUnloadMenuOpen && (
                      <>
                        <div
                          className="auto-unload-backdrop"
                          onClick={() => setIsAutoUnloadMenuOpen(false)}
                        />
                        <div
                          ref={autoUnloadMenuRef}
                          id="auto-unload-menu"
                          data-testid="auto-unload-menu"
                          className="auto-unload-menu"
                          role="menu"
                          aria-label="Model memory retention options"
                        >
                          <div className="auto-unload-menu-header">
                            <span className="auto-unload-menu-title">Model Memory & Auto-Unload</span>
                            {autoUnloadRemaining !== null && (
                              <span className="auto-unload-countdown">
                                Freeing VRAM in {formatRemaining(autoUnloadRemaining)}
                              </span>
                            )}
                          </div>

                          <button
                            type="button"
                            id="auto-unload-eject-btn"
                            data-testid="auto-unload-eject-btn"
                            className="auto-unload-eject-action"
                            onClick={() => {
                              setIsAutoUnloadMenuOpen(false);
                              void handleEjectModel();
                            }}
                          >
                            <IconEject size={13} />
                            <span>Unload Now (Free VRAM)</span>
                          </button>

                          <div className="auto-unload-menu-divider" />
                          <div className="auto-unload-menu-section-label">Keep model loaded in memory:</div>

                          {AUTO_UNLOAD_OPTIONS.map((opt) => (
                            <button
                              key={opt.value}
                              type="button"
                              id={`auto-unload-option-${opt.value}`}
                              data-testid={`auto-unload-option-${opt.value}`}
                              className={`auto-unload-menu-item${autoUnloadTimeout === opt.value ? " auto-unload-menu-item--selected" : ""}`}
                              onClick={() => {
                                void handleUpdateAutoUnloadTimeout(opt.value);
                                setIsAutoUnloadMenuOpen(false);
                              }}
                            >
                              <div className="auto-unload-item-info">
                                <span className="auto-unload-item-label">{opt.label}</span>
                                <span className="auto-unload-item-desc">{opt.description}</span>
                              </div>
                              {autoUnloadTimeout === opt.value && (
                                <span className="auto-unload-item-check" aria-hidden="true">✓</span>
                              )}
                            </button>
                          ))}
                        </div>
                      </>
                    )}
                  </div>
                ) : (
                  (activeEngine === "whisper" ? !noWhisperModel :
                   activeEngine === "granite" ? !noGraniteModel :
                   !noQwen3Model) && (
                    <button
                      type="button"
                      id="model-load-btn"
                      data-testid="model-load-btn"
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
                  graniteModels={graniteModels}
                  currentGraniteModel={currentGraniteModel}
                    qwen3Models={qwen3Models}
                  currentQwen3Model={currentQwen3Model}
                  downloadProgress={downloadProgress}
                  isWhisperDownloading={isWhisperDownloading}
                  isGraniteDownloading={isGraniteDownloading}
                  isQwen3Downloading={isQwen3Downloading}
                  disabled={isRecording || isFileTranscribing}
                  onSelectWhisperModel={(id) => handleModelChange(id)}
                  onSelectGraniteModel={(id) => { void handleSwitchToGranite(id); }}
                  onSelectQwen3Model={(id) => { void handleSwitchToQwen3(id); }}
                  onUnload={handleEjectModel}
                  onOpenDownloads={openModelSettingsForEngine}
                  onClose={() => setIsEnginePickerOpen(false)}
                  autoUnloadTimeout={autoUnloadTimeout}
                  onUpdateAutoUnloadTimeout={handleUpdateAutoUnloadTimeout}
                />
              )}
            </div>

            {navMode === "mic" && (
            <div className="record-btn-wrap">
              <button
                type="button"
                id="record-button"
                data-testid="record-button"
                aria-pressed={isRecording}
                aria-label={recordBtnAriaLabel}
                className={recordBtnClass}
                disabled={!noModel && recordBtnDisabled}
                onClick={onRecordClick}
                title={noModel ? "Download a model first in Settings" : isFileTranscribing ? "Cannot record while a file is being transcribed" : recordBtnBusy ? "Please wait…" : isRecording ? "Stop recording" : "Start recording"}
              >
                {recordBtnLabel}
              </button>
            </div>
            )}

          </div>
          )}

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
            customVocabulary={customVocabulary}
            contextBiasEnabled={contextBiasEnabled}
            addVocabWord={addVocabWord}
            removeVocabWord={removeVocabWord}
            addVocabPreset={addVocabPreset}
            clearVocab={clearVocab}
            setContextBiasEnabled={setContextBiasEnabled}
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
