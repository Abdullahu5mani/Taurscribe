/** Dev-only: every overlay phase side by side (#preview/overlay). */
import { useEffect, useState } from "react";
import { OverlayPill, type Phase } from "../OverlayApp";

const OVERLAY_PHASES: Phase[] = [
  "recording", "paused", "model_loading", "transcribing", "correcting", "done",
  "cancelled", "too_short", "nothing_heard", "paste_failed", "no_model",
];

export default function OverlayGallery() {
  const [t, setT] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setT((v) => v + 1), 60);
    return () => clearInterval(id);
  }, []);
  const levels = Array.from({ length: 21 }, (_, i) => {
    const x = t * 0.18 - Math.abs(i - 10) * 0.45;
    return Math.max(0, Math.abs(Math.sin(x) * 0.7 + Math.sin(x * 2.3) * 0.3)) * (1 - Math.abs(i - 10) / 14);
  });
  return (
    <div style={{ minHeight: "100vh", background: "linear-gradient(135deg,#6b7a8f,#2b3440)", padding: 32, display: "grid", gap: 14, justifyContent: "center", alignContent: "start" }}>
      {OVERLAY_PHASES.map((phase) => (
        <div key={phase} style={{ display: "flex", alignItems: "center", gap: 16 }}>
          <code style={{ width: 110, color: "#fff", opacity: 0.7, fontSize: 12 }}>{phase}</code>
          <div style={{ width: 236, height: 44 }}>
            <OverlayPill phase={phase} elapsedMs={t * 60} latencyMs={820} levels={levels} />
          </div>
        </div>
      ))}
    </div>
  );
}
