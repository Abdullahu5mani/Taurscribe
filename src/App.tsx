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

function App() {
  const [greetMsg, setGreetMsg] = useState("");
  const [isRecording, setIsRecording] = useState(false);
  const [backendInfo, setBackendInfo] = useState("Loading...");
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [currentModel, setCurrentModel] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [loadingMessage, setLoadingMessage] = useState("");

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
      } catch (e) {
        console.error("Failed to load initial data:", e);
        setBackendInfo("Unknown");
      }
    }
    loadInitialData();
  }, []);

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
    };

    setupListeners();

    // Cleanup listeners on unmount
    return () => {
      console.log("[HOTKEY] Cleaning up listeners");
      if (unlistenStart) unlistenStart();
      if (unlistenStop) unlistenStop();
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
      <div className="status-bar">
        <div className="status-item">
          <span className="status-label">GPU Backend:</span>
          <span className="status-value backend">{backendInfo}</span>
        </div>
        <div className="status-item">
          <span className="status-label">Model:</span>
          <span className="status-value model">
            {currentModel ? models.find(m => m.id === currentModel)?.display_name || currentModel : "None"}
          </span>
        </div>
      </div>

      {/* Model Selection */}
      <div className="model-section">
        <label htmlFor="model-select" className="model-label">
          🧠 Whisper Model
        </label>
        <select
          id="model-select"
          className="model-select"
          value={currentModel || ""}
          onChange={(e) => handleModelChange(e.target.value)}
          disabled={isRecording || isLoading}
        >
          {models.length === 0 && <option value="">Loading models...</option>}
          {models.map((model) => (
            <option key={model.id} value={model.id}>
              {model.display_name} ({formatSize(model.size_mb)})
            </option>
          ))}
        </select>
        <p className="model-hint">
          💡 Smaller models are faster, larger models are more accurate
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

        <button
          onClick={async () => {
            try {
              setGreetMsg("Running benchmark...");
              const res = await invoke("benchmark_test", {
                filePath: "taurscribe-runtime/samples/otherjack.wav"
              });
              setGreetMsg(res as string);
            } catch (e) {
              setGreetMsg("Benchmark Error: " + e);
            }
          }}
          disabled={isRecording || isLoading}
          className="btn btn-benchmark"
        >
          🚀 Benchmark
        </button>
      </div>

      {/* Output Area */}
      {greetMsg && (
        <div className="output-area">
          <pre>{greetMsg}</pre>
        </div>
      )}
    </main>
  );
}

export default App;
