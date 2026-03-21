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
import { useRecording } from "./hooks/useRecording";
import { useSounds } from "./hooks/useSounds";
import { usePersonalization } from "./hooks/usePersonalization";
import { TranscriptFeed } from "./components/TranscriptFeed";
import { FileTranscriptionPanel } from "./components/FileTranscriptionPanel";
import { QuickSettings } from "./components/QuickSettings";
import { useDownloads } from "./hooks/useDownloads";
import { MODELS } from "./components/settings/types";
import type { DownloadableModel } from "./components/settings/types";
import type { OnboardingUseCase } from "./modelRecommendations";
import "./components/TitleBar.css";
import "./App.css";
import { IconChat, IconFileText, IconSparkle, IconCode, IconTie, IconBolt, IconCpu, IconDownload, IconMic, IconLightbulb, IconSettings, InfoTooltip } from "./components/Icons";

const ANIMATED_LOGOS = [
  "animated_logo_assemble.svg",
  "animated_logo_blueprint.svg",
  "animated_logo_bottom_spin.svg",
  "animated_logo_bounce.svg",
  "animated_logo_flip.svg",
  "animated_logo_grow.svg",
  "animated_logo_scan_reveal.svg",
  "animated_logo_shockwave.svg",
  "animated_logo_slice.svg",
  "animated_logo_stomp.svg",
  "animated_logo_write.svg",
  "animated_logo_pulse_reveal.svg",
  "animated_logo_swing.svg",
  "animated_logo_zigzag.svg",
  "animated_logo_spiral.svg",
  "animated_logo_breathe.svg",
  "animated_logo_thin_air.svg",
  "animated_logo_glitch.svg",
  "animated_logo_orbit.svg",
  "animated_logo_rubberband.svg",
  "animated_logo_focus.svg",
  "animated_logo_crt.svg",
  "animated_logo_liquid.svg",
  "animated_logo_hologram.svg",
  "animated_logo_wiper.svg",
  "animated_logo_heartbeat.svg",
  "animated_logo_laser_trace.svg",
  "animated_logo_split_door.svg",
  "animated_logo_ripple.svg",
  "animated_logo_flare.svg",
  "animated_logo_handwrite.svg",
  "animated_logo_quantum_flip.svg",
  "animated_logo_debris.svg"
];

// Ticker phrases defined outside the component so the array is never recreated on render
type TickerHighlight = "accent" | "whisper" | "parakeet" | "granite";
const TICKER_PHRASES: { parts: { text: string; highlight?: TickerHighlight }[] }[] = [
  { parts: [{ text: "100% " }, { text: "local", highlight: "accent" }, { text: " · nothing leaves your machine" }] },
  { parts: [{ text: "OpenAI " }, { text: "Whisper", highlight: "whisper" }, { text: " & NVIDIA " }, { text: "Parakeet", highlight: "parakeet" }, { text: " · GPU-accelerated" }] },
  { parts: [{ text: "Hit " }, { text: "REC", highlight: "accent" }, { text: " · speech to text in real time" }] },
  { parts: [{ text: "No cloud", highlight: "accent" }, { text: " · no API keys · no subscriptions" }] },
  { parts: [{ text: "Switch between " }, { text: "Whisper", highlight: "whisper" }, { text: " and " }, { text: "Parakeet", highlight: "parakeet" }, { text: " anytime" }] },
  { parts: [{ text: "IBM " }, { text: "Granite Speech", highlight: "granite" }, { text: " · 1B · ONNX · runs anywhere" }] },
  { parts: [{ text: "Ctrl+Win", highlight: "accent" }, { text: " from anywhere to record" }] },
  { parts: [{ text: "Grammar correction · optional " }, { text: "LLM", highlight: "accent" }, { text: "" }] },
  { parts: [{ text: "Offline-first", highlight: "accent" }, { text: " · your data stays yours" }] },
  { parts: [{ text: "Pick your engine · " }, { text: "Whisper", highlight: "whisper" }, { text: " · " }, { text: "Parakeet", highlight: "parakeet" }, { text: " · " }, { text: "Granite Speech", highlight: "granite" }] },
  { parts: [{ text: "Studio-grade", highlight: "accent" }, { text: " · runs on your hardware" }] },
  { parts: [{ text: "Real-time transcription with " }, { text: "Whisper", highlight: "whisper" }, { text: " or " }, { text: "Parakeet", highlight: "parakeet" }] },
  { parts: [{ text: "Your audio never leaves this device" }] },
  { parts: [{ text: "CUDA · CPU · Metal · flexible backends" }] },
  { parts: [{ text: "Download models once · use forever" }] },
  { parts: [{ text: "Built for privacy · built for speed" }] },
  { parts: [{ text: "Three engines · " }, { text: "Whisper", highlight: "whisper" }, { text: " · " }, { text: "Parakeet", highlight: "parakeet" }, { text: " · " }, { text: "Granite Speech", highlight: "granite" }] },
  { parts: [{ text: "Press REC and speak · that's it" }] },
  { parts: [{ text: "No account", highlight: "accent" }, { text: " · no sign-up · no tracking" }] },
  { parts: [{ text: "Low latency · high accuracy" }] },
  { parts: [{ text: "Use " }, { text: "Whisper", highlight: "whisper" }, { text: " for batch · " }, { text: "Parakeet", highlight: "parakeet" }, { text: " for streaming" }] },
  { parts: [{ text: "Desktop-first · always ready" }] },
  { parts: [{ text: "Your words · your machine · your rules" }] },
  { parts: [{ text: "Multilingual " }, { text: "Whisper", highlight: "whisper" }, { text: " · real-time " }, { text: "Parakeet", highlight: "parakeet" }] },
  { parts: [{ text: "Transcribe meetings · notes · ideas" }] },
  { parts: [{ text: "One click to record", highlight: "accent" }, { text: " · one click to copy" }] },
  { parts: [{ text: "GPU-accelerated when you have it" }] },
  { parts: [{ text: "Open source models · open future" }] },
  { parts: [{ text: "IBM " }, { text: "Granite Speech", highlight: "granite" }, { text: " · no GPU required" }] },
  { parts: [{ text: "Privacy by design", highlight: "accent" }, { text: " · not as an afterthought" }] },
  { parts: [{ text: "Capture every word · edit later" }] },
  { parts: [{ text: "No internet? No problem." }] },
  { parts: [{ text: "Tiny to large · pick your " }, { text: "Whisper", highlight: "whisper" }, { text: " size" }] },
  { parts: [{ text: "Streaming with " }, { text: "Parakeet", highlight: "parakeet" }, { text: " · see text as you speak" }] },
  { parts: [{ text: "Hotkey ready", highlight: "accent" }, { text: " · Ctrl+Win from any app" }] },
  { parts: [{ text: "Local AI · no data in the cloud" }] },
  { parts: [{ text: "Built for creators · built for you" }] },
  { parts: [{ text: "Switch engines mid-workflow" }] },
  { parts: [{ text: "Grammar correction · optional" }] },
  { parts: [{ text: "Whisper", highlight: "whisper" }, { text: " for accuracy · " }, { text: "Parakeet", highlight: "parakeet" }, { text: " for speed · " }, { text: "Granite Speech", highlight: "granite" }, { text: " for reliability" }] },
  { parts: [{ text: "Your microphone · your transcript" }] },
  { parts: [{ text: "Download once · run anywhere" }] },
  { parts: [{ text: "No subscriptions", highlight: "accent" }, { text: " · pay with your hardware" }] },
  { parts: [{ text: "Transcription that respects you" }] },
  { parts: [{ text: "Fast " }, { text: "Whisper", highlight: "whisper" }, { text: " · faster " }, { text: "Parakeet", highlight: "parakeet" }] },
  { parts: [{ text: "Record · transcribe · copy · done" }] },
  { parts: [{ text: "OpenAI · NVIDIA · IBM · three giants · one app" }] },
  { parts: [{ text: "One app · three engines · zero compromise" }] },
];

