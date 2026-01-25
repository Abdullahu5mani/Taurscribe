import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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
  const [parakeetStatus, setParakeetStatus] = useState<ParakeetStatus | null>(null);
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
        const modelList = await invoke("list_models");
        setModels(modelList as ModelInfo[]);

        // Load current model
        const current = await invoke("get_current_model");
        setCurrentModel(current as string | null);

        // Load sample files
        const samples = await invoke("list_sample_files");
        const sampleList = samples as SampleFile[];
        setSampleFiles(sampleList);
        if (sampleList.length > 0) {
          const defaultSample = sampleList.find(s => s.name.includes("otherjack")) || sampleList[0];
          setSelectedSample(defaultSample.path);
        }

        // Load Parakeet data
        const pModels = await invoke("list_parakeet_models");
        setParakeetModels(pModels as ParakeetModelInfo[]);

        const pStatus = await invoke("get_parakeet_status");
        const status = pStatus as ParakeetStatus;
        setParakeetStatus(status);
        setCurrentParakeetModel(status.model_id);

        // Default to Parakeet if a model is loaded and Whisper isn't (rare but possible)
        if (status.loaded && !current) {
          setActiveEngine("parakeet");
        }
      } catch (e) {
        console.error("Failed to load initial data:", e);
        setBackendInfo("Unknown");
        setGreetMsg(`Error loading models: ${e}`);
      } finally {
        setIsInitialLoading(false);
      }
    }
    loadInitialData();
  }, []);

  // Sync active engine with backend
  useEffect(() => {
    if (!isInitialLoading) {
      invoke("set_active_engine", { engine: activeEngine }).catch(console.error);
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
            setGreetMsg(res as string);
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
                  setGreetMsg("Processing transcription...");
                  const stopRes = await invoke("stop_recording");
                  setGreetMsg(stopRes as string);
                  setIsRecording(false);
                  isRecordingRef.current = false;
                  await setTrayState("ready");
                } catch (e) {
                  console.error("Pending stop failed:", e);
                  // Ignore "Not recording" errors, they're expected in race conditions
                  const errStr = String(e);
                  if (!errStr.includes("Not recording")) {
                    setGreetMsg("Error: " + e);
                  }
                  await setTrayState("ready");
                }
              }, 200);
            }
          } catch (e) {
            console.error("Hotkey start recording failed:", e);
            setGreetMsg("Error: " + e);
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
            setGreetMsg("Processing transcription...");
            const res = await invoke("stop_recording");
            setGreetMsg(res as string);
            setIsRecording(false);
            isRecordingRef.current = false;
            await setTrayState("ready");
          } catch (e) {
            console.error("Hotkey stop recording failed:", e);
            // Ignore "Not recording" errors silently - they happen during race conditions
            const errStr = String(e);
            if (!errStr.includes("Not recording")) {
              setGreetMsg("Error: " + e);
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
  }, []);

  const handleModelChange = async (modelId: string) => {
    if (modelId === currentModel) return;

    setIsLoading(true);
    setLoadingMessage(`Loading ${models.find(m => m.id === modelId)?.display_name || modelId}...`);
    setGreetMsg("");

    try {
      await setTrayState("processing");
      const result = await invoke("switch_model", { modelId });
      setCurrentModel(modelId);
      setActiveEngine("whisper");
      setGreetMsg(`✅ ${result}`);

      // Refresh backend info (in case GPU backend changed)
      const backend = await invoke("get_backend_info");
      setBackendInfo(backend as string);
    } catch (e) {
      setGreetMsg(`❌ Error switching model: ${e}`);
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
    setGreetMsg("");

    try {
      await setTrayState("processing");
      const result = await invoke("init_parakeet", { modelId });
      setCurrentParakeetModel(modelId);
      setActiveEngine("parakeet");
      setGreetMsg(`✅ ${result}`);

      const pStatus = await invoke("get_parakeet_status");
      setParakeetStatus(pStatus as ParakeetStatus);
    } catch (e) {
      setGreetMsg(`❌ Error switching Parakeet model: ${e}`);
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
      <h1>🎙️ Taurscribe</h1>

      {/* Status Bar */}
      <div className="status-bar-container">
        <div className={`status-card ${activeEngine === "whisper" ? "active" : ""}`} onClick={() => setActiveEngine("whisper")}>
          <div className="engine-badge whisper">Whisper</div>
          <div className="status-item">
            <span className="status-label">Backend:</span>
            <span className="status-value backend">{backendInfo}</span>
          </div>
          <div className="status-item">
            <span className="status-label">Model:</span>
            <span className="status-value model">
              {currentModel ? models.find(m => m.id === currentModel)?.display_name || currentModel : "None"}
            </span>
          </div>
        </div>

        <div className={`status-card ${activeEngine === "parakeet" ? "active" : ""}`} onClick={() => setActiveEngine("parakeet")}>
          <div className="engine-badge parakeet">Parakeet</div>
          <div className="status-item">
            <span className="status-label">Backend:</span>
            <span className="status-value backend">{parakeetStatus?.backend || "CPU"}</span>
          </div>
          <div className="status-item">
            <span className="status-label">Model:</span>
            <span className="status-value model">
              {currentParakeetModel ? parakeetModels.find(m => m.id === currentParakeetModel)?.display_name || currentParakeetModel : "None"}
            </span>
          </div>
        </div>
      </div>

      {/* Engine Tabs (Mobile/Desktop visibility) */}
      <div className="engine-tabs">
        <button
          className={`tab-btn ${activeEngine === "whisper" ? "active" : ""}`}
          onClick={() => setActiveEngine("whisper")}
        >
          🧠 Whisper AI
        </button>
        <button
          className={`tab-btn ${activeEngine === "parakeet" ? "active" : ""}`}
          onClick={() => setActiveEngine("parakeet")}
        >
          ⚡ Parakeet ASR
        </button>
      </div>

      {/* Model Selection */}
      <div className="model-section">
        {activeEngine === "whisper" ? (
          <>
            <label htmlFor="model-select" className="model-label">
              🧠 Choose Whisper Model
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
                  {model.display_name} ({formatSize(model.size_mb)})
                </option>
              ))}
            </select>
          </>
        ) : (
          <>
            <label htmlFor="parakeet-model-select" className="model-label">
              ⚡ Choose Parakeet Model
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
                  {model.display_name} ({formatSize(model.size_mb)})
                </option>
              ))}
            </select>
          </>
        )}
        <p className="model-hint">
          {activeEngine === "whisper"
            ? "💡 Whisper is highly accurate for various environments."
            : "💡 Parakeet is optimized for extreme speed and real-time streaming."}
        </p>
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
              setGreetMsg(res as string);
              setIsRecording(true);
            } catch (e) {
              await setTrayState("ready");
              setGreetMsg("Error: " + e);
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
              setGreetMsg("Processing transcription...");
              const res = await invoke("stop_recording");
              setGreetMsg(res as string);
              setIsRecording(false);
              await setTrayState("ready");
            } catch (e) {
              setGreetMsg("Error: " + e);
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
                setGreetMsg(`Running benchmark on ${sampleFiles.find(s => s.path === selectedSample)?.name}...`);
                const res = await invoke("benchmark_test", {
                  filePath: selectedSample
                });
                setGreetMsg(res as string);
              } catch (e) {
                setGreetMsg("Benchmark Error: " + e);
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
                <span className="live-indicator">LIVE</span>
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
