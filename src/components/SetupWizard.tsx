import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import './SetupWizard.css';

interface SystemInfo {
  cpu_name: string;
  cpu_cores: number;
  ram_total_gb: number;
  gpu_name: string;
  cuda_available: boolean;
  vram_gb: number | null;
  backend_hint: string;
}

interface Props {
  onComplete: (openSettings: boolean) => void;
}

// Step entry tracks which step and which direction it entered from
interface StepEntry {
  idx: number;
  enterDir: 'left' | 'right';
  key: number;
}

const STEPS = 5;

export function SetupWizard({ onComplete }: Props) {
  const [sysInfo, setSysInfo] = useState<SystemInfo | null>(null);
  const [current, setCurrent] = useState<StepEntry>({ idx: 0, enterDir: 'right', key: 0 });
  const [exiting, setExiting] = useState<{ idx: number; exitDir: 'left' | 'right'; key: number } | null>(null);
  const transitioning = useRef(false);

  useEffect(() => {
    invoke<SystemInfo>('get_system_info')
      .then(setSysInfo)
      .catch(() => setSysInfo({
        cpu_name: 'Unknown',
        cpu_cores: 0,
        ram_total_gb: 0,
        gpu_name: 'Unknown',
        cuda_available: false,
        vram_gb: null,
        backend_hint: 'CPU',
      }));
  }, []);

  const goTo = useCallback((target: number) => {
    if (transitioning.current) return;
    transitioning.current = true;

    const forward = target > current.idx;
    setExiting({ idx: current.idx, exitDir: forward ? 'left' : 'right', key: current.key });
    setCurrent({ idx: target, enterDir: forward ? 'right' : 'left', key: current.key + 1 });

    setTimeout(() => {
      setExiting(null);
      transitioning.current = false;
    }, 400);
  }, [current]);

  const next = useCallback(() => goTo(current.idx + 1), [goTo, current.idx]);
  const back = useCallback(() => goTo(current.idx - 1), [goTo, current.idx]);

  const renderStep = (idx: number) => {
    switch (idx) {
      case 0: return <StepWelcome onNext={next} />;
      case 1: return <StepHardware sysInfo={sysInfo} onNext={next} onBack={back} />;
      case 2: return <StepEngines onNext={next} onBack={back} />;
      case 3: return <StepHotkey onNext={next} onBack={back} />;
      case 4: return <StepReady onComplete={onComplete} />;
      default: return null;
    }
  };

  return (
    <div className="setup-overlay">
      <div className="setup-dots">
        {Array.from({ length: STEPS }).map((_, i) => (
          <div
            key={i}
            className={`setup-dot ${i === current.idx ? 'active' : i < current.idx ? 'passed' : ''}`}
          />
        ))}
      </div>

      <div className="setup-stage">
        {exiting && (
          <div
            key={`exit-${exiting.key}`}
            className={`setup-step setup-step--exit-${exiting.exitDir}`}
          >
            {renderStep(exiting.idx)}
          </div>
        )}
        <div
          key={`enter-${current.key}`}
          className={`setup-step setup-step--enter-${current.enterDir}`}
        >
          {renderStep(current.idx)}
        </div>
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────
// STEP 0 — WELCOME
// ─────────────────────────────────────────────────────────────────
function StepWelcome({ onNext }: { onNext: () => void }) {
  return (
    <>
      <h1 className="welcome-logo">Taurscribe</h1>
      <hr className="welcome-rule" />
      <p className="welcome-tagline">Local AI speech recognition</p>

      <ul className="welcome-features">
        <li className="welcome-feature">
          <span className="welcome-feature-dot" />
          100% offline — nothing leaves your machine
        </li>
        <li className="welcome-feature">
          <span className="welcome-feature-dot" />
          GPU-accelerated transcription with Whisper & Parakeet
        </li>
        <li className="welcome-feature">
          <span className="welcome-feature-dot" />
          Types directly into any app via global hotkey
        </li>
      </ul>

      <div className="setup-nav">
        <button className="setup-btn setup-btn--primary" onClick={onNext}>
          Begin Setup →
        </button>
      </div>
    </>
  );
}

// ─────────────────────────────────────────────────────────────────
// STEP 1 — HARDWARE
// ─────────────────────────────────────────────────────────────────
function StepHardware({
  sysInfo,
  onNext,
  onBack,
}: {
  sysInfo: SystemInfo | null;
  onNext: () => void;
  onBack: () => void;
}) {
  const loading = sysInfo === null;
  const ramOk = (sysInfo?.ram_total_gb ?? 0) >= 8;
  const hasGpu = sysInfo?.gpu_name && sysInfo.gpu_name !== 'Unknown';

  const verdict = () => {
    if (!sysInfo) return null;
    if (sysInfo.cuda_available) {
      return <p className="hw-verdict"><strong>GPU acceleration ready.</strong> Whisper and Parakeet both run at full speed.</p>;
    }
    if (hasGpu) {
      return <p className="hw-verdict"><strong className="amber">GPU detected (no CUDA).</strong> Whisper via CPU — consider downloading a smaller model.</p>;
    }
    return <p className="hw-verdict">No GPU detected. Transcription will use the CPU — choose a small Whisper model for best performance.</p>;
  };

  return (
    <>
      <p className="setup-eyebrow">Step 2 of 5</p>
      <h2 className="setup-heading">System Analysis</h2>
      <p className="setup-sub">Checking your hardware for AI readiness.</p>

      <div className="hw-scan-bar" style={{ display: loading ? undefined : 'none' }} />

      {!loading && (
        <div className="hw-grid">
          <div className="hw-row">
            <span className="hw-label">CPU</span>
            <span className="hw-value">{sysInfo!.cpu_name}{sysInfo!.cpu_cores > 0 ? ` · ${sysInfo!.cpu_cores} threads` : ''}</span>
            <span className="hw-status hw-status--ok" />
          </div>
          <div className="hw-row">
            <span className="hw-label">RAM</span>
            <span className="hw-value">{sysInfo!.ram_total_gb.toFixed(1)} GB</span>
            <span className={`hw-status ${ramOk ? 'hw-status--ok' : 'hw-status--warn'}`} />
          </div>
          <div className="hw-row">
            <span className="hw-label">GPU</span>
            <span className={`hw-value ${!hasGpu ? 'hw-value--dim' : ''}`}>
              {hasGpu ? sysInfo!.gpu_name : 'Not detected'}
            </span>
            <span className={`hw-status ${hasGpu ? 'hw-status--ok' : 'hw-status--warn'}`} />
          </div>
          {sysInfo!.vram_gb !== null && (
            <div className="hw-row">
              <span className="hw-label">VRAM</span>
              <span className="hw-value">{sysInfo!.vram_gb!.toFixed(1)} GB</span>
              <span className="hw-status hw-status--ok" />
            </div>
          )}
          <div className="hw-row">
            <span className="hw-label">AI</span>
            <span className="hw-value">{sysInfo!.backend_hint}</span>
            <span className={`hw-status ${sysInfo!.cuda_available ? 'hw-status--ok' : 'hw-status--warn'}`} />
          </div>
        </div>
      )}

      {verdict()}

      <div className="setup-nav setup-nav--spread">
        <button className="setup-btn setup-btn--ghost" onClick={onBack}>← Back</button>
        <button className="setup-btn setup-btn--primary" onClick={onNext} disabled={loading}>
          Continue →
        </button>
      </div>
    </>
  );
}

// ─────────────────────────────────────────────────────────────────
// STEP 2 — ENGINES
// ─────────────────────────────────────────────────────────────────
function StepEngines({ onNext, onBack }: { onNext: () => void; onBack: () => void }) {
  return (
    <>
      <p className="setup-eyebrow">Step 3 of 5</p>
      <h2 className="setup-heading">Two Engines</h2>
      <p className="setup-sub">Both are included. Download models for either in Settings.</p>

      <div className="engines-grid">
        <div className="engine-card">
          <div>
            <div className="engine-card-name">Whisper</div>
            <div className="engine-card-source">by OpenAI</div>
          </div>
          <ul className="engine-card-traits">
            <li className="engine-card-trait">Highest accuracy</li>
            <li className="engine-card-trait">Multilingual</li>
            <li className="engine-card-trait">Any GPU or CPU</li>
            <li className="engine-card-trait">Buffered (6s chunks)</li>
          </ul>
        </div>

        <div className="engine-card">
          <div>
            <div className="engine-card-name">Parakeet</div>
            <div className="engine-card-source">by NVIDIA Nemotron</div>
          </div>
          <ul className="engine-card-traits">
            <li className="engine-card-trait">Real-time streaming</li>
            <li className="engine-card-trait">Under 500ms latency</li>
            <li className="engine-card-trait">NVIDIA GPU required</li>
            <li className="engine-card-trait">English only</li>
          </ul>
        </div>
      </div>

      <p className="engines-note">Switch between engines anytime in the main UI.</p>

      <div className="engines-coreml-note">
        <span className="engines-coreml-badge">CoreML</span>
        Apple Silicon · CoreML encoder libraries are available for Whisper — download them
        in Settings → Downloads to offload the encoder to the Neural Engine for faster,
        lower-power transcription on M-series Macs.
      </div>

      <div className="setup-nav setup-nav--spread">
        <button className="setup-btn setup-btn--ghost" onClick={onBack}>← Back</button>
        <button className="setup-btn setup-btn--primary" onClick={onNext}>Continue →</button>
      </div>
    </>
  );
}

// ─────────────────────────────────────────────────────────────────
// STEP 3 — HOTKEY
// ─────────────────────────────────────────────────────────────────
function StepHotkey({ onNext, onBack }: { onNext: () => void; onBack: () => void }) {
  return (
    <>
      <p className="setup-eyebrow">Step 4 of 5</p>
      <h2 className="setup-heading">One Hotkey</h2>
      <p className="setup-sub">Use Taurscribe from anywhere, without switching windows.</p>

      <div className="hotkey-keys">
        <div className="hotkey-key">Ctrl</div>
        <div className="hotkey-plus">+</div>
        <div className="hotkey-key">Win</div>
      </div>

      <div className="hotkey-steps">
        {[
          'Focus any text field in any app',
          'Press Ctrl + Win to start recording',
          'Speak naturally',
          'Press Ctrl + Win again to stop',
          'Text appears at your cursor instantly',
        ].map((text, i) => (
          <div className="hotkey-step" key={i}>
            <span className="hotkey-step-num">0{i + 1}</span>
            <span className="hotkey-step-text">{text}</span>
          </div>
        ))}
      </div>

      <p className="hotkey-privacy">No internet · No cloud · No tracking</p>

      <div className="setup-nav setup-nav--spread">
        <button className="setup-btn setup-btn--ghost" onClick={onBack}>← Back</button>
        <button className="setup-btn setup-btn--primary" onClick={onNext}>Continue →</button>
      </div>
    </>
  );
}

// ─────────────────────────────────────────────────────────────────
// STEP 4 — READY
// ─────────────────────────────────────────────────────────────────
function StepReady({
  onComplete,
}: {
  onComplete: (openSettings: boolean) => void;
}) {
  return (
    <>
      <p className="setup-eyebrow">All done</p>
      <h2 className="setup-heading">Ready.</h2>

      <ul className="ready-checks">
        {[
          'Hardware detected and configured',
          'AI engines ready to load',
          'Global hotkey active: Ctrl + Win',
          'Pastes directly into any app',
        ].map((text, i) => (
          <li className="ready-check" key={i}>
            <span className="ready-check-icon">
              <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
                <polyline points="1.5,5 4,7.5 8.5,2.5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            </span>
            {text}
          </li>
        ))}
      </ul>

      <p className="ready-cta-note">
        Download a <strong>Whisper</strong> or <strong>Parakeet</strong> model<br />
        in Settings → Downloads to start transcribing.
      </p>

      <div className="setup-nav--ready">
        <button
          className="setup-btn setup-btn--primary setup-btn--full"
          onClick={() => onComplete(true)}
        >
          Open Settings & Download a Model
        </button>
        <button
          className="setup-btn setup-btn--settings setup-btn--full"
          onClick={() => onComplete(false)}
        >
          Launch App
        </button>
      </div>
    </>
  );
}