const TONE_STYLES: { value: string; label: string; icon: React.ReactNode; accent: string; desc: string }[] = [
  { value: 'Casual', label: 'Casual', icon: <IconChat size={18} />, accent: '#6895d2', desc: 'Relaxed, conversational tone. Great for notes, emails, and quick messages.' },
  { value: 'Verbatim', label: 'Verbatim', icon: <IconFileText size={18} />, accent: '#94a3b8', desc: 'Minimal changes. Keeps your original speech intact with filler words preserved.' },
  { value: 'Enthusiastic', label: 'Enthusiastic', icon: <IconSparkle size={18} />, accent: '#f472b6', desc: 'Energetic and expressive. Perfect for pitches, presentations, and vlogs.' },
  { value: 'Software_Dev', label: 'Software Dev', icon: <IconCode size={18} />, accent: '#3ecfa5', desc: 'Technical language with proper code terms, casing, and dev conventions.' },
  { value: 'Professional', label: 'Professional', icon: <IconTie size={18} />, accent: '#2563eb', desc: 'Formal and polished. Ideal for reports, documentation, and client work.' },
];

const setTrayState = async (newState: "ready" | "recording" | "processing") => {
  try {
    await invoke("set_tray_state", { newState });
  } catch (e) {
    console.error("Failed to set tray state:", e);
  }
};

const formatSize = (sizeMb: number): string => {
  if (sizeMb >= 1024) return `${(sizeMb / 1024).toFixed(1)} GB`;
  return `${Math.round(sizeMb)} MB`;
};

const beautifyModelName = (rawName: string) => {
  let name = rawName
    .replace("ggml-", "")
    .replace(".bin", "")
    .replace("distil-", "Distil ")
    .replace("medium.en", "Medium")
    .replace("small.en", "Small")
    .replace("tiny.en", "Tiny")
    .replace("base.en", "Base")
    .replace("-q8_0", " (Fast)")
    .replace("-q5_1", " (Balanced)")
    .replace("nemotron", "Nemotron")
    .replace("parakeet", "")
    .replace("ctc-", "CTC ")
    .replace("tdt-", "TDT ")
    .replace("streaming", "Streaming")
    .replace("-", " ")
    .replace("_", " ")
    .trim();
  return name.replace(/\b\w/g, l => l.toUpperCase());
};

