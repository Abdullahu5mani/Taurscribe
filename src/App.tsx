import { useState, useEffect } from "react";

import { invoke } from "@tauri-apps/api/core";
import "./App.css";

function App() {
  const [greetMsg, setGreetMsg] = useState("");
  const [isRecording, setIsRecording] = useState(false);
  const [backendInfo, setBackendInfo] = useState("Loading...");

  // Load backend info on mount
  useEffect(() => {
    async function loadBackend() {
      try {
        const backend = await invoke("get_backend_info");
        setBackendInfo(backend as string);
      } catch (e) {
        setBackendInfo("Unknown");
      }
    }
    loadBackend();
  }, []);

  return (
    <main className="container">
      <h1>Taurscribe Recorder</h1>

      {/* GPU Backend Info */}
      <div style={{
        padding: "10px",
        background: "#2a2a2a",
        borderRadius: "8px",
        marginBottom: "20px",
        fontSize: "14px",
        color: "#a0a0a0"
      }}>
        <strong style={{ color: "#4CAF50" }}>GPU Backend:</strong> {backendInfo}
      </div>

      <div className="row">
        <button
          onClick={async () => {
            try {
              const res = await invoke("start_recording");
              setGreetMsg(res as string);
              setIsRecording(true);
            } catch (e) {
              setGreetMsg("Error: " + e);
            }
          }}
          disabled={isRecording}
        >
          Start Recording
        </button>

        <button
          onClick={async () => {
            try {
              const res = await invoke("stop_recording");
              setGreetMsg(res as string);
              setIsRecording(false);
            } catch (e) {
              setGreetMsg("Error: " + e);
            }
          }}
          disabled={!isRecording}
        >
          Stop Recording
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
          disabled={isRecording}
          style={{ background: "#FF9800" }}
        >
          🚀 Benchmark
        </button>
      </div>

      <p style={{ whiteSpace: "pre-wrap" }}>{greetMsg}</p>
    </main>
  );
}

export default App;
