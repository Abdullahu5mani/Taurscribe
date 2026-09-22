import { useState, useEffect, useLayoutEffect, useRef, useCallback } from "react";
import { useDismissOnOutside } from "../hooks/useDismissOnOutside";
import type { ASREngine } from "../hooks/useEngineSwitch";
import type { ModelInfo, GraniteModelInfo, Qwen3ModelInfo } from "../hooks/useModels";
import type { DownloadProgress } from "./settings/types";
import { beautifyModelName, formatSize } from "../utils/modelDisplay";
import { AUTO_UNLOAD_OPTIONS } from "../hooks/useAutoUnload";
import "./EnginePicker.css";

interface EnginePickerProps {
  activeEngine: ASREngine;
  loadedEngine: ASREngine | null;
  loadingTargetEngine: ASREngine | null;
  models: ModelInfo[];
  currentModel: string | null;
  graniteModels: GraniteModelInfo[];
  currentGraniteModel: string | null;
  qwen3Models: Qwen3ModelInfo[];
  currentQwen3Model: string | null;
  downloadProgress: Record<string, DownloadProgress>;
  isWhisperDownloading: boolean;
  isGraniteDownloading: boolean;
  isQwen3Downloading: boolean;
  disabled: boolean;
  onSelectWhisperModel: (id: string) => void;
  onSelectGraniteModel: (id: string) => void;
  onSelectQwen3Model: (id: string) => void;
  onUnload: () => void;
  onOpenDownloads: (engine: ASREngine) => void;
  onClose: () => void;
  autoUnloadTimeout?: number;
  onUpdateAutoUnloadTimeout?: (seconds: number) => void;
}

const ENGINE_META: Record<ASREngine, { label: string; color: string; blurb: string }> = {
  whisper: { label: "Whisper", color: "var(--whisper-color)", blurb: "Any language, any computer" },
  granite: { label: "Granite", color: "var(--granite-color)", blurb: "Fastest for English" },
  qwen3: { label: "Qwen3-ASR", color: "#a78bfa", blurb: "Most accurate, punctuated" },
};

const ENGINES: ASREngine[] = ["whisper", "granite", "qwen3"];

/** Matches the exit animation in EnginePicker.css. */
const CLOSE_MS = 140;

/**
 * Engine and model picker, anchored above the engine chip. Two views (engines,
 * then one engine's models) slide sideways; the popover resizes smoothly to fit
 * each view and animates in and out.
 */
