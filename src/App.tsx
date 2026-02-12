import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Store } from "@tauri-apps/plugin-store";
import { SettingsModal } from "./components/SettingsModal";
import { TitleBar } from "./components/TitleBar";
import "./components/TitleBar.css";
import "./App.css";

interface ModelInfo {
  id: string;
  display_name: string;
  file_name: string;
  size_mb: number;
}

interface ParakeetModelInfo {
  id: string;
  display_name: string;
  model_type: string;
  size_mb: number;
}

interface ParakeetStatus {
  loaded: boolean;
  model_id: string | null;
  model_type: string | null;
  backend: string;
}



type ASREngine = "whisper" | "parakeet";

function App() {
  const storeRef = useRef<Store | null>(null);
  const [liveTranscript, setLiveTranscript] = useState("");
  const [latestLatency, setLatestLatency] = useState<number | null>(null);
  const [isRecording, setIsRecording] = useState(false);
  const [backendInfo, setBackendInfo] = useState("Loading...");
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [currentModel, setCurrentModel] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isInitialLoading, setIsInitialLoading] = useState(true);
  const [loadingMessage, setLoadingMessage] = useState("");
  const [loadingTargetEngine, setLoadingTargetEngine] = useState<ASREngine | null>(null);
  const [transferLineFadingOut, setTransferLineFadingOut] = useState(false);
  const transferLineFadeRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Parakeet state
  const [parakeetModels, setParakeetModels] = useState<ParakeetModelInfo[]>([]);
  const [currentParakeetModel, setCurrentParakeetModel] = useState<string | null>(null);
  const [, setParakeetStatus] = useState<ParakeetStatus | null>(null);
  const [activeEngine, setActiveEngine] = useState<ASREngine>("whisper");

  // Tracks which engine is actually loaded in GPU/memory (only one at a time)
  const [loadedEngine, setLoadedEngine] = useState<ASREngine | null>(null);
  const [isCorrecting, setIsCorrecting] = useState(false);
  const [isProcessingTranscript, setIsProcessingTranscript] = useState(false);
  const [llmStatus, setLlmStatus] = useState("Not Loaded");
  const [enableGrammarLM, setEnableGrammarLM] = useState(false);

  // SymSpell spell check state
  const [enableSpellCheck, setEnableSpellCheck] = useState(false);
  const [spellCheckStatus, setSpellCheckStatus] = useState("Not Loaded");
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [copyJustConfirmed, setCopyJustConfirmed] = useState(false);
  const [copyJustReset, setCopyJustReset] = useState(false);
  const copyConfirmTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const copyResetTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!copyJustReset) return;
    if (copyResetTimeoutRef.current) clearTimeout(copyResetTimeoutRef.current);
    copyResetTimeoutRef.current = setTimeout(() => {
      setCopyJustReset(false);
      copyResetTimeoutRef.current = null;
    }, 520);
    return () => {
      if (copyResetTimeoutRef.current) clearTimeout(copyResetTimeoutRef.current);
    };
  }, [copyJustReset]);

  useEffect(() => {
    if (!transferLineFadingOut) return;
    if (transferLineFadeRef.current) clearTimeout(transferLineFadeRef.current);
    transferLineFadeRef.current = setTimeout(() => {
      setTransferLineFadingOut(false);
      transferLineFadeRef.current = null;
    }, 450);
    return () => {
      if (transferLineFadeRef.current) clearTimeout(transferLineFadeRef.current);
    };
  }, [transferLineFadingOut]);

  // Header status: shows message temporarily, then reverts to scrolling idle text
  const [headerStatusMessage, setHeaderStatusMessage] = useState<string | null>(null);
  const [headerStatusIsProcessing, setHeaderStatusIsProcessing] = useState(false);
  const headerStatusTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

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

  const setHeaderStatus = (message: string, durationMs = 3200, isProcessing = false) => {
    if (headerStatusTimeoutRef.current) clearTimeout(headerStatusTimeoutRef.current);
    setHeaderStatusMessage(message);
    setHeaderStatusIsProcessing(isProcessing);
    headerStatusTimeoutRef.current = setTimeout(() => {
      setHeaderStatusMessage(null);
      setHeaderStatusIsProcessing(false);
      headerStatusTimeoutRef.current = null;
    }, durationMs);
  };


  // Ref to track recording state for hotkey handlers (avoids stale closure)
  const isRecordingRef = useRef(false);
  // Ref to prevent double model-switch: state updates are async, so a second click can see isLoading still false
  const isLoadingRef = useRef(false);

  // Load backend info and models on mount
  // Uses cancelled flag to prevent duplicate work from React StrictMode double-mount
  useEffect(() => {
    let cancelled = false;

    async function loadInitialData() {
      try {
        // Load backend info
        const backend = await invoke("get_backend_info");
        if (cancelled) return;
        setBackendInfo(backend as string);

        // Load available models
        const modelList = await invoke("list_models") as ModelInfo[];
        if (cancelled) return;
        setModels(modelList);

        // Load current model
        const current = await invoke("get_current_model") as string | null;
        if (cancelled) return;
        setCurrentModel(current);

        // Whisper auto-loads in Rust on startup, so if a model is set it's loaded
        if (current) setLoadedEngine("whisper");

        // Load Parakeet data
        const pModels = await invoke("list_parakeet_models") as ParakeetModelInfo[];
        if (cancelled) return;
        setParakeetModels(pModels);

        const pStatus = await invoke("get_parakeet_status") as ParakeetStatus;
        if (cancelled) return;
        setParakeetStatus(pStatus);
        setCurrentParakeetModel(pStatus.model_id);

        // Load settings from store
        let savedEngine: ASREngine | null = null;
        try {
          const loadedStore = await Store.load("settings.json");
          if (cancelled) return;
          storeRef.current = loadedStore;

          savedEngine = (await loadedStore.get<ASREngine>("active_engine")) || null;
          if (savedEngine) setActiveEngine(savedEngine);

          const savedWhisper = await loadedStore.get<string>("whisper_model");
          if (savedWhisper && modelList.find(m => m.id === savedWhisper)) {
            // Optional: Auto-switch logic could go here
          }

          const savedParakeet = await loadedStore.get<string>("parakeet_model");

          // Auto-load Parakeet model if it was the saved engine
          if (savedEngine === "parakeet" && pModels.length > 0) {
            const targetModel = (savedParakeet && pModels.find(m => m.id === savedParakeet))
              ? savedParakeet
              : pModels[0].id;

            isLoadingRef.current = true;
            setIsLoading(true);
            setLoadingMessage(`Loading Parakeet (${targetModel})...`);
            console.log("[LOADING] Loading Parakeet (" + targetModel + ") — called from loadInitialData (auto-load on startup)");
            try {
              if (cancelled) return;
              await invoke("init_parakeet", { modelId: targetModel });
              if (cancelled) return;
              setCurrentParakeetModel(targetModel);
              setLoadedEngine("parakeet");
              const pStatusUpdated = await invoke("get_parakeet_status") as ParakeetStatus;
              if (cancelled) return;
              setParakeetStatus(pStatusUpdated);
              setHeaderStatus("Parakeet model loaded");
            } catch (e) {
              if (cancelled) return;
              console.error("Failed to auto-load Parakeet:", e);
              setHeaderStatus(`Failed to auto-load Parakeet: ${e}`, 5000);
            } finally {
              if (!cancelled) {
                console.log("[LOADING] Set loading FALSE — loadInitialData (Parakeet auto-load)");
                isLoadingRef.current = false;
                setIsLoading(false);
                setLoadingMessage("");
              }
            }
          }
        } catch (storeErr) {
          console.warn("Store load failed:", storeErr);
        }

        // Default to Parakeet if a model is loaded and Whisper isn't (rare but possible)
        if (!cancelled && pStatus.loaded && !current && !savedEngine) {
          setActiveEngine("parakeet");
        }
      } catch (e) {
        if (cancelled) return;
        console.error("Failed to load initial data:", e);
        setBackendInfo("Unknown");
        setHeaderStatus(`Error loading models: ${e}`, 5000);
      } finally {
        if (!cancelled) {
          setIsInitialLoading(false);
        }
      }
    }
    loadInitialData();

    return () => { cancelled = true; };
  }, []);

  // Refresh model lists (called after download completes or file system changes)
  // Uses functional state updates to avoid stale closure issues
  const refreshModels = async (showToast = true) => {
    try {
      console.log("[INFO] Refreshing model lists...");
      const modelList = await invoke("list_models") as ModelInfo[];
      setModels(modelList);

      const pModels = await invoke("list_parakeet_models") as ParakeetModelInfo[];
      setParakeetModels(pModels);

      // If we now have models but didn't before, auto-select the first one
      // Uses functional updates to read current state (avoids stale closure)
      if (modelList.length > 0) {
        setCurrentModel(prev => prev ?? modelList[0].id);
      }
      if (pModels.length > 0) {
        setCurrentParakeetModel(prev => prev ?? pModels[0].id);
      }

      if (showToast) {
        setHeaderStatus("Model list refreshed!");
      }
    } catch (e) {
      console.error("Failed to refresh models:", e);
    }
  };

  // Ref to always access the latest refreshModels (avoids stale closure in listeners)
  const refreshModelsRef = useRef(refreshModels);
  useEffect(() => {
    refreshModelsRef.current = refreshModels;
  });

  // Auto-init/Unload LLM when enabled/disabled
  useEffect(() => {
    if (enableGrammarLM && llmStatus === "Not Loaded") {
      setHeaderStatus("Auto-loading Qwen LLM...", 60_000);
      setLlmStatus("Loading...");
      invoke("init_llm").then((res) => {
        setLlmStatus("Loaded");
        setHeaderStatus(res as string);
      }).catch(() => setLlmStatus("Error"));
    } else if (!enableGrammarLM && llmStatus === "Loaded") {
      setLlmStatus("Loading..."); // Unloading uses loading status temporarily or I could add "Unloading..."
      invoke("unload_llm").then(() => {
        setLlmStatus("Not Loaded");
        setHeaderStatus("Qwen LLM unloaded");
      }).catch((e) => {
        setLlmStatus("Error");
        setHeaderStatus(`Failed to unload: ${e}`, 5000);
      });
    }
  }, [enableGrammarLM, llmStatus]);

  // Auto-init/Unload SpellCheck when enabled/disabled
  useEffect(() => {
    if (enableSpellCheck && spellCheckStatus === "Not Loaded") {
      setHeaderStatus("Loading SymSpell dictionary...", 60_000);
      setSpellCheckStatus("Loading...");
      invoke("init_spellcheck").then((res) => {
        setSpellCheckStatus("Loaded");
        setHeaderStatus(res as string);
      }).catch((err) => {
        setSpellCheckStatus("Error");
        setHeaderStatus("SymSpell failed: " + err, 5000);
      });
    } else if (!enableSpellCheck && spellCheckStatus === "Loaded") {
      setSpellCheckStatus("Loading...");
      invoke("unload_spellcheck").then(() => {
        setSpellCheckStatus("Not Loaded");
        setHeaderStatus("SymSpell unloaded");
      }).catch((e) => {
        setSpellCheckStatus("Error");
        setHeaderStatus(`Failed to unload: ${e}`, 5000);
      });
    }
  }, [enableSpellCheck, spellCheckStatus]);

  // Listen for file system changes to models directory
  // Uses refreshModelsRef to always call the latest version
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    const setupListener = async () => {
      const unsub = await listen("models-changed", () => {
        console.log("[INFO] Models directory changed, refreshing...");
        refreshModelsRef.current(false); // Refresh silently via ref
      });
      // Only assign if effect is still active (guards against cleanup race)
      if (active) {
        unlisten = unsub;
      } else {
        unsub(); // Effect was cleaned up while we were awaiting, unsubscribe immediately
      }
    };

    setupListener();

    return () => {
      active = false;
      if (unlisten) unlisten();
    };
  }, []);

  // Sync active engine with backend & Save
  useEffect(() => {
    if (!isInitialLoading) {
      invoke("set_active_engine", { engine: activeEngine }).catch(console.error);
      if (storeRef.current) {
        storeRef.current.set("active_engine", activeEngine).then(() => storeRef.current?.save());
      }
    }
  }, [activeEngine, isInitialLoading]);

  // Refs for hotkey state management (refs persist across async calls)
  const startingRecordingRef = useRef(false);
  const pendingStopRef = useRef(false);
  const lastStartTime = useRef(0);  // Debounce start events
  const recordingStartTimeRef = useRef(0);  // When recording actually started (for minimum duration check)
  const enableGrammarLMRef = useRef(enableGrammarLM);

  const MIN_RECORDING_MS = 1500;  // Shorter than this → "Recording too short" (no transcript)
  const enableSpellCheckRef = useRef(enableSpellCheck);

  // Sync refs
  useEffect(() => {
    enableGrammarLMRef.current = enableGrammarLM;
  }, [enableGrammarLM]);

  useEffect(() => {
    enableSpellCheckRef.current = enableSpellCheck;
  }, [enableSpellCheck]);

  const activeEngineRef = useRef(activeEngine);
  useEffect(() => {
    activeEngineRef.current = activeEngine;
  }, [activeEngine]);

  // --- Unified Handlers ---
  const handleStartRecording = async () => {
    const currentEngine = activeEngineRef.current;
    // Safety check for missing models
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
      // Check if Parakeet engine is actually initialized (not just files on disk)
      try {
        const pStatus = await invoke("get_parakeet_status") as ParakeetStatus;
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
      const res = await invoke("start_recording");
      setHeaderStatus(res as string);
      recordingStartTimeRef.current = Date.now();

      setIsRecording(true);
      isRecordingRef.current = true;
    } catch (e) {
      console.error("Start recording failed:", e);
      setHeaderStatus("Error: " + e, 5000);
      await setTrayState("ready");
      // Ensure state is reset if start failed
      setIsRecording(false);
      isRecordingRef.current = false;
    }
  };

  const handleStopRecording = async () => {
    const currentEngine = activeEngineRef.current;
    const processingStartMs = Date.now();
    console.log("[STOP] handleStopRecording called. GrammarLM:", enableGrammarLMRef.current, "SpellCheck:", enableSpellCheckRef.current);
    setIsRecording(false);
    isRecordingRef.current = false;
    setIsProcessingTranscript(true);
    try {
      await setTrayState("processing");
      if (currentEngine === "whisper") setHeaderStatus("Processing transcription...", 60_000, true);

      let finalTrans = await invoke("stop_recording") as string;

      const recordingDurationMs = Date.now() - recordingStartTimeRef.current;
      if (recordingDurationMs < MIN_RECORDING_MS) {
        setHeaderStatus("Recording too short — try at least 1.5 seconds", 5000);
        setLiveTranscript("");
        setIsProcessingTranscript(false);
        await setTrayState("ready");
        return;
      }

      // Step 1: SymSpell spell check (fast, runs first if enabled)
      if (enableSpellCheckRef.current) {
        setIsCorrecting(true);
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

      // Step 2: Grammar LM correction (slower, runs after spell check if enabled)
      if (enableGrammarLMRef.current) {
        setHeaderStatus("Correcting grammar...", 60_000, true);
        try {
          finalTrans = await invoke("correct_text", { text: finalTrans });
          setHeaderStatus("Transcribed & Corrected!");
        } catch (e) {
          setHeaderStatus("Grammar correction failed: " + e, 5000);
        }
      }

      setIsCorrecting(false);

      const totalMs = Date.now() - processingStartMs;
      setLatestLatency(totalMs);
      setLiveTranscript(finalTrans);

      // Type out the final transcript once (after optional spell/grammar steps)
      await invoke("type_text", { text: finalTrans });

      setIsProcessingTranscript(false);
      await setTrayState("ready");
    } catch (e) {
      console.error("Stop recording failed:", e);
      // Ignore "Not recording" errors which can happen in race conditions
      const errStr = String(e);
      if (!errStr.includes("Not recording")) {
        setHeaderStatus("Error: " + e, 5000);
      }
      isRecordingRef.current = false;
      setIsCorrecting(false);
      setIsProcessingTranscript(false);
      await setTrayState("ready");
    }
  };

  // Keep handler refs up-to-date so hotkey listeners always call the latest version
  // This avoids stale closures where handlers capture outdated models/state
  const handleStartRecordingRef = useRef(handleStartRecording);
  const handleStopRecordingRef = useRef(handleStopRecording);
  useEffect(() => {
    handleStartRecordingRef.current = handleStartRecording;
    handleStopRecordingRef.current = handleStopRecording;
  });

  // Listen for hotkey events from Rust backend
  // Uses handler refs so callbacks always invoke the latest version of the handlers
  useEffect(() => {
    let active = true;
    let unlistenStart: (() => void) | undefined;
    let unlistenStop: (() => void) | undefined;
    let unlistenChunk: (() => void) | undefined;

    const setupListeners = async () => {
      // Listen for hotkey start recording
      const unsub1 = await listen("hotkey-start-recording", async () => {
        // Debounce: ignore if another start happened within 500ms
        const now = Date.now();
        if (now - lastStartTime.current < 500) {
          console.log("[HOTKEY] Debouncing duplicate start event");
          return;
        }
        lastStartTime.current = now;

        if (!isRecordingRef.current && !startingRecordingRef.current) {
          console.log("[HOTKEY] Starting recording via Ctrl+Win");
          startingRecordingRef.current = true;
          pendingStopRef.current = false;

          await handleStartRecordingRef.current();
          startingRecordingRef.current = false;

          // If stop was requested while we were starting, handle it now
          if (pendingStopRef.current) {
            console.log("[HOTKEY] Processing pending stop request");
            pendingStopRef.current = false;
            // Small delay to ensure recording has time to capture something
            setTimeout(async () => {
              await handleStopRecordingRef.current();
            }, 200);
          }
        }
      });

      // Listen for hotkey stop recording
      const unsub2 = await listen("hotkey-stop-recording", async () => {
        // If we're still starting, queue the stop
        if (startingRecordingRef.current) {
          console.log("[HOTKEY] Stop requested while starting - queuing");
          pendingStopRef.current = true;
          return;
        }

        if (isRecordingRef.current) {
          console.log("[HOTKEY] Stopping recording via Ctrl+Win release");
          await handleStopRecordingRef.current();
        } else {
          // Silently ignore - stop was called but nothing was recording
          console.log("[HOTKEY] Stop requested but not recording - ignoring");
        }
      });

      const unsub3 = await listen("transcription-chunk", () => {
        // We no longer show live chunks; only the final transcript is displayed with its latency
      });

      // Only assign if effect is still active (guards against cleanup race)
      if (active) {
        unlistenStart = unsub1;
        unlistenStop = unsub2;
        unlistenChunk = unsub3;
      } else {
        // Effect was cleaned up while we were awaiting, unsubscribe immediately
        unsub1();
        unsub2();
        unsub3();
      }
    };

    setupListeners();

    return () => {
      active = false;
      console.log("[HOTKEY] Cleaning up listeners");
      if (unlistenStart) unlistenStart();
      if (unlistenStop) unlistenStop();
      if (unlistenChunk) unlistenChunk();
    };
  }, []); // Empty deps: set up once, handlers called through refs

  const handleModelChange = async (modelId: string) => {
    if (modelId === currentModel && activeEngine === "whisper") return; // Already loaded and active
    if (isLoading || isLoadingRef.current) {
      console.log("[LOADING] Skipping handleModelChange — already loading (ref or state)");
      return;
    }

    isLoadingRef.current = true;
    setIsLoading(true);
    setLoadedEngine(null);
    setLoadingTargetEngine("whisper");
    const displayName = models.find(m => m.id === modelId)?.display_name || modelId;
    const msg = `Loading ${displayName}...`;
    setLoadingMessage(msg);
    setHeaderStatus(msg, 60_000);
    console.log("[LOADING] Loading Whisper model " + modelId + " — called from handleModelChange");

    try {
      if (activeEngine !== "whisper") {
        await setTrayState("processing");
        await invoke("switch_model", { modelId });
        setActiveEngine("whisper");
        setHeaderStatus(`Switched to Whisper (${modelId})`);
      } else {
        await setTrayState("processing");
        await invoke("switch_model", { modelId });
        setHeaderStatus(`Switched model to ${modelId}`);
      }

      setCurrentModel(modelId);
      setLoadedEngine("whisper");

      if (storeRef.current) {
        await storeRef.current.set("whisper_model", modelId);
        await storeRef.current.set("active_engine", "whisper");
        await storeRef.current.save();
      }

      const backend = await invoke("get_backend_info");
      setBackendInfo(backend as string);
    } catch (e) {
      setHeaderStatus(`Error switching model: ${e}`, 5000);
    } finally {
      console.log("[LOADING] Set loading FALSE — handleModelChange (finally)");
      isLoadingRef.current = false;
      setIsLoading(false);
      setLoadingMessage("");
      setLoadingTargetEngine(null);
      setTransferLineFadingOut(true);
      await setTrayState("ready");
    }
  };

  const handleSwitchToWhisper = async () => {
    if (activeEngine === "whisper") return;
    if (!currentModel && models.length > 0) {
      await handleModelChange(models[0].id);
    } else if (currentModel) {
      await handleModelChange(currentModel);
    } else {
      setActiveEngine("whisper"); // Just switch UI if no models
    }
  };

  const handleSwitchToParakeet = async () => {
    if (parakeetModels.length === 0) {
      setActiveEngine("parakeet"); // Just switch UI to show error state
      return;
    }
    if (isLoading || isLoadingRef.current) {
      console.log("[LOADING] Skipping handleSwitchToParakeet — already loading (ref or state)");
      return;
    }

    // Allow retry: check if already active AND model is loaded
    if (activeEngine === "parakeet") {
      try {
        const pStatus = await invoke("get_parakeet_status") as ParakeetStatus;
        if (pStatus.loaded) return; // Already active and loaded, nothing to do
      } catch {
        // If status check fails, proceed with loading attempt
      }
    }

    const targetModel = currentParakeetModel || parakeetModels[0].id;

    isLoadingRef.current = true;
    setIsLoading(true);
    setLoadedEngine(null);
    setLoadingTargetEngine("parakeet");
    const msg = `Loading Parakeet (${targetModel})...`;
    setLoadingMessage(msg);
    setHeaderStatus(msg, 60_000);
    console.log("[LOADING] Loading Parakeet (" + targetModel + ") — called from handleSwitchToParakeet");

    try {
      await setTrayState("processing");
      await invoke("init_parakeet", { modelId: targetModel });

      setCurrentParakeetModel(targetModel);
      setActiveEngine("parakeet");
      setLoadedEngine("parakeet");

      if (storeRef.current) {
        await storeRef.current.set("parakeet_model", targetModel);
        await storeRef.current.set("active_engine", "parakeet");
        await storeRef.current.save();
      }

      setHeaderStatus(`Switched to Parakeet`);

      const backend = await invoke("get_backend_info");
      setBackendInfo(backend as string);
    } catch (e) {
      setHeaderStatus(`Error switching to Parakeet: ${e}`, 5000);
    } finally {
      console.log("[LOADING] Set loading FALSE — handleSwitchToParakeet (finally)");
      isLoadingRef.current = false;
      setIsLoading(false);
      setLoadingMessage("");
      setLoadingTargetEngine(null);
      setTransferLineFadingOut(true);
      await setTrayState("ready");
    }
  };

  const formatSize = (sizeMb: number): string => {
    if (sizeMb >= 1024) {
      return `${(sizeMb / 1024).toFixed(1)} GB`;
    }
    return `${Math.round(sizeMb)} MB`;
  };

  // Helper to beautify model names
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

    // Capitalize words
    return name.replace(/\b\w/g, l => l.toUpperCase());
  };

  // Helper to update tray icon state
  const setTrayState = async (newState: "ready" | "recording" | "processing") => {
    try {
      await invoke("set_tray_state", { newState });
    } catch (e) {
      console.error("Failed to set tray state:", e);
    }
  };

  const noModel = (activeEngine === "whisper" && models.length === 0) || (activeEngine === "parakeet" && parakeetModels.length === 0);
  const recordBtnBusy = isLoading || isCorrecting || isProcessingTranscript; /* model loading, post-processing, or transcribing */
  const recordBtnClass =
    noModel ? "record-btn disabled" :
      isRecording ? "record-btn recording" :
        recordBtnBusy ? "record-btn processing" :
          "record-btn idle";
  const recordBtnLabel =
    noModel ? "NO MODEL" :
      isRecording ? "STOP" :
        recordBtnBusy ? "..." : "REC";
  const recordBtnDisabled = (isLoading && !isRecording) || isCorrecting || isProcessingTranscript; /* allow Stop while recording only */
  const onRecordClick = () => {
    if (noModel) {
      setIsSettingsOpen(true);
      return;
    }
    if (isRecording) handleStopRecording();
    else handleStartRecording();
  };

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
        />
      </main >
    </>
  );
}

export default App;
