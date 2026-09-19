import { useState } from "react";
import type { ASREngine } from "../hooks/useEngineSwitch";
import type { ModelInfo, ParakeetModelInfo, CohereModelInfo } from "../hooks/useModels";
import type { DownloadProgress } from "./settings/types";
import { beautifyModelName, formatSize } from "../utils/modelDisplay";
import { AUTO_UNLOAD_OPTIONS } from "../hooks/useAutoUnload";

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
  autoUnloadTimeout?: number;
  onUpdateAutoUnloadTimeout?: (seconds: number) => void;
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
    autoUnloadTimeout, onUpdateAutoUnloadTimeout,
  } = props;

  const [drilled, setDrilled] = useState<ASREngine | null>(null);
  const graniteBadgeForId = (id: string) => id.includes("cuda") ? "CUDA" : id.includes("portable") ? "PORTABLE" : null;

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
          <button
            type="button"
            id="ep-back-btn"
            data-testid="ep-back-btn"
            className="ep-back"
            onClick={() => setDrilled(null)}
            aria-label="Back to engine list"
          >
            ‹
          </button>
          <span className="ep-dot" style={{ background: meta.color }} />
          <span className="ep-title" style={{ color: meta.color }}>{meta.label}</span>
          {meta.pill && <span className="ep-pill">{meta.pill}</span>}
        </div>
        <div
          id="ep-models-list"
          data-testid="ep-models-list"
          className="ep-models"
          role="radiogroup"
          aria-label={`${meta.label} models`}
        >
          {rows.length === 0 ? (
            <button
              type="button"
              id={`ep-download-${drilled}-btn`}
              data-testid={`ep-download-${drilled}-btn`}
              className="ep-model-row ep-model-row--empty"
              onClick={() => { onOpenDownloads(drilled); onClose(); }}
              aria-label={`Open settings to download ${drilled} model`}
            >
              {isDownloading ? "Downloading…" : "Download from Settings"}
            </button>
          ) : rows.map(r => (
            <button
              key={r.id}
              type="button"
              id={`ep-model-row-${r.id}`}
              data-testid={`ep-model-row-${r.id}`}
              role="radio"
              aria-checked={r.selected}
              aria-label={`Select model ${r.name}, size ${r.size}`}
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
              {drilled === "granite" && graniteBadgeForId(r.id) && (
                <span className={`ep-model-hardware-badge${graniteBadgeForId(r.id) === "CUDA" ? " ep-model-hardware-badge--cuda" : " ep-model-hardware-badge--portable"}`}>
                  {graniteBadgeForId(r.id)}
                </span>
              )}
              <span className="ep-model-size">{r.size}</span>
              {isLoadingThis && r.selected && <span className="ep-model-spinner" aria-hidden="true" />}
              {r.selected && loadedEngine === drilled && !isLoadingThis && <span className="ep-model-check" style={{ background: meta.color }} />}
            </button>
          ))}
          {loadedEngine === drilled && (
            <div className="ep-unload-section">
              <button
                type="button"
                id="ep-unload-btn"
                data-testid="ep-unload-btn"
                className="ep-unload"
                onClick={() => { onUnload(); onClose(); }}
                aria-label="Unload model and free VRAM"
              >
                Unload — free VRAM
              </button>
              {onUpdateAutoUnloadTimeout && (
                <div className="ep-auto-unload-row">
                  <span className="ep-auto-unload-label">Auto-unload:</span>
                  <div className="ep-auto-unload-chips" role="group" aria-label="Auto-unload inactivity options">
                    {AUTO_UNLOAD_OPTIONS.map((opt) => (
                      <button
                        key={opt.value}
                        type="button"
                        className={`ep-auto-unload-chip${autoUnloadTimeout === opt.value ? " ep-auto-unload-chip--selected" : ""}`}
                        onClick={() => onUpdateAutoUnloadTimeout(opt.value)}
                        title={opt.description}
                      >
                        {opt.shortLabel}
                      </button>
                    ))}
                  </div>
                </div>
              )}
            </div>
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
            id={`ep-engine-row-${engine}`}
            data-testid={`ep-engine-row-${engine}`}
            className={`ep-row${isActive ? " ep-row--active" : ""}`}
            onClick={() => setDrilled(engine)}
            aria-label={`Engine ${meta.label}${isActive ? ", active" : ""}`}
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
      <div
        className="engine-picker-backdrop"
        id="engine-picker-backdrop"
        data-testid="engine-picker-backdrop"
        onClick={onClose}
        aria-hidden="true"
      />
      <div
        className="engine-picker"
        id="engine-picker-dialog"
        data-testid="engine-picker-dialog"
        role="dialog"
        aria-label="Engine picker"
        aria-modal="true"
      >
        {content}
      </div>
    </>
  );
}
