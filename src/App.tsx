import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Store } from "@tauri-apps/plugin-store";
import { Toaster, toast } from "sonner";
import "./App.css";

interface ModelInfo {
  id: string;
  display_name: string;
  file_name: string;
  size_mb: number;
}

interface SampleFile {
  name: string;
  path: string;
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

interface LiveTranscriptionPayload {
  text: string;
  processing_time_ms: number;
  method: string;
}

type ASREngine = "whisper" | "parakeet";

function App() {
  const storeRef = useRef<Store | null>(null);
  const [greetMsg, setGreetMsg] = useState("");
  const [liveTranscript, setLiveTranscript] = useState("");
  const [latestLatency, setLatestLatency] = useState<number | null>(null);
  const [isRecording, setIsRecording] = useState(false);
  const [backendInfo, setBackendInfo] = useState("Loading...");
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [currentModel, setCurrentModel] = useState<string | null>(null);
  const [sampleFiles, setSampleFiles] = useState<SampleFile[]>([]);
  const [selectedSample, setSelectedSample] = useState<string>("");
  const [isLoading, setIsLoading] = useState(false);
  const [isInitialLoading, setIsInitialLoading] = useState(true);
  const [loadingMessage, setLoadingMessage] = useState("");

  // Parakeet state
  const [parakeetModels, setParakeetModels] = useState<ParakeetModelInfo[]>([]);
  const [currentParakeetModel, setCurrentParakeetModel] = useState<string | null>(null);
  const [, setParakeetStatus] = useState<ParakeetStatus | null>(null);
  const [activeEngine, setActiveEngine] = useState<ASREngine>("whisper");

  // Ref to track recording state for hotkey handlers (avoids stale closure)
  const isRecordingRef = useRef(false);

  // Load backend info and models on mount
  useEffect(() => {
    async function loadInitialData() {
      try {
        // Load backend info
        const backend = await invoke("get_backend_info");
        setBackendInfo(backend as string);

        // Load available models
        const modelList = await invoke("list_models") as ModelInfo[];
        setModels(modelList);

        // Load current model
        const current = await invoke("get_current_model") as string | null;
        setCurrentModel(current);

        // Load sample files
        const samples = await invoke("list_sample_files");
        const sampleList = samples as SampleFile[];
        setSampleFiles(sampleList);
        if (sampleList.length > 0) {
          const defaultSample = sampleList.find(s => s.name.includes("otherjack")) || sampleList[0];
          setSelectedSample(defaultSample.path);
        }

        // Load Parakeet data
        const pModels = await invoke("list_parakeet_models") as ParakeetModelInfo[];
        setParakeetModels(pModels);

        const pStatus = await invoke("get_parakeet_status") as ParakeetStatus;
        setParakeetStatus(pStatus);
        setCurrentParakeetModel(pStatus.model_id);

        // Load Settings from Store
        // Load Settings from Store
        let savedEngine: ASREngine | null = null;
        try {
          const loadedStore = await Store.load("settings.json");
          storeRef.current = loadedStore;

          savedEngine = (await loadedStore.get<ASREngine>("active_engine")) || null;
          if (savedEngine) setActiveEngine(savedEngine);

          const savedWhisper = await loadedStore.get<string>("whisper_model");
          if (savedWhisper && modelList.find(m => m.id === savedWhisper)) {
            // Optional: Auto-switch logic could go here
          }

          const savedParakeet = await loadedStore.get<string>("parakeet_model");
          if (savedParakeet && pModels.find(m => m.id === savedParakeet)) {
            // Optional: Auto-switch logic
          }
        } catch (storeErr) {
          console.warn("Store load failed:", storeErr);
        }

        // Default to Parakeet if a model is loaded and Whisper isn't (rare but possible)
        if (pStatus.loaded && !current && !savedEngine) {
          setActiveEngine("parakeet");
        }
      } catch (e) {
        console.error("Failed to load initial data:", e);
        setBackendInfo("Unknown");
        toast.error(`Error loading models: ${e}`);
      } finally {
        setIsInitialLoading(false);
      }
    }
    loadInitialData();
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
  const listenersSetupRef = useRef(false);  // Prevent duplicate listeners from HMR
  const lastStartTime = useRef(0);  // Debounce start events

  // Listen for hotkey events from Rust backend
  useEffect(() => {
    // Prevent duplicate listeners (HMR can cause this)
    if (listenersSetupRef.current) {
      console.log("[HOTKEY] Listeners already setup, skipping");
      return;
    }
    listenersSetupRef.current = true;

    let unlistenStart: (() => void) | undefined;
    let unlistenStop: (() => void) | undefined;
    let unlistenChunk: (() => void) | undefined;

    const setupListeners = async () => {
      // Listen for hotkey start recording
      unlistenStart = await listen("hotkey-start-recording", async () => {
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

          try {
            await setTrayState("recording");
            const res = await invoke("start_recording");
            toast.success(res as string);
            setIsRecording(true);
            isRecordingRef.current = true;

            // If stop was requested while we were starting, handle it now
            if (pendingStopRef.current) {
              console.log("[HOTKEY] Processing pending stop request");
              pendingStopRef.current = false;
              // Small delay to ensure recording has time to capture something
              setTimeout(async () => {
                try {
                  await setTrayState("processing");
                  toast.info("Processing transcription...");
                  const stopRes = await invoke("stop_recording");
                  // Check if it's the specific Parakeet response
                  if ((stopRes as string).startsWith("[FINAL_TRANSCRIPT]")) {
                    // Already printed? No, invoke returns just the string.
                  }
                  toast.success("Recording saved.");
                  setIsRecording(false);
                  isRecordingRef.current = false;
                  await setTrayState("ready");
                } catch (e) {
                  console.error("Pending stop failed:", e);
                  const errStr = String(e);
                  if (!errStr.includes("Not recording")) {
                    toast.error("Error: " + e);
                  }
                  await setTrayState("ready");
                }
              }, 200);
            }
          } catch (e) {
            console.error("Hotkey start recording failed:", e);
            toast.error("Error: " + e);
            await setTrayState("ready");
          } finally {
            startingRecordingRef.current = false;
          }
        }
      });

      // Listen for hotkey stop recording
      unlistenStop = await listen("hotkey-stop-recording", async () => {
        // If we're still starting, queue the stop
        if (startingRecordingRef.current) {
          console.log("[HOTKEY] Stop requested while starting - queuing");
          pendingStopRef.current = true;
          return;
        }

        if (isRecordingRef.current) {
          console.log("[HOTKEY] Stopping recording via Ctrl+Win release");
          try {
            await setTrayState("processing");
            if (activeEngine === "whisper") toast.loading("Processing transcription..."); // Only show loading for Whisper
            await invoke("stop_recording");
            toast.dismiss(); // Dismiss loading
            // toast.success(res as string); // Don't toast the transcript, just "Saved"

            setIsRecording(false);
            isRecordingRef.current = false;
            await setTrayState("ready");
          } catch (e) {
            console.error("Hotkey stop recording failed:", e);
            // Ignore "Not recording" errors silently - they happen during race conditions
            const errStr = String(e);
            if (!errStr.includes("Not recording")) {
              toast.error("Error: " + e);
            }
            setIsRecording(false);
            isRecordingRef.current = false;
            await setTrayState("ready");
          }
        } else {
          // Silently ignore - stop was called but nothing was recording
          console.log("[HOTKEY] Stop requested but not recording - ignoring");
        }
      });

      unlistenChunk = await listen("transcription-chunk", (event) => {
        const payload = event.payload as LiveTranscriptionPayload;
        setLiveTranscript((prev) => prev + (prev ? " " : "") + payload.text);
        setLatestLatency(payload.processing_time_ms);
      });
    };

    setupListeners();

    return () => {
      console.log("[HOTKEY] Cleaning up listeners");
      if (unlistenStart) unlistenStart();
      if (unlistenStop) unlistenStop();
      if (unlistenChunk) unlistenChunk();
      listenersSetupRef.current = false;  // Allow re-setup after HMR
    };
  }, [activeEngine]); // Re-bind listeners if engine changes? No, unsafe. Listeners are generic.

  const handleModelChange = async (modelId: string) => {
    if (modelId === currentModel) return;

    setIsLoading(true);
    setLoadingMessage(`Loading ${models.find(m => m.id === modelId)?.display_name || modelId}...`);
    // toast.dismiss();

    try {
      await setTrayState("processing");
      const result = await invoke("switch_model", { modelId });
      setCurrentModel(modelId);
      setActiveEngine("whisper");
      if (storeRef.current) {
        await storeRef.current.set("whisper_model", modelId);
        await storeRef.current.set("active_engine", "whisper");
        await storeRef.current.save();
      }

      toast.success(`✅ ${result}`);

      // Refresh backend info (in case GPU backend changed)
      const backend = await invoke("get_backend_info");
      setBackendInfo(backend as string);
    } catch (e) {
      toast.error(`❌ Error switching model: ${e}`);
    } finally {
      setIsLoading(false);
      setLoadingMessage("");
      await setTrayState("ready");
    }
  };

  const handleParakeetModelChange = async (modelId: string) => {
    if (modelId === currentParakeetModel) return;

    setIsLoading(true);
    setLoadingMessage(`Loading Parakeet ${parakeetModels.find(m => m.id === modelId)?.display_name || modelId}...`);

    try {
      await setTrayState("processing");
      const result = await invoke("init_parakeet", { modelId });
      setCurrentParakeetModel(modelId);
      setActiveEngine("parakeet");
      if (storeRef.current) {
        await storeRef.current.set("parakeet_model", modelId);
        await storeRef.current.set("active_engine", "parakeet");
        await storeRef.current.save();
      }

      toast.success(`✅ ${result}`);

      const pStatus = await invoke("get_parakeet_status");
      setParakeetStatus(pStatus as ParakeetStatus);
    } catch (e) {
      toast.error(`❌ Error switching Parakeet model: ${e}`);
    } finally {
      setIsLoading(false);
      setLoadingMessage("");
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

  return (
    <main className="container">
      <Toaster position="top-center" richColors theme="dark" />
      <h1>🎙️ Taurscribe</h1>

      {/* Status Bar & Engine Selection */}
      <div className="status-bar-container">
        <div className={`status-card whisper ${activeEngine === "whisper" ? "active" : ""}`} onClick={() => setActiveEngine("whisper")}>
          <div className="engine-badge whisper">Whisper</div>
          <div className="status-item">
            <span className="status-label">Model:</span>
            <span className="status-value model">
              {currentModel ? beautifyModelName(models.find(m => m.id === currentModel)?.display_name || currentModel) : "None"}
            </span>
          </div>
        </div>

        <div className={`status-card parakeet ${activeEngine === "parakeet" ? "active" : ""}`} onClick={() => setActiveEngine("parakeet")}>
          <div className="engine-badge parakeet">Parakeet</div>
          <div className="status-item">
            <span className="status-label">Model:</span>
            <span className="status-value model">
              {currentParakeetModel ? beautifyModelName(parakeetModels.find(m => m.id === currentParakeetModel)?.display_name || currentParakeetModel) : "None"}
            </span>
          </div>
        </div>
      </div>

      {/* Backend Info Badge */}
      <div style={{ textAlign: 'center', fontSize: '0.8rem', color: 'var(--text-muted)', marginTop: '-10px' }}>
        Hardware Acceleration: <span style={{ color: 'var(--accent-secondary)', fontWeight: 600 }}>{backendInfo}</span>
      </div>

      {/* Model Selection Dropdown */}
      <div className="model-section">
        {activeEngine === "whisper" ? (
          <>
            <label htmlFor="model-select" className="model-label">
              🧠 Active Model
            </label>
            <select
              id="model-select"
              className="model-select"
              value={currentModel || ""}
              onChange={(e) => handleModelChange(e.target.value)}
              disabled={isRecording || isLoading || isInitialLoading}
            >
              {isInitialLoading && <option value="">Loading models...</option>}
              {!isInitialLoading && models.length === 0 && <option value="">No models found</option>}
              {models.map((model) => (
                <option key={model.id} value={model.id}>
                  {beautifyModelName(model.display_name)} ({formatSize(model.size_mb)})
                </option>
              ))}
            </select>
          </>
        ) : (
          <>
            <label htmlFor="parakeet-model-select" className="model-label">
              ⚡ Active Model
            </label>
            <select
              id="parakeet-model-select"
              className="model-select"
              value={currentParakeetModel || ""}
              onChange={(e) => handleParakeetModelChange(e.target.value)}
              disabled={isRecording || isLoading || isInitialLoading}
            >
              {isInitialLoading && <option value="">Loading models...</option>}
              {!isInitialLoading && parakeetModels.length === 0 && <option value="">No models found</option>}
              {parakeetModels.map((model) => (
                <option key={model.id} value={model.id}>
                  {beautifyModelName(model.display_name)} ({formatSize(model.size_mb)})
                </option>
              ))}
            </select>
          </>
        )}
      </div>

      {/* Loading overlay */}
      {isLoading && (
        <div className="loading-overlay">
          <div className="loading-spinner"></div>
          <span className="loading-text">{loadingMessage}</span>
        </div>
      )}

      {/* Recording Controls */}
      <div className="controls">
        <button
          onClick={async () => {
            try {
              await setTrayState("recording");
              setLiveTranscript("");
              setLatestLatency(null);
              const res = await invoke("start_recording");
              toast.success(res as string);
              setIsRecording(true);
            } catch (e) {
              await setTrayState("ready");
              toast.error("Error: " + e);
            }
          }}
          disabled={isRecording || isLoading}
          className="btn btn-start"
        >
          ⏺️ Start Recording
        </button>

        <button
          onClick={async () => {
            try {
              await setTrayState("processing");
              if (activeEngine === "whisper") toast.loading("Processing transcription...");
              await invoke("stop_recording");
              toast.dismiss();
              // toast.success(res as string);
              setIsRecording(false);
              await setTrayState("ready");
            } catch (e) {
              toast.error("Error: " + e);
              await setTrayState("ready");
            }
          }}
          disabled={!isRecording || isLoading}
          className="btn btn-stop"
        >
          ⏹️ Stop Recording
        </button>

        {/* Benchmark Section */}
        <div className="benchmark-controls">
          <select
            className="sample-select"
            value={selectedSample}
            onChange={(e) => setSelectedSample(e.target.value)}
            disabled={isRecording || isLoading || sampleFiles.length === 0}
          >
            {sampleFiles.length === 0 && <option value="">No samples found</option>}
            {sampleFiles.map((file) => (
              <option key={file.path} value={file.path}>
                📄 {file.name}
              </option>
            ))}
          </select>

          <button
            onClick={async () => {
              if (!selectedSample) return;
              try {
                toast.info(`Running benchmark on ${sampleFiles.find(s => s.path === selectedSample)?.name}...`);
                const res = await invoke("benchmark_test", {
                  filePath: selectedSample
                });
                setGreetMsg(res as string); // Benchmark results still go to text area (too big for toast)
                toast.success("Benchmark completed!");
              } catch (e) {
                toast.error("Benchmark Error: " + e);
              }
            }}
            disabled={isRecording || isLoading || !selectedSample}
            className="btn btn-benchmark"
          >
            🚀 Run Benchmark
          </button>
        </div>
      </div>

      {/* Output Area */}
      {(liveTranscript || greetMsg) && (
        <div className="output-area">
          {isRecording ? (
            <div className="live-transcript">
              <div className="live-header">
                <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                  <span className="live-indicator">LIVE</span>
                  <span style={{ fontSize: '1.2rem' }}>
                    {activeEngine === "whisper" ? "🎙️" : "🦜"}
                  </span>
                </div>
                {latestLatency !== null && (
                  <span className="latency-badge">
                    ⚡ {latestLatency}ms
                  </span>
                )}
              </div>
              <p>{liveTranscript || "Listening..."}</p>
            </div>
          ) : (
            <pre>{greetMsg}</pre>
          )}
        </div>
      )}
    </main>
  );
}

export default App;
