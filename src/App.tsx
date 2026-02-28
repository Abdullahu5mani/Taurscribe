import { useEffect, useRef, useCallback } from "react";
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
import "./components/TitleBar.css";
import "./App.css";

// Ticker phrases defined outside the component so the array is never recreated on render
type TickerHighlight = "accent" | "whisper" | "parakeet";
const TICKER_PHRASES: { parts: { text: string; highlight?: TickerHighlight }[] }[] = [
  { parts: [{ text: "100% " }, { text: "local", highlight: "accent" }, { text: " · nothing leaves your machine" }] },
  { parts: [{ text: "OpenAI " }, { text: "Whisper", highlight: "whisper" }, { text: " & NVIDIA " }, { text: "Parakeet", highlight: "parakeet" }, { text: " · GPU-accelerated" }] },
  { parts: [{ text: "Hit " }, { text: "REC", highlight: "accent" }, { text: " · speech to text in real time" }] },
  { parts: [{ text: "No cloud", highlight: "accent" }, { text: " · no API keys · no subscriptions" }] },
  { parts: [{ text: "Switch between " }, { text: "Whisper", highlight: "whisper" }, { text: " and " }, { text: "Parakeet", highlight: "parakeet" }, { text: " anytime" }] },
  { parts: [{ text: "Ctrl+Win", highlight: "accent" }, { text: " from anywhere to record" }] },
  { parts: [{ text: "Grammar & spell check · optional " }, { text: "LLM", highlight: "accent" }, { text: "" }] },
  { parts: [{ text: "Offline-first", highlight: "accent" }, { text: " · your data stays yours" }] },
  { parts: [{ text: "Pick your engine · " }, { text: "Whisper", highlight: "whisper" }, { text: " for OpenAI · " }, { text: "Parakeet", highlight: "parakeet" }, { text: " for NVIDIA" }] },
  { parts: [{ text: "Studio-grade", highlight: "accent" }, { text: " · runs on your hardware" }] },
  { parts: [{ text: "Real-time transcription with " }, { text: "Whisper", highlight: "whisper" }, { text: " or " }, { text: "Parakeet", highlight: "parakeet" }, { text: "" }] },
  { parts: [{ text: "Your audio never leaves this device" }] },
  { parts: [{ text: "CUDA · CPU · Metal · flexible backends" }] },
  { parts: [{ text: "Download models once · use forever" }] },
  { parts: [{ text: "Built for privacy · built for speed" }] },
  { parts: [{ text: "Two engines · " }, { text: "Whisper", highlight: "whisper" }, { text: " & " }, { text: "Parakeet", highlight: "parakeet" }, { text: " · one app" }] },
  { parts: [{ text: "Press REC and speak · that's it" }] },
  { parts: [{ text: "No account", highlight: "accent" }, { text: " · no sign-up · no tracking" }] },
  { parts: [{ text: "Low latency · high accuracy" }] },
  { parts: [{ text: "Use " }, { text: "Whisper", highlight: "whisper" }, { text: " for batch · " }, { text: "Parakeet", highlight: "parakeet" }, { text: " for streaming" }] },
  { parts: [{ text: "Desktop-first · always ready" }] },
  { parts: [{ text: "Your words · your machine · your rules" }] },
  { parts: [{ text: "Multilingual " }, { text: "Whisper", highlight: "whisper" }, { text: " · real-time " }, { text: "Parakeet", highlight: "parakeet" }, { text: "" }] },
  { parts: [{ text: "Transcribe meetings · notes · ideas" }] },
  { parts: [{ text: "One click to record", highlight: "accent" }, { text: " · one click to copy" }] },
  { parts: [{ text: "GPU-accelerated when you have it" }] },
  { parts: [{ text: "Open source models · open future" }] },
  { parts: [{ text: "From " }, { text: "Whisper", highlight: "whisper" }, { text: " to " }, { text: "Parakeet", highlight: "parakeet" }, { text: " in one tap" }] },
  { parts: [{ text: "Privacy by design", highlight: "accent" }, { text: " · not as an afterthought" }] },
  { parts: [{ text: "Capture every word · edit later" }] },
  { parts: [{ text: "No internet? No problem." }] },
  { parts: [{ text: "Tiny to large · pick your " }, { text: "Whisper", highlight: "whisper" }, { text: " size" }] },
  { parts: [{ text: "Streaming with " }, { text: "Parakeet", highlight: "parakeet" }, { text: " · see text as you speak" }] },
  { parts: [{ text: "Hotkey ready", highlight: "accent" }, { text: " · Ctrl+Win from any app" }] },
  { parts: [{ text: "Local AI · no data in the cloud" }] },
  { parts: [{ text: "Built for creators · built for you" }] },
  { parts: [{ text: "Switch engines mid-workflow" }] },
  { parts: [{ text: "Spell check · grammar · optional" }] },
  { parts: [{ text: "Whisper", highlight: "whisper" }, { text: " for accuracy · " }, { text: "Parakeet", highlight: "parakeet" }, { text: " for speed" }] },
  { parts: [{ text: "Your microphone · your transcript" }] },
  { parts: [{ text: "Download once · run anywhere" }] },
  { parts: [{ text: "No subscriptions", highlight: "accent" }, { text: " · pay with your hardware" }] },
  { parts: [{ text: "Transcription that respects you" }] },
  { parts: [{ text: "Fast " }, { text: "Whisper", highlight: "whisper" }, { text: " · faster " }, { text: "Parakeet", highlight: "parakeet" }, { text: "" }] },
  { parts: [{ text: "Record · transcribe · copy · done" }] },
  { parts: [{ text: "One app · two engines · zero compromise" }] },
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
  const storeRef = useRef<Store | null>(null);
  const [backendInfo, setBackendInfo] = useState("Loading...");
  const [isInitialLoading, setIsInitialLoading] = useState(true);
  const [loadingMessage, setLoadingMessage] = useState("");
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  /** null = not yet loaded from store; true = show wizard (first run); false = show main app */
  const [showSetupWizard, setShowSetupWizard] = useState<boolean | null>(null);
  const [copyJustConfirmed, setCopyJustConfirmed] = useState(false);
  const [copyJustReset, setCopyJustReset] = useState(false);
  const copyConfirmTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const copyResetTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // --- Custom Hooks ---
  const { headerStatusMessage, headerStatusIsProcessing, setHeaderStatus } = useHeaderStatus();
  const {
    models, setModels, currentModel, setCurrentModel,
    parakeetModels, setParakeetModels, currentParakeetModel, setCurrentParakeetModel,
    refreshModels,
  } = useModels(setHeaderStatus);

  const {
    llmStatus, enableGrammarLM, setEnableGrammarLM, enableGrammarLMRef,
    enableSpellCheck, setEnableSpellCheck, enableSpellCheckRef, spellCheckStatus,
    transcriptionStyle, setTranscriptionStyle, transcriptionStyleRef,
    llmBackend, setLlmBackend,
  } = usePostProcessing(setHeaderStatus);

  const {
    activeEngine, setActiveEngine, activeEngineRef,
    loadedEngine, setLoadedEngine,
    isLoading, setIsLoading, isLoadingRef,
    loadingTargetEngine, transferLineFadingOut, setTransferLineFadingOut,
    handleModelChange, handleSwitchToWhisper, handleSwitchToParakeet,
  } = useEngineSwitch({
    models, parakeetModels, currentModel, currentParakeetModel,
    setCurrentModel, setCurrentParakeetModel, setBackendInfo,
    storeRef, setHeaderStatus, setTrayState,
  });

  const {
    isRecording, isRecordingRef, isProcessingTranscript, isCorrecting,
    liveTranscript, latestLatency,
    handleStartRecording, handleStopRecording,
  } = useRecording({
    activeEngineRef, models, parakeetModels, currentModel, currentParakeetModel,
    setCurrentModel, setLoadedEngine, enableGrammarLMRef, enableSpellCheckRef,
    transcriptionStyleRef, setHeaderStatus, setTrayState, setIsSettingsOpen,
  });

  // --- Copy reset animation ---
  useEffect(() => {
    if (!copyJustReset) return;
    if (copyResetTimeoutRef.current) clearTimeout(copyResetTimeoutRef.current);
    copyResetTimeoutRef.current = setTimeout(() => {
      setCopyJustReset(false);
      copyResetTimeoutRef.current = null;
    }, 520);
    return () => { if (copyResetTimeoutRef.current) clearTimeout(copyResetTimeoutRef.current); };
  }, [copyJustReset]);

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

  // --- Initial data load ---
  useEffect(() => {
    let cancelled = false;

    async function loadInitialData() {
      try {
        const backend = await invoke("get_backend_info");
        if (cancelled) return;
        setBackendInfo(backend as string);

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

        let savedEngine: "whisper" | "parakeet" | null = null;
        try {
          const loadedStore = await Store.load("settings.json");
          if (cancelled) return;
          storeRef.current = loadedStore;

          const setupComplete = await loadedStore.get<boolean>("setup_complete");
          if (!cancelled) setShowSetupWizard(setupComplete !== true);

          savedEngine = (await loadedStore.get<"whisper" | "parakeet">("active_engine")) || null;
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
        if (!cancelled) setIsInitialLoading(false);
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

  // --- File system watcher for models dir ---
  const refreshModelsRef = useRef(refreshModels);
  useEffect(() => { refreshModelsRef.current = refreshModels; });

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    const setup = async () => {
      const unsub = await listen("models-changed", () => {
        refreshModelsRef.current(false);
      });
      if (active) unlisten = unsub;
      else unsub();
    };
    setup();
    return () => { active = false; if (unlisten) unlisten(); };
  }, []);

  // --- Hotkey listeners ---
  const handleStartRecordingRef = useRef(handleStartRecording);
  const handleStopRecordingRef = useRef(handleStopRecording);
  useEffect(() => {
    handleStartRecordingRef.current = handleStartRecording;
    handleStopRecordingRef.current = handleStopRecording;
  });

  const startingRecordingRef = useRef(false);
  const pendingStopRef = useRef(false);
  const lastStartTime = useRef(0);

  useEffect(() => {
    let active = true;
    let unlistenStart: (() => void) | undefined;
    let unlistenStop: (() => void) | undefined;
    let unlistenChunk: (() => void) | undefined;

    const setup = async () => {
      const unsub1 = await listen("hotkey-start-recording", async () => {
        const now = Date.now();
        if (now - lastStartTime.current < 500) return;
        lastStartTime.current = now;

        if (!isRecordingRef.current && !startingRecordingRef.current) {
          startingRecordingRef.current = true;
          pendingStopRef.current = false;
          await handleStartRecordingRef.current();
          startingRecordingRef.current = false;
          if (pendingStopRef.current) {
            pendingStopRef.current = false;
            setTimeout(async () => { await handleStopRecordingRef.current(); }, 200);
          }
        }
      });

      const unsub2 = await listen("hotkey-stop-recording", async () => {
        if (startingRecordingRef.current) {
          pendingStopRef.current = true;
          return;
        }
        if (isRecordingRef.current) {
          await handleStopRecordingRef.current();
        }
      });

      const unsub3 = await listen("transcription-chunk", () => {
        // Live chunks not displayed; only final transcript shown
      });

      if (active) {
        unlistenStart = unsub1;
        unlistenStop = unsub2;
        unlistenChunk = unsub3;
      } else {
        unsub1(); unsub2(); unsub3();
      }
    };

    setup();
    return () => {
      active = false;
      if (unlistenStart) unlistenStart();
      if (unlistenStop) unlistenStop();
      if (unlistenChunk) unlistenChunk();
    };
  }, []);

  // --- Ticker ---
  const tickerContent = (
    <>
      {TICKER_PHRASES.flatMap((phrase, i) => [
        i > 0 ? <span key={`sep-${i}`} className="ticker-sep"> — </span> : null,
        <span key={i} className="header-ticker-phrase">
          {phrase.parts.map((p, j) => {
            if (!p.highlight) return p.text;
            const cls = p.highlight === "whisper" ? "ticker-whisper" : p.highlight === "parakeet" ? "ticker-parakeet" : "ticker-accent";
            return <span key={j} className={cls}>{p.text}</span>;
          })}
        </span>,
      ]).filter(Boolean)}
    </>
  );

  // --- Derived UI state ---
  const noModel = (activeEngine === "whisper" && models.length === 0) || (activeEngine === "parakeet" && parakeetModels.length === 0);
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

  const handleSetupComplete = useCallback((openSettings: boolean) => {
    storeRef.current?.set("setup_complete", true);
    storeRef.current?.save().catch(console.error);
    setShowSetupWizard(false);
    if (openSettings) setIsSettingsOpen(true);
  }, []);

  if (showSetupWizard === null) {
    return (
      <div className="app-loading" style={{ minHeight: "100vh", display: "flex", alignItems: "center", justifyContent: "center", background: "#0f172a", color: "#94a3b8" }}>
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
      <main className="container">
        <div>
          <div className="app-header">
            <h1 className="app-title">TAURSCRIBE</h1>
            <div className="header-status">
              {headerStatusMessage !== null ? (
                <span
                  className={`header-status-message ${headerStatusIsProcessing ? "header-status-message--processing" : ""}`}
                  key={headerStatusMessage}
                >
                  {headerStatusMessage}
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
            <button
              type="button"
              className="settings-btn"
              onClick={() => setIsSettingsOpen(true)}
              title="Settings"
              aria-label="Settings"
            >
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="12" cy="12" r="3" />
                <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
              </svg>
            </button>
          </div>
          <div className="hardware-bar">
            Hardware: <span>{backendInfo}</span>
          </div>
        </div>

        <div className="status-bar-container">
          {(isLoading || transferLineFadingOut) && (
            <div
              className={`status-bar-transfer-line ${transferLineFadingOut ? "status-bar-transfer-line--fade-out" : ""}`}
              aria-hidden="true"
            />
          )}
          <div
            className={`status-card whisper ${activeEngine === "whisper" ? "active" : ""}`}
            onClick={handleSwitchToWhisper}
            style={isLoading ? { pointerEvents: 'none' } : {}}
            role="button"
            tabIndex={0}
            onKeyDown={(e) => e.key === "Enter" && handleSwitchToWhisper()}
          >
            <div className="status-card-header">
              <span className="engine-badge">Whisper</span>
              <span
                className={`led-dot ${loadingTargetEngine === "whisper" ? "loading" :
                  loadedEngine === "whisper" ? "loaded" : "unloaded"
                  }`}
                aria-label={loadingTargetEngine === "whisper" ? "Loading" : loadedEngine === "whisper" ? "Loaded" : "Unloaded"}
              />
            </div>
            <div className="status-item">
              <span className="status-label">Model</span>
              <span className={`status-value ${models.length === 0 ? "error" : ""}`}>
                {models.length === 0 ? "Download required" : (currentModel ? beautifyModelName(models.find(m => m.id === currentModel)?.display_name || currentModel) : "None")}
              </span>
            </div>
          </div>

          <div
            className={`status-card parakeet ${activeEngine === "parakeet" ? "active" : ""}`}
            onClick={handleSwitchToParakeet}
            style={isLoading ? { pointerEvents: 'none' } : {}}
            role="button"
            tabIndex={0}
            onKeyDown={(e) => e.key === "Enter" && handleSwitchToParakeet()}
          >
            <div className="status-card-header">
              <span className="engine-badge">Parakeet</span>
              <span
                className={`led-dot ${loadingTargetEngine === "parakeet" ? "loading" :
                  loadedEngine === "parakeet" ? "loaded" : "unloaded"
                  }`}
                aria-label={loadingTargetEngine === "parakeet" ? "Loading" : loadedEngine === "parakeet" ? "Loaded" : "Unloaded"}
              />
            </div>
            <div className="status-item">
              <span className="status-label">Model</span>
              <span className={`status-value ${parakeetModels.length === 0 ? "error" : ""}`}>
                {parakeetModels.length === 0 ? "Download required" : (currentParakeetModel ? beautifyModelName(parakeetModels.find(m => m.id === currentParakeetModel)?.display_name || currentParakeetModel) : "None")}
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
                    {!isInitialLoading && models.length === 0 && <option value="">No models — open Settings to download</option>}
                    {models.map((model) => (
                      <option key={model.id} value={model.id}>
                        {beautifyModelName(model.display_name)} ({formatSize(model.size_mb)})
                      </option>
                    ))}
                  </select>
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
              )}
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

        <div className="output-area">
          {isRecording ? (
            <div className="live-transcript">
              <p className="listening-placeholder">Listening...</p>
            </div>
          ) : liveTranscript ? (
            <>
              <div className="final-transcript-header">
                {latestLatency !== null && (
                  <span className="latency-badge">{latestLatency} ms</span>
                )}
              </div>
              <pre>{liveTranscript}</pre>
              <div className="copy-container">
                <button
                  type="button"
                  onClick={async () => {
                    try {
                      await navigator.clipboard.writeText(liveTranscript);
                      if (copyConfirmTimeoutRef.current) clearTimeout(copyConfirmTimeoutRef.current);
                      setCopyJustConfirmed(true);
                      setHeaderStatus("Copied to clipboard");
                      copyConfirmTimeoutRef.current = setTimeout(() => {
                        setCopyJustConfirmed(false);
                        setCopyJustReset(true);
                        copyConfirmTimeoutRef.current = null;
                      }, 2000);
                    } catch (e) {
                      setHeaderStatus("Copy failed", 5000);
                    }
                  }}
                  className={`btn-copy ${copyJustConfirmed ? "btn-copy--confirmed" : ""} ${copyJustReset ? "btn-copy--resetting" : ""}`}
                  title={copyJustConfirmed ? "Copied!" : "Copy to clipboard"}
                  aria-label={copyJustConfirmed ? "Copied" : "Copy to clipboard"}
                >
                  {copyJustConfirmed ? (
                    <span className="btn-copy-inner btn-copy-inner--confirmed">
                      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                        <polyline points="20 6 9 17 4 12" />
                      </svg>
                      <span className="btn-copy-label">Copied</span>
                    </span>
                  ) : (
                    <span className={`btn-copy-inner ${copyJustReset ? "btn-copy-inner--animate-in" : ""}`}>
                      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                        <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
                        <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
                      </svg>
                    </span>
                  )}
                </button>
              </div>
            </>
          ) : (
            <div className="output-area-empty" aria-hidden="true" />
          )}
        </div>

        <SettingsModal
          isOpen={isSettingsOpen}
          onClose={() => setIsSettingsOpen(false)}
          onModelDownloaded={refreshModels}
          enableGrammarLM={enableGrammarLM}
          setEnableGrammarLM={setEnableGrammarLM}
          llmStatus={llmStatus}
          enableSpellCheck={enableSpellCheck}
          setEnableSpellCheck={setEnableSpellCheck}
          spellCheckStatus={spellCheckStatus}
          transcriptionStyle={transcriptionStyle}
          setTranscriptionStyle={setTranscriptionStyle}
          llmBackend={llmBackend}
          setLlmBackend={setLlmBackend}
        />
      </main>
    </>
  );
}

export default App;
