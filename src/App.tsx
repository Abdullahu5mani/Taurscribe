import { useState } from "react";

import { invoke } from "@tauri-apps/api/core";
import "./App.css";

function App() {
  const [greetMsg, setGreetMsg] = useState("");
  const [isRecording, setIsRecording] = useState(false);



  return (
    <main className="container">
      <h1>Taurscribe Recorder</h1>

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
      </div>

      <p>{greetMsg}</p>
    </main>
  );
}

export default App;