function App() {
  const pickRandomLogo = useCallback(() => {
    return ANIMATED_LOGOS[Math.floor(Math.random() * ANIMATED_LOGOS.length)];
  }, []);

  const [randomLogo, setRandomLogo] = useState(pickRandomLogo);
  const [isLogoShuttering, setIsLogoShuttering] = useState(false);

  const [isBooting, setIsBooting] = useState(true);
  // M6 fix: containerBooting controls the CSS stagger class; cleared after
  // the boot animation completes so re-mounts don't re-trigger the stagger.
  const [containerBooting, setContainerBooting] = useState(true);

  useEffect(() => {
    // Boot title scramble: 600ms
    const titleTimer = setTimeout(() => setIsBooting(false), 600);
    // Container stagger: clear after all children finish (10 × 80ms + 500ms duration)
    const staggerTimer = setTimeout(() => setContainerBooting(false), 1400);
    return () => {
      clearTimeout(titleTimer);
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

  // Re-randomize the logo animation when the window is restored from the tray
  useEffect(() => {
    const unlisten = listen("window-restored", () => {
      setRandomLogo(pickRandomLogo());
    });
    return () => { unlisten.then(fn => fn()); };
  }, [pickRandomLogo]);

  // Close the settings modal when the window is hidden to tray so the hotkey
  // works immediately when the user restores the window.
  useEffect(() => {
    const unlisten = listen("window-hidden", () => {
      setIsSettingsOpen(false);
    });
    return () => { unlisten.then(fn => fn()); };
  }, []);

  const storeRef = useRef<Store | null>(null);
  const [backendInfo, setBackendInfo] = useState("Loading...");
  const [isInitialLoading, setIsInitialLoading] = useState(true);
  const [loadingMessage, setLoadingMessage] = useState("");
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [settingsInitialTab, setSettingsInitialTab] = useState<string | undefined>(undefined);
  /** null = not yet loaded from store; true = show wizard (first run); false = show main app */
  const [showSetupWizard, setShowSetupWizard] = useState<boolean | null>(null);
  /** Incremented after each successful save_transcript_history; tells TranscriptFeed to reload. */
  const [historyRefreshKey, setHistoryRefreshKey] = useState(0);
  /** Whether the output area is in file-transcription mode vs mic-recording mode */
  const [fileMode, setFileMode] = useState(false);
  /** True while FileTranscriptionPanel has a file actively transcribing */
  const [isFileTranscribing, setIsFileTranscribing] = useState(false);

  // macOS fix: Detect the runtime platform so we can hide/adjust UI elements
  // that don't apply on macOS (e.g. GPU/CPU toggle, VRAM display).
  const [platform, setPlatform] = useState('');
  // macOS fix: Tracks whether macOS Accessibility/Input Monitoring permission
  // is missing. The Rust backend emits an "accessibility-missing" event on
  // launch if AXIsProcessTrustedWithOptions returns false — without it,
  // rdev's global hotkey listener silently receives zero key events.
  const [accessibilityMissing, setAccessibilityMissing] = useState(false);
  // macOS fix: Track microphone permission so we can show a banner when denied.
  const [micPermission, setMicPermission] = useState<'granted' | 'denied' | 'undetermined' | null>(null);
  // Active microphone name and full device list — shown as a dropdown on the
  // home view so the user can switch mics without opening Settings.
  const [activeMic, setActiveMic] = useState<string | null>(null);
  const [inputDevices, setInputDevices] = useState<string[]>([]);
  // Close-button behavior: 'tray' = hide to tray (default), 'quit' = exit process
  const [closeBehavior, setCloseBehavior] = useState<'tray' | 'quit'>('tray');
  // Overlay HUD style: 'full' = interactive card with controls, 'minimal' = compact status pill
  const [overlayStyle, setOverlayStyle] = useState<'minimal' | 'full'>('full');
  const [isAppleSilicon, setIsAppleSilicon] = useState(false);
  useEffect(() => {
    invoke<string>('get_platform').then(setPlatform).catch(() => {});
    invoke<boolean>('is_apple_silicon').then(setIsAppleSilicon).catch(() => {});
  }, []);
  const isMac = platform === 'macos';

  // Fetch the active mic and the full device list on launch (all platforms).
  useEffect(() => {
    invoke<string>('get_active_input_device').then(setActiveMic).catch(() => {});
    invoke<string[]>('list_input_devices').then(setInputDevices).catch(() => {});
  }, []);

  // Handle mic selection from the hardware bar dropdown.
  const handleMicChange = async (name: string) => {
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
    } catch (e) { console.error('Failed to set input device:', e); }
  };

  // macOS fix: Check microphone permission on launch so the user sees a
  // prompt before attempting to record.
  useEffect(() => {
    if (!isMac) return;
    invoke<string>('check_microphone_permission')
      .then((status) => setMicPermission(status as 'granted' | 'denied' | 'undetermined'))
      .catch(() => {});
  }, [isMac]);

  const [settingsModels, setSettingsModels] = useState<DownloadableModel[]>(MODELS);

  // --- Custom Hooks ---
  const { headerStatusMessage, headerStatusIsProcessing, setHeaderStatus } = useHeaderStatus();
  const {
    models, setModels, currentModel, setCurrentModel,
    parakeetModels, setParakeetModels, currentParakeetModel, setCurrentParakeetModel,
    graniteModels, setGraniteModels, currentGraniteModel, setCurrentGraniteModel,
    refreshModels,
  } = useModels(setHeaderStatus);

  const setHeaderStatusRef = useRef(setHeaderStatus);
  useEffect(() => { setHeaderStatusRef.current = setHeaderStatus; }, [setHeaderStatus]);

  const onModelDownloadedImpl = async (id: string) => {
    // Refresh the main-menu model lists immediately AND query verified status
    // in parallel so the Whisper/Parakeet cards update without delay.
    const [statuses] = await Promise.all([
      invoke<any[]>("get_download_status", { modelIds: [id] }).catch(() => null),
      refreshModels(false),
    ]);
    if (statuses) {
      const s = statuses.find((x: any) => x.id === id);
      setSettingsModels(prev => prev.map(m =>
        m.id === id ? { ...m, downloaded: true, verified: s?.verified ?? false } : m
      ));
    } else {
      setSettingsModels(prev => prev.map(m =>
        m.id === id ? { ...m, downloaded: true, verified: false } : m
      ));
    }
  };
  // Keep a stable function reference so useDownloads doesn't re-subscribe its
  // event listener on every render (which would cause missed events).
  const onModelDownloadedRef = useRef(onModelDownloadedImpl);
  useEffect(() => { onModelDownloadedRef.current = onModelDownloadedImpl; });
  const onModelDownloaded = useCallback((id: string) => onModelDownloadedRef.current(id), []);

  const onDownloadFailedImpl = async (id: string) => {
    const [statuses] = await Promise.all([
      invoke<any[]>("get_download_status", { modelIds: [id] }).catch(() => null),
      refreshModels(false),
    ]);
    if (statuses) {
      const s = statuses.find((x: any) => x.id === id);
      setSettingsModels((prev) =>
        prev.map((m) =>
          m.id === id
            ? { ...m, downloaded: s?.downloaded ?? false, verified: s?.verified ?? false }
            : m,
        ),
      );
    } else {
      setSettingsModels((prev) =>
        prev.map((m) => (m.id === id ? { ...m, downloaded: false, verified: false } : m)),
      );
    }
  };
  const onDownloadFailedRef = useRef(onDownloadFailedImpl);
  useEffect(() => { onDownloadFailedRef.current = onDownloadFailedImpl; });
  const onDownloadFailed = useCallback((id: string) => onDownloadFailedRef.current(id), []);

  const { downloadProgress, handleDownload, handleCancelDownload } = useDownloads(onModelDownloaded, onDownloadFailed);
  const downloadProgressRef = useRef(downloadProgress);
  useEffect(() => { downloadProgressRef.current = downloadProgress; });

  // On Apple Silicon, automatically download the matching CoreML encoder alongside any Whisper model.
  const WHISPER_TO_COREML: Record<string, string> = {
    'whisper-tiny-en': 'whisper-tiny-en-coreml',       'whisper-tiny-en-q5_1': 'whisper-tiny-en-coreml',
    'whisper-tiny': 'whisper-tiny-coreml',              'whisper-tiny-q5_1': 'whisper-tiny-coreml',
    'whisper-base-en': 'whisper-base-en-coreml',        'whisper-base-en-q5_1': 'whisper-base-en-coreml',
    'whisper-base': 'whisper-base-coreml',              'whisper-base-q5_1': 'whisper-base-coreml',
    'whisper-small-en': 'whisper-small-en-coreml',      'whisper-small-en-q5_1': 'whisper-small-en-coreml',
    'whisper-small': 'whisper-small-coreml',            'whisper-small-q5_1': 'whisper-small-coreml',
    'whisper-medium-en': 'whisper-medium-en-coreml',    'whisper-medium-en-q5_0': 'whisper-medium-en-coreml',
    'whisper-medium': 'whisper-medium-coreml',          'whisper-medium-q5_0': 'whisper-medium-coreml',
    'whisper-large-v3-turbo': 'whisper-large-v3-turbo-coreml', 'whisper-large-v3-turbo-q5_0': 'whisper-large-v3-turbo-coreml',
    'whisper-large-v3': 'whisper-large-v3-coreml',      'whisper-large-v3-q5_0': 'whisper-large-v3-coreml',
  };
  const handleDownloadWithCoreml = (id: string, name: string) => {
    handleDownload(id, name);
    if (isAppleSilicon) {
      const coremlId = WHISPER_TO_COREML[id];
      if (coremlId) {
        const coremlModel = settingsModels.find(m => m.id === coremlId);
        if (coremlModel && !coremlModel.downloaded) {
          handleDownload(coremlId, coremlModel.name);
        }
      }
    }
  };

  const handleDeleteModel = async (id: string, _name: string) => {
    try {
      await invoke("delete_model", { modelId: id });
      setSettingsModels(prev => prev.map(m => m.id === id ? { ...m, downloaded: false, verified: false } : m));

      // If the deleted model was the one currently loaded, turn off the LED.
      if (currentModel === id || currentParakeetModel === id || currentGraniteModel === id) {
        setLoadedEngine(null);
      }
      if (currentModel === id) setCurrentModel(null);
      if (currentParakeetModel === id) setCurrentParakeetModel(null);
      if (currentGraniteModel === id) setCurrentGraniteModel(null);

      await refreshModels(false);
    } catch (e) {
      console.error("Failed to delete model", e);
      throw e; // re-throw so ModelRow can catch it
    }
  };

  const {
    llmStatus, enableGrammarLM, setEnableGrammarLM, enableGrammarLMRef,
    enableDenoise, setEnableDenoise, enableDenoiseRef,
    enableOverlay, setEnableOverlay, enableOverlayRef,
    muteBackgroundAudio, setMuteBackgroundAudio, muteBackgroundAudioRef,
    transcriptionStyle, setTranscriptionStyle, transcriptionStyleRef,
    llmBackend, setLlmBackend,
    asrBackend, setAsrBackend,
  } = usePostProcessing(setHeaderStatus, () => setIsSettingsOpen(true));

  const {
    activeEngine, setActiveEngine, activeEngineRef,
    loadedEngine, setLoadedEngine,
    isLoading, setIsLoading, isLoadingRef,
    loadingTargetEngine, transferLineFadingOut, setTransferLineFadingOut,
    handleModelChange, handleSwitchToWhisper, handleSwitchToParakeet, handleSwitchToGranite,
  } = useEngineSwitch({
    models, parakeetModels, graniteModels,
    currentModel, currentParakeetModel, currentGraniteModel,
    setCurrentModel, setCurrentParakeetModel, setCurrentGraniteModel,
    setBackendInfo, storeRef, setHeaderStatus, setTrayState, asrBackend,
    downloadProgressRef,
  });

  const { volume, muted, setVolume, setMuted, playStart, playPaste, playError } = useSounds();

  const {
    dictionary, dictionaryRef, addDictEntry, updateDictEntry, removeDictEntry,
    snippets, snippetsRef, addSnippet, updateSnippet, removeSnippet,
  } = usePersonalization();

  const {
    isRecording, isRecordingRef, isPaused, isProcessingTranscript, isCorrecting,
    latestLatency,
    handleStartRecording, handlePauseRecording, handleResumeRecording, handleStopRecording, handleCancelRecording, handleTranscriptionChunk,
  } = useRecording({
    activeEngineRef, models, parakeetModels, graniteModels, currentModel, currentParakeetModel,
    setCurrentModel, setLoadedEngine, enableGrammarLMRef,
    enableDenoiseRef, enableOverlayRef, muteBackgroundAudioRef, transcriptionStyleRef, setHeaderStatus, setTrayState, setIsSettingsOpen,
    playStart, playPaste, playError,
    dictionaryRef, snippetsRef,
    onHistorySaved: () => setHistoryRefreshKey(k => k + 1),
  });

  // Track the engine that was loaded before a switch began (for power-routing-out visual)
  const prevLoadedEngineRef = useRef<string | null>(null);
  useEffect(() => {
    // When loading starts, snapshot the engine that was loaded just before
    if (loadingTargetEngine && loadedEngine === null) {
      // prevLoadedEngineRef already holds the previous value from the last render
    }
    // When loading ends, clear the prev
    if (!loadingTargetEngine) {
      prevLoadedEngineRef.current = null;
    }
  }, [loadingTargetEngine, loadedEngine]);
  // Keep prev updated whenever loadedEngine changes (and we're not mid-switch)
  useEffect(() => {
    if (loadedEngine && !loadingTargetEngine) {
      prevLoadedEngineRef.current = loadedEngine;
    }
  }, [loadedEngine, loadingTargetEngine]);

  // Helper to compute power-routing classes for engine cards
  const engineCardRouting = (engine: string) => {
    if (!loadingTargetEngine) return "";
    if (engine === loadingTargetEngine) return " power-routing-in";
    if (engine === prevLoadedEngineRef.current) return " power-routing-out";
    return "";
  };

  // --- Transfer line fade ---
  const transferLineFadeRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    if (!transferLineFadingOut) return;
    if (transferLineFadeRef.current) clearTimeout(transferLineFadeRef.current);
    transferLineFadeRef.current = setTimeout(() => {
      setTransferLineFadingOut(false);
      transferLineFadeRef.current = null;
    }, 450);
    return () => { if (transferLineFadeRef.current) clearTimeout(transferLineFadeRef.current); };
  }, [transferLineFadingOut]);

  // --- CPU / GPU hot-swap: reload the currently active engine on the new backend immediately ---
  const handleToggleAsrBackend = async (newBackend: "gpu" | "cpu") => {
    if (newBackend === asrBackend) return;
    if (isLoading || isLoadingRef.current) return;

    // Persist the new preference first so any subsequent engine loads use it.
    setAsrBackend(newBackend);

    // Re-invoke the current engine's load command with the new useGpu value.
    // We call the existing handlers which already manage all loading state
    // but they read asrBackend from props which hasn't updated yet in this
    // render — so we temporarily patch by doing a direct invoke pathway:
    const useGpu = newBackend === "gpu";
    const label = useGpu ? "GPU" : "CPU";
    const engine = activeEngineRef.current;

    // Fast-path: If the active engine has no loaded model, just update preference immediately.
    const hasWhisperModel = engine === "whisper" && !!currentModel;
    const hasParakeetModel = engine === "parakeet" && (currentParakeetModel || parakeetModels.length > 0);
    const hasGraniteModel = engine === "granite_speech" && graniteModels.length > 0;

    if (!hasWhisperModel && !hasParakeetModel && !hasGraniteModel) {
      setHeaderStatus(`ASR backend set to ${label}`);
      return;
    }

    // Heavy-path: Reloading the actual model into memory
    isLoadingRef.current = true;
    setIsLoading(true);
    setLoadedEngine(null);
    setLoadingMessage(`Reloading on ${label}...`);
    setHeaderStatus(`Reloading on ${label}...`, 60_000);

    try {
      await setTrayState("processing");

      if (engine === "whisper") {
        const displayName = models.find(m => m.id === currentModel)?.display_name || currentModel;
        setLoadingMessage(`Reloading ${displayName} on ${label}...`);
        await invoke("switch_model", { modelId: currentModel, useGpu });
        setLoadedEngine("whisper");
        const info = await invoke("get_backend_info");
        setBackendInfo(info as string);
        setHeaderStatus(`Whisper running on ${label}`);
      } else if (engine === "parakeet") {
        const targetModel = currentParakeetModel || parakeetModels[0]?.id;
        await invoke("init_parakeet", { modelId: targetModel, useGpu });
        setLoadedEngine("parakeet");
        const info = await invoke("get_backend_info");
        setBackendInfo(info as string);
        setHeaderStatus(`Parakeet running on ${label}`);
      } else if (engine === "granite_speech") {
        await invoke("init_granite_speech", { forceCpu: !useGpu });
        setLoadedEngine("granite_speech");
        const info = await invoke("get_backend_info");
        setBackendInfo(info as string);
        setHeaderStatus(`Granite Speech running on ${label}`);
      }
    } catch (e) {
      setHeaderStatus(`Failed to switch to ${label}: ${e}`, 5000);
    } finally {
      isLoadingRef.current = false;
      setIsLoading(false);
      setLoadingMessage("");
      setTransferLineFadingOut(true);
      await setTrayState("ready");
    }
  };

  // --- Initial data load ---
  useEffect(() => {
    let cancelled = false;

    async function loadInitialData() {
      try {
        const backend = await invoke("get_backend_info");
        if (cancelled) return;
        setBackendInfo(backend as string);

        // Pre-fetch the download status of all models
        try {
          const statuses = await invoke<any[]>("get_download_status", { modelIds: MODELS.map(m => m.id) });
          if (!cancelled) {
            setSettingsModels(prev => prev.map(m => {
              const s = statuses.find(x => x.id === m.id);
              return s ? { ...m, downloaded: s.downloaded, verified: s.verified } : m;
            }));
          }
        } catch (e) {
          console.error("Failed to fetch initial model statuses:", e);
        }

        const modelList = await invoke("list_models") as typeof models;
        if (cancelled) return;
        setModels(modelList);

        const current = await invoke("get_current_model") as string | null;
        if (cancelled) return;
        setCurrentModel(current ?? "");
        if (current) setLoadedEngine("whisper");

        const pModels = await invoke("list_parakeet_models") as typeof parakeetModels;
        if (cancelled) return;
        setParakeetModels(pModels);

        const pStatus = await invoke("get_parakeet_status") as { loaded: boolean; model_id: string | null };
        if (cancelled) return;
        setCurrentParakeetModel(pStatus.model_id ?? "");

        const gModels = await invoke("list_granite_models") as typeof graniteModels;
        if (cancelled) return;
        setGraniteModels(gModels);
        if (gModels.length > 0) setCurrentGraniteModel(gModels[0].id);

        let savedEngine: "whisper" | "parakeet" | "granite_speech" | null = null;
        try {
          const loadedStore = await Store.load("settings.json");
          if (cancelled) return;
          storeRef.current = loadedStore;
          await loadedStore.save(); // ensure the file exists on disk from first launch

          const setupComplete = await loadedStore.get<boolean>("setup_complete");
          if (!cancelled) setShowSetupWizard(setupComplete !== true);

          // Restore saved hotkey binding so the listener uses the user's preference immediately.
          const savedHotkey = await loadedStore.get<{ keys: string[] }>("hotkey_binding");
          if (savedHotkey?.keys?.length && !cancelled) {
            invoke("set_hotkey", { binding: savedHotkey }).catch(() => { });
          }

          // Restore saved input device preference.
          const savedDevice = await loadedStore.get<string>("input_device");
          if (savedDevice && !cancelled) {
            invoke("set_input_device", { name: savedDevice }).catch(() => { });
          }

          // Restore close-button behavior preference.
          const savedCloseBehavior = await loadedStore.get<'tray' | 'quit'>("close_behavior");
          if (savedCloseBehavior && !cancelled) {
            setCloseBehavior(savedCloseBehavior);
            invoke("set_close_behavior", { behavior: savedCloseBehavior }).catch(() => { });
          }

          // Restore overlay style preference.
          const savedOverlayStyle = await loadedStore.get<'minimal' | 'full'>("overlay_style");
          if (savedOverlayStyle && !cancelled) {
            setOverlayStyle(savedOverlayStyle);
          }

          savedEngine = (await loadedStore.get<"whisper" | "parakeet" | "granite_speech">("active_engine")) || null;
          if (savedEngine) {
            setActiveEngine(savedEngine);
            activeEngineRef.current = savedEngine;
          }

          const savedParakeet = await loadedStore.get<string>("parakeet_model");

          if (savedEngine === "parakeet" && pModels.length > 0) {
            const targetModel = (savedParakeet && pModels.find(m => m.id === savedParakeet))
              ? savedParakeet
              : pModels[0].id;

            isLoadingRef.current = true;
            setIsLoading(true);
            setLoadingMessage(`Loading Parakeet (${targetModel})...`);
            try {
              if (cancelled) return;
              await invoke("init_parakeet", { modelId: targetModel });
              if (cancelled) return;
              setCurrentParakeetModel(targetModel);
              setLoadedEngine("parakeet");
              setHeaderStatus("Parakeet model loaded");
            } catch (e) {
              if (cancelled) return;
              setHeaderStatus(`Failed to auto-load Parakeet: ${e}`, 5000);
            } finally {
              if (!cancelled) {
                isLoadingRef.current = false;
                setIsLoading(false);
                setLoadingMessage("");
              }
            }
          } else if (savedEngine === "granite_speech" && gModels.length > 0) {
            isLoadingRef.current = true;
            setIsLoading(true);
            setLoadingMessage("Loading Granite Speech...");
            try {
              if (cancelled) return;
              await invoke("init_granite_speech", {});
              if (cancelled) return;
              setCurrentGraniteModel(gModels[0].id);
              setLoadedEngine("granite_speech");
              setHeaderStatus("Granite Speech model loaded");
            } catch (e) {
              if (cancelled) return;
              setHeaderStatus(`Failed to auto-load Granite Speech: ${e}`, 5000);
            } finally {
              if (!cancelled) {
                isLoadingRef.current = false;
                setIsLoading(false);
                setLoadingMessage("");
              }
            }
          }
        } catch (storeErr) {
          console.warn("Store load failed:", storeErr);
          if (!cancelled) setShowSetupWizard(true);
        }

        if (!cancelled && pStatus.loaded && !current && !savedEngine) {
          setActiveEngine("parakeet");
          activeEngineRef.current = "parakeet";
        }
      } catch (e) {
        if (cancelled) return;
        console.error("Failed to load initial data:", e);
        setBackendInfo("Unknown");
        setHeaderStatus(`Error loading models: ${e}`, 5000);
        setShowSetupWizard(false);
      } finally {
        if (!cancelled) {
          setIsInitialLoading(false);
          invoke("show_main_window").catch(() => { });
        }
      }
    }

    loadInitialData();
    return () => { cancelled = true; };
  }, []);

  // --- Sync active engine with backend & persist ---
  useEffect(() => {
    if (!isInitialLoading) {
      invoke("set_active_engine", { engine: activeEngine }).catch(console.error);
      if (storeRef.current) {
        storeRef.current.set("active_engine", activeEngine).then(() => storeRef.current?.save());
      }
    }
  }, [activeEngine, isInitialLoading]);

  // --- File system watcher for models dir & verification status ---
  const refreshModelsRef = useRef(refreshModels);
  useEffect(() => { refreshModelsRef.current = refreshModels; });

  // downloadProgressRef is defined near the top of the component (after useDownloads)
  // so it's available to both useEngineSwitch and the FS watcher callback below.

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    const handleModelsChanged = async () => {
      // Refresh backend model lists (Whisper + Parakeet)
      refreshModelsRef.current(false);

      // Also refresh AppMall status (downloaded / verified flags) so the UI
      // reflects SHA-256 verification results as soon as they complete.
      try {
        const statuses = await invoke<any[]>("get_download_status", { modelIds: MODELS.map(m => m.id) });
        if (!active) return;
        const activeOps = downloadProgressRef.current;
        setSettingsModels(prev =>
          prev.map(m => {
            // Don't overwrite state for models with an active operation
            // (download, verify, delete) — the FS watcher sees partial
            // files on disk and would prematurely report them as downloaded.
            const op = activeOps[m.id];
            if (op && ['starting', 'downloading', 'extracting', 'verifying', 'finalizing', 'deleting'].includes(op.status)) {
              return m;
            }
            const s = statuses.find(x => x.id === m.id);
            return s ? { ...m, downloaded: s.downloaded, verified: s.verified } : m;
          }),
        );
      } catch (e) {
        console.error("Failed to refresh model statuses after models-changed:", e);
      }
    };

    const setup = async () => {
      const unsub = await listen("models-changed", handleModelsChanged);
      if (active) unlisten = unsub;
      else unsub();
    };

    setup();
    return () => {
      active = false;
      if (unlisten) unlisten();
    };
  }, []);

  // --- Hotkey listeners ---
  const handleStartRecordingRef = useRef(handleStartRecording);
  const handlePauseRecordingRef = useRef(handlePauseRecording);
  const handleResumeRecordingRef = useRef(handleResumeRecording);
  const handleStopRecordingRef = useRef(handleStopRecording);
  const handleCancelRecordingRef = useRef(handleCancelRecording);
  const handleTranscriptionChunkRef = useRef(handleTranscriptionChunk);
  useEffect(() => {
    handleStartRecordingRef.current = handleStartRecording;
    handlePauseRecordingRef.current = handlePauseRecording;
    handleResumeRecordingRef.current = handleResumeRecording;
    handleStopRecordingRef.current = handleStopRecording;
    handleCancelRecordingRef.current = handleCancelRecording;
    handleTranscriptionChunkRef.current = handleTranscriptionChunk;
  });

  const startingRecordingRef = useRef(false);
  const pendingStopRef = useRef(false);
  const stopInProgressRef = useRef(false);
  const lastStartTime = useRef(0);
  const lastStopTime = useRef(0);
  const HOTKEY_DEBOUNCE_MS = 700;

  useEffect(() => {
    let active = true;
    let unlistenStart: (() => void) | undefined;
    let unlistenStop: (() => void) | undefined;
    let unlistenChunk: (() => void) | undefined;
    let unlistenAccessibility: (() => void) | undefined;
    let unlistenAudioFallback: (() => void) | undefined;
    let unlistenAudioDisconnect: (() => void) | undefined;
    let unlistenOverlayAction: (() => void) | undefined;
    const setup = async () => {
      const unsub1 = await listen("hotkey-start-recording", async () => {
        const now = Date.now();
        if (now - lastStartTime.current < HOTKEY_DEBOUNCE_MS) return;
        lastStartTime.current = now;

        // Don't start if model is loading, already recording, starting, or processing a previous stop
        if (isLoadingRef.current || isRecordingRef.current || startingRecordingRef.current || stopInProgressRef.current) return;

        startingRecordingRef.current = true;
        pendingStopRef.current = false;
        await handleStartRecordingRef.current(true);
        startingRecordingRef.current = false;
        if (pendingStopRef.current) {
          pendingStopRef.current = false;
          setTimeout(async () => { await handleStopRecordingRef.current(); }, 250);
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
          await handleStopRecordingRef.current();
        } finally {
          stopInProgressRef.current = false;
        }
      });

      const unsub3 = await listen<{ text: string }>("transcription-chunk", (event) => {
        handleTranscriptionChunkRef.current(event.payload.text);
      });

      // macOS fix: Listen for accessibility-missing event from the Rust backend.
      // Shows a dismissible warning banner prompting the user to grant
      // Accessibility & Input Monitoring permissions in System Settings.
      const unsub4 = await listen("accessibility-missing", () => {
        setAccessibilityMissing(true);
      });

      const unsub5 = await listen("audio-fallback", (event) => {
        const deviceName = event.payload as string;
        setHeaderStatusRef.current(`Mic lost, using fallback: ${deviceName}`, 6000);
      });

      const unsub6 = await listen("audio-disconnected", (_event) => {
        setHeaderStatusRef.current("Microphone disconnected! Recording stopped.", 6000);
        if (isRecordingRef.current && !stopInProgressRef.current) {
          stopInProgressRef.current = true;
          handleStopRecordingRef.current().finally(() => {
            stopInProgressRef.current = false;
          });
        }
      });

      const unsub7 = await listen<string>("overlay-action", async (event) => {
        const action = String(event.payload);
        if (action === "pause") {
          await handlePauseRecordingRef.current();
          return;
        }
        if (action === "resume") {
          await handleResumeRecordingRef.current();
          return;
        }
        if (action === "cancel") {
          if (stopInProgressRef.current) return;
          stopInProgressRef.current = true;
          try {
            await handleCancelRecordingRef.current();
          } finally {
            stopInProgressRef.current = false;
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
      } else {
        unsub1(); unsub2(); unsub3(); unsub4(); unsub5(); unsub6(); unsub7();
      }
    };

    setup();
    return () => {
      active = false;
      if (unlistenStart) unlistenStart();
      if (unlistenStop) unlistenStop();
      if (unlistenChunk) unlistenChunk();
      if (unlistenAccessibility) unlistenAccessibility();
      if (unlistenAudioFallback) unlistenAudioFallback();
      if (unlistenAudioDisconnect) unlistenAudioDisconnect();
      if (unlistenOverlayAction) unlistenOverlayAction();
    };
  }, []);

  // --- Ticker ---
  // macOS fix: Filter ticker phrases to remove "CUDA" from the scrolling
  // ticker on macOS, since CUDA is not available on Apple Silicon.
  const filteredTickerPhrases = useMemo(() => {
    if (!isMac) return TICKER_PHRASES;
    return TICKER_PHRASES.map(phrase => {
      const flat = phrase.parts.map(p => p.text).join('');
      if (flat === 'CUDA · CPU · Metal · flexible backends') {
        return { parts: [{ text: 'Metal · CPU · flexible backends' }] };
      }
      return phrase;
    });
  }, [isMac]);

  const tickerContent = useMemo(() => (
    <>
      {filteredTickerPhrases.flatMap((phrase, i) => [
        i > 0 ? <span key={`sep-${i}`} className="ticker-sep"> — </span> : null,
        <span key={i} className="header-ticker-phrase">
          {phrase.parts.map((p, j) => {
            if (!p.highlight) return p.text;
            const cls = p.highlight === "whisper" ? "ticker-whisper" : p.highlight === "parakeet" ? "ticker-parakeet" : p.highlight === "granite" ? "ticker-granite" : "ticker-accent";
            return <span key={j} className={cls}>{p.text}</span>;
          })}
        </span>,
      ]).filter(Boolean)}
    </>
  ), [filteredTickerPhrases]);

  // --- Derived UI state ---
  const noWhisperModel = models.length === 0;
  const noParakeetModel = parakeetModels.length === 0;
  const noGraniteModel = graniteModels.length === 0;
  const noAnyAsrModel = noWhisperModel && noParakeetModel && noGraniteModel;
  const activeEngineHasNoModel =
    (activeEngine === "whisper" && noWhisperModel) ||
    (activeEngine === "parakeet" && noParakeetModel) ||
    (activeEngine === "granite_speech" && noGraniteModel);
  const noModel = activeEngineHasNoModel;
  const noLlm = llmStatus === "Not Downloaded";

  const recordBtnBusy = isLoading || isCorrecting || isProcessingTranscript;
  const recordBtnClass =
    noModel ? "record-btn disabled" :
      isRecording ? "record-btn recording" :
        recordBtnBusy ? "record-btn processing" :
          "record-btn idle";
  const recordBtnLabel =
    noModel ? "NO MODEL" :
      isRecording ? "STOP" :
        recordBtnBusy ? "..." : "REC";
  const recordBtnDisabled = (isLoading && !isRecording) || isCorrecting || isProcessingTranscript;

  const onRecordClick = () => {
    if (noModel) { setIsSettingsOpen(true); return; }
    if (isRecording) handleStopRecording();
    else handleStartRecording();
  };

  const colorizeStatusMessage = (msg: string) => {
    const parts = msg.split(/(Granite Speech|Whisper|Parakeet|Granite|OpenAI|NVIDIA|IBM)/g);
    return parts.map((part, i) => {
      if (part === "Whisper" || part === "OpenAI") return <span key={i} style={{ color: 'var(--whisper-color)' }}>{part}</span>;
      if (part === "Parakeet" || part === "NVIDIA") return <span key={i} style={{ color: 'var(--parakeet-color)' }}>{part}</span>;
      if (part === "Granite Speech" || part === "Granite" || part === "IBM") return <span key={i} style={{ color: 'var(--granite-color)' }}>{part}</span>;
      return part;
    });
  };

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
      <div className="app-loading" style={{ minHeight: "100vh", display: "flex", alignItems: "center", justifyContent: "center", background: "var(--bg-primary, #09090b)", color: "var(--text-secondary)" }}>
        Loading…
      </div>
    );
  }

  if (showSetupWizard === true) {
    return <SetupWizard onComplete={handleSetupComplete} />;
  }

  return (
    <>
      <TitleBar />
      <div className={`app-body ${isRecording ? "app-body--recording" : ""} theme-${activeEngine}`}>
        <main className={`container${containerBooting ? " container--booting" : ""}`}>
          <div>
            <div className="app-header">
              <div className="app-title-container">
                {/* H1 fix: wrapped in <button> so it's keyboard-reachable and
                    announced as interactive by screen readers */}
                <button
                  type="button"
                  className="logo-btn"
                  onClick={handleLogoClick}
                  aria-label="Cycle logo animation"
                  title="Cycle Logo"
                >
                  <img
                    key={randomLogo}
                    src={`/logos/${randomLogo}`}
                    alt=""
                    className={`app-title-logo ${isLogoShuttering ? "app-title-logo--shutter" : ""}`}
                  />
                </button>
                <h1 className={`app-title ${isBooting ? "app-title--boot" : ""}`}>
                  TAURSCRIBE
                </h1>
              </div>
              <div className="header-status">
                {headerStatusMessage !== null ? (
                  <span
                    className={`header-status-message ${headerStatusIsProcessing ? "header-status-message--processing" : ""}`}
                    key={headerStatusMessage}
                  >
                    {colorizeStatusMessage(headerStatusMessage)}
                  </span>
                ) : (
                  <div className="header-ticker header-ticker-fade-in" aria-hidden="true">
                    <div className="header-ticker-track">
                      <span className="header-ticker-segment">{tickerContent}</span>
                      <span className="header-ticker-segment" aria-hidden="true">{tickerContent}</span>
                    </div>
                  </div>
                )}
              </div>
              {/* L4 fix: replaced inline SVG with IconSettings from Icons.tsx */}
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
            <div className="hardware-bar">
              <span>Hardware: <span>{backendInfo}</span></span>
              {/* macOS fix: Hide the GPU/CPU toggle on macOS — Apple Silicon uses
                  Metal automatically and there is no discrete GPU to switch. */}
              {!isMac && (
                <div className="backend-toggle-inline">
                  {/* M5 fix: aria-pressed communicates toggle state to screen readers */}
                  <button
                    className={`backend-toggle-inline-btn ${asrBackend === 'gpu' ? 'active' : ''}`}
                    onClick={() => handleToggleAsrBackend('gpu')}
                    disabled={isLoading}
                    aria-pressed={asrBackend === 'gpu'}
                  ><IconBolt size={11} style={{ color: '#facc15' }} /> GPU</button>
                  <button
                    className={`backend-toggle-inline-btn ${asrBackend === 'cpu' ? 'active' : ''}`}
                    onClick={() => handleToggleAsrBackend('cpu')}
                    disabled={isLoading}
                    aria-pressed={asrBackend === 'cpu'}
                  ><IconCpu size={11} /> CPU</button>
                  <InfoTooltip size={11} text="GPU for max speed; CPU if no GPU or to save VRAM." />
                </div>
              )}
            </div>

            {/* Microphone selector — sits right below the hardware bar so the
                user can see and switch the active mic without opening Settings.
                The dropdown lists all available input devices; selecting one
                persists the choice to settings.json and updates the backend. */}
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
                onFocus={() => {
                  invoke<string[]>('list_input_devices').then(setInputDevices).catch(() => {});
                }}
                onMouseEnter={() => {
                  invoke<string[]>('list_input_devices').then(setInputDevices).catch(() => {});
                }}
              >
                <option value="">System Default</option>
                {inputDevices.map((d) => (
                  <option key={d} value={d}>{d}</option>
                ))}
              </select>
              <InfoTooltip size={11} text="Input device. Changes apply on next recording." />
            </div>

            {/* macOS fix: Show a warning banner when Accessibility permission
                is not granted. Without it, the global hotkey listener (rdev)
                silently fails to receive any key events. */}
            {accessibilityMissing && (
              <div className="accessibility-banner">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={{ flexShrink: 0 }}>
                  <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
                  <line x1="12" y1="9" x2="12" y2="13" />
                  <line x1="12" y1="17" x2="12.01" y2="17" />
                </svg>
                <span>Hotkeys disabled — grant <strong>Accessibility</strong> &amp; <strong>Input Monitoring</strong> permission in System Settings → Privacy &amp; Security, then restart the app.</span>
                <button type="button" className="accessibility-banner-dismiss" onClick={() => setAccessibilityMissing(false)} aria-label="Dismiss">✕</button>
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
                  <span>Microphone access denied — open <strong>System Settings → Privacy &amp; Security → Microphone</strong> and enable Taurscribe, then restart the app.</span>
                )}
                <button type="button" className="mic-banner-dismiss" onClick={() => setMicPermission(null)} aria-label="Dismiss">✕</button>
              </div>
            )}
          </div>

          <div className="status-bar-container">
            {(isLoading || transferLineFadingOut || isProcessingTranscript || isCorrecting) && (
              <div
                className={`status-bar-transfer-line ${
                  transferLineFadingOut ? "status-bar-transfer-line--fade-out" : ""
                } ${
                  isProcessingTranscript || isCorrecting ? "status-bar-transfer-line--active" : ""
                }`}
                aria-hidden="true"
              />
            )}

            <div
              className={`status-card whisper ${activeEngine === "whisper" ? "active" : ""}${engineCardRouting("whisper")}`}
              onClick={handleSwitchToWhisper}
              style={isLoading ? { pointerEvents: 'none' } : {}}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => e.key === "Enter" && handleSwitchToWhisper()}
            >
              <div className="status-card-header">
                <span className="engine-badge">Whisper</span>
                <div className="status-card-header-right">
                  <span className="info-icon" data-tooltip="OpenAI Whisper — general-purpose speech recognition. Supports multiple languages, sizes from Tiny to Large, and runs locally on CPU or GPU.">
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <circle cx="12" cy="12" r="10" />
                      <line x1="12" y1="16" x2="12" y2="12" />
                      <line x1="12" y1="8" x2="12.01" y2="8" />
                    </svg>
                  </span>
                  <span
                    className={`led-dot ${loadingTargetEngine === "whisper" ? "loading" :
                      loadedEngine === "whisper" ? "loaded" : "unloaded"
                      }`}
                    aria-label={loadingTargetEngine === "whisper" ? "Loading" : loadedEngine === "whisper" ? "Loaded" : "Unloaded"}
                  />
                </div>
              </div>
              <div className="status-item">
                <span className="status-label">Model</span>
                <span className={`status-value ${models.length === 0 ? "error" : ""}`}>
                  {models.length === 0 ? "Download required" : (currentModel ? beautifyModelName(models.find(m => m.id === currentModel)?.display_name || currentModel) : "None")}
                </span>
              </div>
            </div>

            <div
              className={`status-card parakeet ${activeEngine === "parakeet" ? "active" : ""}${engineCardRouting("parakeet")}`}
              onClick={handleSwitchToParakeet}
              style={isLoading ? { pointerEvents: 'none' } : {}}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => e.key === "Enter" && handleSwitchToParakeet()}
            >
              <div className="status-card-header">
                <span className="engine-badge">Parakeet</span>
                <div className="status-card-header-right">
                  <span className="info-icon" data-tooltip="NVIDIA Parakeet — fast streaming ASR optimized for English. Real-time transcription with low latency using CTC decoding.">
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <circle cx="12" cy="12" r="10" />
                      <line x1="12" y1="16" x2="12" y2="12" />
                      <line x1="12" y1="8" x2="12.01" y2="8" />
                    </svg>
                  </span>
                  <span
                    className={`led-dot ${loadingTargetEngine === "parakeet" ? "loading" :
                      loadedEngine === "parakeet" ? "loaded" : "unloaded"
                      }`}
                    aria-label={loadingTargetEngine === "parakeet" ? "Loading" : loadedEngine === "parakeet" ? "Loaded" : "Unloaded"}
                  />
                </div>
              </div>
              <div className="status-item">
                <span className="status-label">Model</span>
                <span className={`status-value ${parakeetModels.length === 0 && !Object.keys(downloadProgress).some(k => k.startsWith('parakeet')) ? "error" : Object.keys(downloadProgress).some(k => k.startsWith('parakeet')) ? "processing" : ""}`}>
                  {Object.keys(downloadProgress).some(k => k.startsWith('parakeet')) ? "Downloading…" : parakeetModels.length === 0 ? "Download required" : (parakeetModels.find(m => m.id === currentParakeetModel) ?? parakeetModels[0]).display_name.split(' - ')[0].replace(/\s*\(.*?\)/g, '').trim()}
                </span>
              </div>
            </div>

            <div
              className={`status-card granite ${activeEngine === "granite_speech" ? "active" : ""}${engineCardRouting("granite_speech")}`}
              onClick={handleSwitchToGranite}
              style={isLoading ? { pointerEvents: 'none' } : {}}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => e.key === "Enter" && handleSwitchToGranite()}
            >
              <div className="status-card-header">
                <span className="engine-badge">Granite</span>
                <div className="status-card-header-right">
                  <span className="info-icon" data-tooltip="IBM Granite 4.0 1B Speech — encoder-decoder ONNX model. English speech recognition.">
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <circle cx="12" cy="12" r="10" />
                      <line x1="12" y1="16" x2="12" y2="12" />
                      <line x1="12" y1="8" x2="12.01" y2="8" />
                    </svg>
                  </span>
                  <span
                    className={`led-dot ${loadingTargetEngine === "granite_speech" ? "loading" :
                      loadedEngine === "granite_speech" ? "loaded" : "unloaded"
                      }`}
                    aria-label={loadingTargetEngine === "granite_speech" ? "Loading" : loadedEngine === "granite_speech" ? "Loaded" : "Unloaded"}
                  />
                </div>
              </div>
              <div className="status-item">
                <span className="status-label">Model</span>
                <span className={`status-value ${graniteModels.length === 0 && !Object.keys(downloadProgress).some(k => k.startsWith('granite')) ? "error" : Object.keys(downloadProgress).some(k => k.startsWith('granite')) ? "processing" : ""}`}>
                  {Object.keys(downloadProgress).some(k => k.startsWith('granite')) ? "Downloading…" : graniteModels.length === 0 ? "Download required" : (graniteModels.find(m => m.id === currentGraniteModel) ?? graniteModels[0]).display_name}
                </span>
              </div>
            </div>
          </div>

          <div className="model-row">
            <div className="model-section">
              <div key={activeEngine} className="model-content">
                {activeEngine === "whisper" ? (
                  <>
                    <label htmlFor="model-select" className="model-label">Active model</label>
                    <select
                      id="model-select"
                      className="model-select"
                      value={currentModel || ""}
                      onChange={(e) => handleModelChange(e.target.value)}
                      disabled={isRecording || isLoading || isInitialLoading}
                    >
                      {isInitialLoading && <option value="">Loading models...</option>}
                      {!isInitialLoading && models.filter(m => !downloadProgress['whisper-' + m.id.replace('.', '-')]).length === 0 && (
                        <option value="">
                          {Object.keys(downloadProgress).some(k => k.startsWith('whisper-'))
                            ? 'Downloading model...'
                            : 'No models — open Settings to download'}
                        </option>
                      )}
                      {models
                        .filter(m => !downloadProgress['whisper-' + m.id.replace('.', '-')])
                        .map((model) => (
                          <option key={model.id} value={model.id}>
                            {beautifyModelName(model.display_name)} ({formatSize(model.size_mb)}){model.has_coreml ? ' ⚡' : ''}
                          </option>
                        ))}
                    </select>
                  </>
                ) : activeEngine === "parakeet" ? (
                  <>
                    <span className="model-label">Active model</span>
                    <div
                      className="model-select"
                      style={{
                        display: 'flex',
                        alignItems: 'center',
                        justifyContent: 'center',
                        cursor: 'default',
                        background: parakeetModels.length === 0 ? 'rgba(220, 38, 38, 0.08)' : 'var(--bg-tertiary)',
                        color: parakeetModels.length === 0 ? 'var(--error)' : 'inherit'
                      }}
                    >
                      {isInitialLoading ? "Loading..." : (
                        parakeetModels.length === 0
                          ? "Download Nemotron from Settings"
                          : `${beautifyModelName(parakeetModels[0]?.display_name || "Nemotron")} (${formatSize(parakeetModels[0]?.size_mb || 0)})`
                      )}
                    </div>
                  </>
                ) : (
                  <>
                    <span className="model-label">Active model</span>
                    <div
                      className="model-select"
                      style={{
                        display: 'flex',
                        alignItems: 'center',
                        justifyContent: 'center',
                        cursor: 'default',
                        background: graniteModels.length === 0 ? 'rgba(220, 38, 38, 0.08)' : 'var(--bg-tertiary)',
                        color: graniteModels.length === 0 ? 'var(--error)' : 'inherit'
                      }}
                    >
                      {isInitialLoading ? "Loading..." : (
                        graniteModels.length === 0
                          ? "Download Granite Speech from Settings"
                          : `${graniteModels[0]?.display_name} (${formatSize(graniteModels[0]?.size_mb || 0)})`
                      )}
                    </div>
                  </>
                )}
              </div>

              {/* ── LLM toggle + tone pills ── */}
              <div className="llm-section">
                <div className="llm-section-header">
                  <div className="llm-identity">
                    <span
                      className="llm-status-dot"
                      style={{
                        background: !enableGrammarLM ? 'var(--text-muted)' :
                          llmStatus === 'Loaded' ? 'var(--success)' :
                            llmStatus === 'Loading...' ? 'var(--warning)' : 'var(--error)'
                      }}
                    />
                    <span className="llm-name">FlowScribe 2.5 0.5B</span>
                    <InfoTooltip size={11} text="Fine-tuned Qwen 2.5 0.5B. Fixes grammar, punctuation & tone after each recording." />
                    {/* macOS fix: Hide the GPU/CPU backend badge on macOS since
                        there is no GPU/CPU choice — Metal is used automatically. */}
                    {llmStatus === 'Loaded' && !isMac && (
                      <span className={`llm-backend-badge llm-backend-badge--${llmBackend}`} title={llmBackend === 'gpu' ? 'Currently using GPU acceleration' : 'Currently using CPU (slower)'}>
                        {llmBackend === 'gpu' ? <><IconBolt size={10} style={{ color: '#facc15' }} /> GPU</> : <><IconCpu size={10} /> CPU</>}
                      </span>
                    )}
                    {llmStatus === 'Loading...' && (
                      <span className="llm-backend-badge llm-backend-badge--loading">switching…</span>
                    )}
                    <span className="llm-meta">fine-tuned · grammar & tone</span>
                  </div>
                  <label className={`mini-toggle ${llmStatus === 'Not Downloaded' ? 'mini-toggle--disabled' : ''}`} title={llmStatus === 'Not Downloaded' ? 'Download FlowScribe Qwen from Settings > Models' : enableGrammarLM ? 'Disable grammar LLM' : 'Enable grammar LLM'}>
                    <input
                      type="checkbox"
                      checked={enableGrammarLM}
                      onChange={e => setEnableGrammarLM(e.target.checked)}
                      disabled={llmStatus === 'Loading...' || llmStatus === 'Not Downloaded'}
                    />
                    <span className="mini-toggle-track" />
                  </label>
                </div>
                {llmStatus === 'Not Downloaded' && (
                  <p className="llm-section-hint" style={{ color: 'var(--error)', marginTop: '6px', fontSize: '0.8rem' }}>
                    Model not downloaded. Download FlowScribe Qwen from Settings → Models.
                  </p>
                )}
                <div className={`tone-tiles ${!enableGrammarLM ? 'tone-tiles--off' : ''}`}>
                  {TONE_STYLES.map(s => {
                    const isActive = transcriptionStyle === s.value;
                    return (
                      <button
                        key={s.value}
                        className={`tone-tile ${isActive ? 'tone-tile--active' : ''}`}
                        onClick={() => setTranscriptionStyle(s.value)}
                        disabled={!enableGrammarLM || llmStatus !== 'Loaded'}
                        style={{
                          '--tile-accent': s.accent,
                          '--tile-accent-glow': `${s.accent}40`,
                          '--tile-accent-bg': `${s.accent}14`,
                        } as React.CSSProperties}
                      >
                        <div className="tone-tile-stripe" />
                        <span className="tone-tile-icon">{s.icon}</span>
                        <span className="tone-tile-label">{s.label}</span>
                        <span className="tone-tile-desc">{s.desc}</span>
                      </button>
                    );
                  })}
                </div>
              </div>
            </div>
            <div className="record-btn-wrap">
              <button
                type="button"
                className={recordBtnClass}
                disabled={!noModel && recordBtnDisabled}
                onClick={onRecordClick}
                title={noModel ? "Download a model first in Settings" : recordBtnBusy ? "Please wait…" : isRecording ? "Stop recording" : "Start recording"}
              >
                {recordBtnLabel}
              </button>
            </div>
          </div>

          {isInitialLoading && (
            <div className="loading-overlay-backdrop" aria-busy="true" aria-live="polite">
              <div className="loading-overlay">
                <div className="loading-spinner" />
                <span className="loading-text">{loadingMessage || "Loading..."}</span>
              </div>
            </div>
          )}

          {/* Mic / File mode toggle */}
          <div className="mode-toggle">
            <button
              type="button"
              className={`mode-toggle-btn${!fileMode ? " mode-toggle-btn--active" : ""}`}
              onClick={() => setFileMode(false)}
              disabled={fileMode && isFileTranscribing}
              title={fileMode && isFileTranscribing ? "Wait for file transcription to finish" : undefined}
            >
              <IconMic size={13} /> Microphone
            </button>
            <button
              type="button"
              className={`mode-toggle-btn${fileMode ? " mode-toggle-btn--active" : ""}`}
              onClick={() => setFileMode(true)}
            >
              <IconFileText size={13} /> File
            </button>
          </div>

          <div className="output-area output-area--feed">
            <div style={fileMode ? undefined : { display: 'none' }}>
              <FileTranscriptionPanel
                activeEngine={activeEngine}
                currentModel={currentModel}
                currentParakeetModel={currentParakeetModel}
                currentGraniteModel={currentGraniteModel}
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
                        : "Granite Speech not downloaded"}
                </h2>
                <p className="empty-state-body">
                  {noAnyAsrModel ? (
                    <>Download a <strong>Whisper</strong>, <strong>Parakeet</strong>, or <strong>Granite Speech</strong> model to start transcribing. Whisper Base is a good starting point — it's fast and accurate.</>
                  ) : activeEngine === "whisper" ? (
                    <>You're on the <strong>Whisper</strong> engine but haven't downloaded a model yet. Try <strong>Whisper Base</strong> — it's small and accurate. Or switch to Parakeet if you already have it.</>
                  ) : activeEngine === "parakeet" ? (
                    <>You're on the <strong>Parakeet</strong> engine but the Nemotron model isn't downloaded yet. Switch to Whisper if you already have a model, or download Parakeet from Settings.</>
                  ) : (
                    <>You're on the <strong>Granite Speech</strong> engine but the model isn't downloaded yet. Switch to Whisper or Parakeet if you already have a model, or download Granite Speech from Settings.</>
                  )}
                </p>
                {!noAnyAsrModel && (
                  <p className="empty-state-hint">
                    {activeEngine === "whisper" && !noParakeetModel
                      ? <><IconLightbulb size={14} /> You already have a Parakeet model — click the Parakeet card above to switch.</>
                      : activeEngine === "parakeet" && !noWhisperModel
                        ? <><IconLightbulb size={14} /> You already have a Whisper model — click the Whisper card above to switch.</>
                        : activeEngine === "granite_speech" && !noWhisperModel
                          ? <><IconLightbulb size={14} /> You already have a Whisper model — click the Whisper card above to switch.</>
                          : null}
                  </p>
                )}
                <button
                  type="button"
                  className="empty-state-cta"
                  onClick={() => setIsSettingsOpen(true)}
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
                isCorrecting={isCorrecting}
                latestLatency={latestLatency}
              />
            ))}
          </div>

          <SettingsModal
            isOpen={isSettingsOpen}
            onClose={() => {
              setIsSettingsOpen(false);
              // Refresh the mic dropdown in case the user changed the device in Settings.
              invoke<string>('get_active_input_device').then(setActiveMic).catch(() => {});
              invoke<string[]>('list_input_devices').then(setInputDevices).catch(() => {});
            }}
            initialTab={settingsInitialTab as Parameters<typeof SettingsModal>[0]['initialTab']}
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
            onCancelDownload={handleCancelDownload}
            closeBehavior={closeBehavior}
            setCloseBehavior={setCloseBehavior}
            overlayStyle={overlayStyle}
            setOverlayStyle={setOverlayStyle}
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
          transcriptionStyle={transcriptionStyle}
          setTranscriptionStyle={setTranscriptionStyle}
          llmBackend={llmBackend}
          setLlmBackend={setLlmBackend}
          soundVolume={volume}
          soundMuted={muted}
          setSoundVolume={setVolume}
          setSoundMuted={setMuted}
          dictionaryCount={dictionary.length}
          snippetsCount={snippets.length}
          onOpenSettingsTab={(tab) => {
            setSettingsInitialTab(tab);
            setIsSettingsOpen(true);
          }}
        />
      </div>
    </>
  );
}

export default App;