export function EnginePicker(props: EnginePickerProps) {
  const {
    activeEngine, loadedEngine, loadingTargetEngine,
    models, currentModel, graniteModels, currentGraniteModel, qwen3Models, currentQwen3Model,
    isWhisperDownloading, isGraniteDownloading, isQwen3Downloading,
    disabled,
    onSelectWhisperModel, onSelectGraniteModel, onSelectQwen3Model,
    onUnload, onOpenDownloads, onClose,
    autoUnloadTimeout, onUpdateAutoUnloadTimeout,
  } = props;

  const [drilled, setDrilled] = useState<ASREngine | null>(null);
  const [direction, setDirection] = useState<"forward" | "back">("forward");
  const [closing, setClosing] = useState(false);
  const [height, setHeight] = useState<number | null>(null);
  const viewRef = useRef<HTMLDivElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);

  const close = useCallback(() => {
    if (closing) return;
    setClosing(true);
    setTimeout(onClose, CLOSE_MS);
  }, [closing, onClose]);

  const open = (engine: ASREngine) => {
    setDirection("forward");
    setDrilled(engine);
  };
  const back = () => {
    setDirection("back");
    setDrilled(null);
  };

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        if (drilled) back();
        else close();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [close, drilled]);

  useDismissOnOutside(dialogRef, close, { ignoreSelector: "#engine-chip-button" });

  // The popover's height follows the current view, animated by CSS.
  useLayoutEffect(() => {
    const el = viewRef.current;
    if (!el) return;
    const measure = () => setHeight(el.offsetHeight);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [drilled]);

  const view = drilled ? (() => {
    const meta = ENGINE_META[drilled];
    const rows = drilled === "whisper"
      ? models.map(m => ({ id: m.id, name: beautifyModelName(m.display_name), size: formatSize(m.size_mb), selected: m.id === currentModel }))
      : drilled === "granite"
        ? graniteModels.map(m => ({ id: m.id, name: beautifyModelName(m.display_name), size: formatSize(m.size_mb), selected: m.id === (currentGraniteModel ?? graniteModels[0]?.id) }))
        : qwen3Models.map(m => ({ id: m.id, name: m.display_name, size: formatSize(m.size_mb), selected: m.id === (currentQwen3Model ?? qwen3Models[0]?.id) }));
    const isDownloading = drilled === "whisper" ? isWhisperDownloading : drilled === "granite" ? isGraniteDownloading : isQwen3Downloading;
    const isLoadingThis = loadingTargetEngine === drilled;

    return (
      <>
        <div className="ep-header">
          <button type="button" id="ep-back-btn" data-testid="ep-back-btn" className="ep-back" onClick={back} aria-label="Back to engine list">
            <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true"><path d="M8.5 3L4.5 7l4 4" stroke="currentColor" strokeWidth="1.6" fill="none" strokeLinecap="round" strokeLinejoin="round" /></svg>
          </button>
          <span className="ep-dot" style={{ background: meta.color }} />
          <span className="ep-title">{meta.label}</span>
        </div>
        <div id="ep-models-list" data-testid="ep-models-list" className="ep-models" role="radiogroup" aria-label={`${meta.label} models`}>
          {rows.length === 0 ? (
            <button
              type="button"
              id={`ep-download-${drilled}-btn`}
              data-testid={`ep-download-${drilled}-btn`}
              className="ep-model-row ep-model-row--empty"
              onClick={() => { onOpenDownloads(drilled); close(); }}
              aria-label={`Open settings to download ${drilled} model`}
            >
              {isDownloading ? "Downloading…" : "No model yet · Download in Settings"}
            </button>
          ) : rows.map((r, i) => (
            <button
              key={r.id}
              type="button"
              id={`ep-model-row-${r.id}`}
              data-testid={`ep-model-row-${r.id}`}
              role="radio"
              aria-checked={r.selected}
              aria-label={`Select model ${r.name}, size ${r.size}`}
              className={`ep-model-row${r.selected ? " ep-model-row--selected" : ""}`}
              style={{ animationDelay: `${40 + i * 28}ms` }}
              disabled={disabled}
              onClick={() => {
                if (drilled === "whisper") onSelectWhisperModel(r.id);
                else if (drilled === "granite") onSelectGraniteModel(r.id);
                else onSelectQwen3Model(r.id);
                close();
              }}
            >
              <span className="ep-model-name">{r.name}</span>
              <span className="ep-model-size">{r.size}</span>
              <span className="ep-model-state" aria-hidden="true">
                {isLoadingThis && r.selected ? (
                  <span className="ep-model-spinner" />
                ) : r.selected ? (
                  <svg className="ep-model-check" width="14" height="14" viewBox="0 0 14 14" style={{ color: meta.color }}>
                    <path d="M3 7.4l2.6 2.6L11 4.5" stroke="currentColor" strokeWidth="1.8" fill="none" strokeLinecap="round" strokeLinejoin="round" />
                  </svg>
                ) : null}
              </span>
            </button>
          ))}
        </div>
        {loadedEngine === drilled && (
          <div className="ep-unload-section">
            {onUpdateAutoUnloadTimeout && (
              <div className="ep-auto-unload-row">
                <span className="ep-auto-unload-label">Unload after inactivity</span>
                <div id="engine-picker-auto-unload-options" data-testid="engine-picker-auto-unload-options" className="ep-auto-unload-chips" role="group" aria-label="Auto-unload inactivity options">
                  {AUTO_UNLOAD_OPTIONS.map((opt) => (
                    <button
                      key={opt.value}
                      type="button"
                      id={`engine-picker-auto-unload-${opt.value}`}
                      data-testid={`engine-picker-auto-unload-${opt.value}`}
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
            <button type="button" id="ep-unload-btn" data-testid="ep-unload-btn" className="ep-unload" onClick={() => { onUnload(); close(); }} aria-label="Unload model and free VRAM">
              Unload now and free memory
            </button>
          </div>
        )}
      </>
    );
  })() : (
    <>
      <div className="ep-hd">Speech engine</div>
      <div className="ep-engines">
        {ENGINES.map((engine, i) => {
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
              style={{ animationDelay: `${30 + i * 40}ms`, ["--engine-color" as string]: meta.color }}
              onClick={() => open(engine)}
              aria-label={`Engine ${meta.label}${isActive ? ", active" : ""}`}
            >
              <span className="ep-row-dot" />
              <span className="ep-row-text">
                <span className="ep-row-name">{meta.label}</span>
                <span className="ep-row-blurb">{meta.blurb}</span>
              </span>
              {isLoadingThis && <span className="ep-row-badge">Loading…</span>}
              {isActive && !isLoadingThis && <span className="ep-row-badge ep-row-badge--active">In use</span>}
              <svg className="ep-row-caret" width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"><path d="M4.5 2.5L8 6l-3.5 3.5" stroke="currentColor" strokeWidth="1.5" fill="none" strokeLinecap="round" strokeLinejoin="round" /></svg>
            </button>
          );
        })}
      </div>
    </>
  );

  return (
    <>
      <div className="engine-picker-backdrop" id="engine-picker-backdrop" data-testid="engine-picker-backdrop" onClick={close} aria-hidden="true" />
      <div
        className={`engine-picker${closing ? " engine-picker--closing" : ""}`}
        ref={dialogRef}
        id="engine-picker-dialog"
        data-testid="engine-picker-dialog"
        role="dialog"
        aria-label="Engine picker"
        aria-modal="true"
        style={height != null ? { height } : undefined}
      >
        <div key={drilled ?? "engines"} ref={viewRef} className={`ep-view ep-view--${direction}`}>
          {view}
        </div>
      </div>
    </>
  );
}
