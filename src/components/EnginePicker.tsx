import { useState } from "react";
import type { ASREngine } from "../hooks/useEngineSwitch";
import type { ModelInfo, ParakeetModelInfo, CohereModelInfo } from "../hooks/useModels";
import type { DownloadProgress } from "./settings/types";
import { beautifyModelName, formatSize } from "../utils/modelDisplay";

interface EnginePickerProps {
  activeEngine: ASREngine;
  loadedEngine: ASREngine | null;
  loadingTargetEngine: ASREngine | null;
  models: ModelInfo[];
  currentModel: string | null;
  parakeetModels: ParakeetModelInfo[];
  currentParakeetModel: string | null;
  cohereModels: CohereModelInfo[];
  currentCohereModel: string | null;
  downloadProgress: Record<string, DownloadProgress>;
  isWhisperDownloading: boolean;
  isParakeetDownloading: boolean;
  isCohereDownloading: boolean;
  disabled: boolean;
  onSelectWhisperModel: (id: string) => void;
  onSelectParakeetModel: (id: string) => void;
  onSelectCohereModel: (id: string) => void;
  onUnload: () => void;
  onOpenDownloads: (engine: "whisper" | "parakeet" | "granite") => void;
  onClose: () => void;
}

const ENGINE_META: Record<ASREngine, { label: string; color: string; pill?: string }> = {
  whisper: { label: "Whisper", color: "var(--whisper-color)" },
  parakeet: { label: "Parakeet", color: "var(--parakeet-color)" },
  granite: { label: "Granite", color: "var(--cohere-color)", pill: "Experimental" },
};

const ENGINES: ASREngine[] = ["whisper", "parakeet", "granite"];

export function EnginePicker(props: EnginePickerProps) {
  const {
    activeEngine, loadedEngine, loadingTargetEngine,
    models, currentModel, parakeetModels, currentParakeetModel, cohereModels, currentCohereModel,
    isWhisperDownloading, isParakeetDownloading, isCohereDownloading,
    disabled,
    onSelectWhisperModel, onSelectParakeetModel, onSelectCohereModel,
    onUnload, onOpenDownloads, onClose,
  } = props;

  const [drilled, setDrilled] = useState<ASREngine | null>(null);

  const content = drilled ? (() => {
    const meta = ENGINE_META[drilled];
    const rows = drilled === "whisper"
      ? models.map(m => ({ id: m.id, name: beautifyModelName(m.display_name), size: formatSize(m.size_mb), selected: m.id === currentModel }))
      : drilled === "parakeet"
        ? parakeetModels.map(m => ({ id: m.id, name: beautifyModelName(m.display_name), size: formatSize(m.size_mb), selected: m.id === (currentParakeetModel ?? parakeetModels[0]?.id) }))
        : cohereModels.map(m => ({ id: m.id, name: m.display_name, size: formatSize(m.size_mb), selected: m.id === (currentCohereModel ?? cohereModels[0]?.id) }));

    const isDownloading = drilled === "whisper" ? isWhisperDownloading : drilled === "parakeet" ? isParakeetDownloading : isCohereDownloading;
    const isLoadingThis = loadingTargetEngine === drilled;

    return (
      <>
        <div className="ep-header">
          <button type="button" className="ep-back" onClick={() => setDrilled(null)} aria-label="Back">‹</button>
          <span className="ep-dot" style={{ background: meta.color }} />
          <span className="ep-title" style={{ color: meta.color }}>{meta.label}</span>
          {meta.pill && <span className="ep-pill">{meta.pill}</span>}
        </div>
        <div className="ep-models">
          {rows.length === 0 ? (
            <button type="button" className="ep-model-row ep-model-row--empty" onClick={() => { onOpenDownloads(drilled); onClose(); }}>
              {isDownloading ? "Downloading…" : "Download from Settings"}
            </button>
          ) : rows.map(r => (
            <button
              key={r.id}
              type="button"
              className={`ep-model-row${r.selected ? " ep-model-row--selected" : ""}`}
              disabled={disabled}
              onClick={() => {
                if (drilled === "whisper") onSelectWhisperModel(r.id);
                else if (drilled === "parakeet") onSelectParakeetModel(r.id);
                else onSelectCohereModel(r.id);
                onClose();
              }}
            >
              <span className="ep-model-name">{r.name}</span>
              <span className="ep-model-size">{r.size}</span>
              {isLoadingThis && r.selected && <span className="ep-model-spinner" aria-hidden="true" />}
              {r.selected && loadedEngine === drilled && !isLoadingThis && <span className="ep-model-check" style={{ background: meta.color }} />}
            </button>
          ))}
          {loadedEngine === drilled && (
            <button type="button" className="ep-unload" onClick={() => { onUnload(); onClose(); }}>
              Unload — free VRAM
            </button>
          )}
        </div>
      </>
    );
  })() : (
    <>
      <div className="ep-hd">Select Engine</div>
      {ENGINES.map((engine) => {
        const meta = ENGINE_META[engine];
        const isActive = activeEngine === engine;
        const isLoadingThis = loadingTargetEngine === engine;
        return (
          <button
            key={engine}
            type="button"
            className={`ep-row${isActive ? " ep-row--active" : ""}`}
            onClick={() => setDrilled(engine)}
          >
            <span
              className="ep-row-dot"
              style={{ borderColor: meta.color, background: isActive ? meta.color : "transparent" }}
            />
            <span className="ep-row-name" style={isActive ? { color: meta.color } : undefined}>{meta.label}</span>
            {meta.pill && <span className="ep-pill">{meta.pill}</span>}
            {isLoadingThis && <span className="ep-row-badge">loading…</span>}
            {isActive && !isLoadingThis && <span className="ep-row-badge">active</span>}
            <span className="ep-row-caret" aria-hidden="true">›</span>
          </button>
        );
      })}
    </>
  );

  return (
    <>
      <div className="engine-picker-backdrop" onClick={onClose} />
      <div className="engine-picker">{content}</div>
    </>
  );
}
