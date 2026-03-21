import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { TitleBar } from './TitleBar';
import {
  ONBOARDING_USE_CASES,
  computeModelRecommendation,
  type OnboardingUseCase,
  type SystemInfo,
} from '../modelRecommendations';
import './SetupWizard.css';

interface Props {
  onComplete: (result: { openSettings: boolean; useCase: OnboardingUseCase }) => void;
}

// Step entry tracks which step and which direction it entered from
interface StepEntry {
  idx: number;
  enterDir: 'left' | 'right';
  key: number;
}

const STEPS = 6;

export function SetupWizard({ onComplete }: Props) {
  const [sysInfo, setSysInfo] = useState<SystemInfo | null>(null);
  const [platform, setPlatform] = useState<string>('');
  const [isAppleSilicon, setIsAppleSilicon] = useState(false);
  const [useCase, setUseCase] = useState<OnboardingUseCase>('quick_notes');
  const [current, setCurrent] = useState<StepEntry>({ idx: 0, enterDir: 'right', key: 0 });
  const [exiting, setExiting] = useState<{ idx: number; exitDir: 'left' | 'right'; key: number } | null>(null);
  const transitioning = useRef(false);
  const recommendation = computeModelRecommendation({ sysInfo, isAppleSilicon, useCase });

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
    invoke<string>('get_platform').then(setPlatform).catch(() => {});
    invoke<boolean>('is_apple_silicon').then(setIsAppleSilicon).catch(() => {});
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
      case 1: return <StepHardware sysInfo={sysInfo} platform={platform} onNext={next} onBack={back} />;
      case 2: return (
        <StepEngines
          sysInfo={sysInfo}
          platform={platform}
          isAppleSilicon={isAppleSilicon}
          useCase={useCase}
          onUseCaseChange={setUseCase}
          onNext={next}
          onBack={back}
        />
      );
      case 3: return <StepHotkey onNext={next} onBack={back} platform={platform} />;
      case 4: 
        // Skip permissions step on non-macOS platforms
        if (platform !== 'macos') {
          return <StepReady onComplete={onComplete} platform={platform} recommendation={recommendation} useCase={useCase} />;
        }
        return <StepPermissions onNext={next} onBack={back} platform={platform} />;
      case 5: return <StepReady onComplete={onComplete} platform={platform} recommendation={recommendation} useCase={useCase} />;
      default: return null;
    }
  };

  return (
    <div className="setup-overlay">
      <TitleBar />
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
          Three local engines: Whisper, Parakeet, and Granite Speech
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
  platform,
  onNext,
  onBack,
}: {
  sysInfo: SystemInfo | null;
  platform: string;
  onNext: () => void;
  onBack: () => void;
}) {
  const loading = sysInfo === null;
  // macOS fix: On Apple Silicon, memory is unified (shared between CPU and GPU).
  // Show "Unified Memory" instead of a separate VRAM row.
  const isMac = platform === 'macos';
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
      <p className="setup-eyebrow">Step 2 of 6</p>
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
          {/* macOS fix: Apple Silicon has unified memory shared between CPU
              and GPU, so show a single "Unified" row instead of separate VRAM. */}
          {isMac ? (
            <div className="hw-row">
              <span className="hw-label">Memory</span>
              <span className="hw-value">{sysInfo!.ram_total_gb.toFixed(1)} GB Unified</span>
              <span className="hw-status hw-status--ok" />
            </div>
          ) : sysInfo!.vram_gb !== null ? (
            <div className="hw-row">
              <span className="hw-label">VRAM</span>
              <span className="hw-value">{sysInfo!.vram_gb!.toFixed(1)} GB</span>
              <span className="hw-status hw-status--ok" />
            </div>
          ) : null}
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
function StepEngines({
  sysInfo,
  isAppleSilicon,
  useCase,
  onUseCaseChange,
  onNext,
  onBack,
}: {
  sysInfo: SystemInfo | null;
  platform: string;
  isAppleSilicon: boolean;
  useCase: OnboardingUseCase;
  onUseCaseChange: (value: OnboardingUseCase) => void;
  onNext: () => void;
  onBack: () => void;
}) {
  const recommendation = computeModelRecommendation({ sysInfo, isAppleSilicon, useCase });

  return (
    <>
      <p className="setup-eyebrow">Step 3 of 6</p>
      <h2 className="setup-heading">Recommended Setup</h2>
      <p className="setup-sub">Pick what you do most often.</p>

      <div className="setup-use-cases">
        {ONBOARDING_USE_CASES.map((option) => (
          <button
            key={option.id}
            type="button"
            className={`setup-use-case${useCase === option.id ? ' setup-use-case--active' : ''}`}
            onClick={() => onUseCaseChange(option.id)}
          >
            <span className="setup-use-case-kicker">{option.audience}</span>
            <span className="setup-use-case-title">{option.label}</span>
          </button>
        ))}
      </div>

      <div className="setup-recommendation-stack">
        <div className="setup-recommendation-card setup-recommendation-card--primary">
          <div className="setup-recommendation-topline">
            <span className="setup-recommendation-badge">Start with</span>
            <span className={`setup-recommendation-engine setup-recommendation-engine--${recommendation.primaryEngine}`}>
              {recommendation.primaryEngineLabel}
            </span>
          </div>
          <div className="setup-recommendation-model">{recommendation.primaryLabel}</div>
          <p className="setup-recommendation-summary">{recommendation.summary}</p>
        </div>

        {recommendation.backupModelId && recommendation.backupLabel && recommendation.backupEngine && recommendation.backupEngineLabel && (
          <div className="setup-recommendation-card">
            <div className="setup-recommendation-topline">
              <span className="setup-recommendation-badge setup-recommendation-badge--secondary">Backup</span>
              <span className={`setup-recommendation-engine setup-recommendation-engine--${recommendation.backupEngine}`}>
                {recommendation.backupEngineLabel}
              </span>
            </div>
            <div className="setup-recommendation-model">{recommendation.backupLabel}</div>
          </div>
        )}
      </div>

      <p className="engines-note">{recommendation.hardwareLine}</p>

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
function StepHotkey({ onNext, onBack, platform }: { onNext: () => void; onBack: () => void; platform: string }) {
  // macOS default: Ctrl + Option (Cmd is intercepted by the OS)
  // Windows/Linux default: Ctrl + Win/Super
  const isMac = platform === 'macos';
  const modifierLabel = isMac ? 'Option' : 'Win';
  const comboLabel = `Ctrl + ${modifierLabel}`;

  return (
    <>
      <p className="setup-eyebrow">Step 4 of 6</p>
      <h2 className="setup-heading">One Hotkey</h2>
      <p className="setup-sub">Use Taurscribe from anywhere, without switching windows.</p>

      <div className="hotkey-keys">
        <div className="hotkey-key">Ctrl</div>
        <div className="hotkey-plus">+</div>
        <div className="hotkey-key">{modifierLabel}</div>
      </div>

      <div className="hotkey-steps">
        {[
          'Focus any text field in any app',
          `Press ${comboLabel} to start recording`,
          'Speak naturally',
          `Press ${comboLabel} again to stop`,
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
// STEP 4 — PERMISSIONS
// ─────────────────────────────────────────────────────────────────
function StepPermissions({
  onNext,
  onBack,
  platform,
}: {
  onNext: () => void;
  onBack: () => void;
  platform: string;
}) {
  const isMac = platform === 'macos';

  const [micStatus, setMicStatus] = useState<string>('checking');
  const [accGranted, setAccGranted] = useState<boolean | null>(null);
  const [restartNeeded, setRestartNeeded] = useState(false);
  const [micRequesting, setMicRequesting] = useState(false);
  const [initialCheckDone, setInitialCheckDone] = useState(false);
  const initialAccRef = useRef<boolean | null>(null);

  const checkStatuses = useCallback(async () => {
    try {
      const mic = await invoke<string>('check_microphone_permission');
      setMicStatus(mic);
    } catch { setMicStatus('undetermined'); }
    try {
      const acc = await invoke<boolean>('check_accessibility_permission');
      setAccGranted(acc);
      if (initialAccRef.current === false && acc === true) {
        setRestartNeeded(true);
      }
    } catch { setAccGranted(false); }
    setInitialCheckDone(true);
  }, []);

  useEffect(() => {
    if (!isMac) return;
    invoke<boolean>('check_accessibility_permission')
      .then(v => { initialAccRef.current = v; })
      .catch(() => { initialAccRef.current = false; });
    checkStatuses();
    const timer = setInterval(checkStatuses, 1500);
    return () => clearInterval(timer);
  }, [isMac, checkStatuses]);

  const micOk = micStatus === 'granted';
  const accOk = accGranted === true;

  // Auto-advance once the initial status check confirms both are already granted.
  // This avoids showing the permissions step at all on repeat launches.
  useEffect(() => {
    if (initialCheckDone && micOk && accOk) {
      onNext();
    }
  }, [initialCheckDone, micOk, accOk, onNext]);

  const requestMic = async () => {
    setMicRequesting(true);
    try { await invoke('request_microphone_permission'); } catch {}
    setMicRequesting(false);
    checkStatuses();
  };

  const openAccessibility = async () => {
    try { await invoke('open_accessibility_settings'); } catch {}
  };

  const relaunchApp = async () => {
    try { await invoke('relaunch_app'); } catch {}
  };

  // While the first check is in flight, render nothing to avoid a flash of
  // the permissions UI before auto-advancing.
  if (!initialCheckDone) return null;

  return (
    <>
      <p className="setup-eyebrow">Step 5 of 6</p>
      <h2 className="setup-heading">Permissions</h2>
      <p className="setup-sub">Two permissions needed to record and type text system-wide.</p>

      <div className="perm-list">
        {/* ── Microphone ─────────────────────────────── */}
        <div className={`perm-row${micOk ? ' perm-row--granted' : ''}`}>
          <div className="perm-icon">
            {micOk
              ? <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
              : <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z" /><path d="M19 10v2a7 7 0 0 1-14 0v-2" /><line x1="12" y1="19" x2="12" y2="23" /><line x1="8" y1="23" x2="16" y2="23" /></svg>
            }
          </div>
          <div className="perm-info">
            <div className="perm-name">Microphone</div>
            <div className="perm-desc">Record audio for transcription</div>
          </div>
          <div className="perm-action">
            {micOk
              ? <span className="perm-badge perm-badge--ok">Granted</span>
              : micStatus === 'restricted'
                ? <span className="perm-badge perm-badge--denied">Restricted by policy</span>
                : micStatus === 'denied'
                  ? <button className="setup-btn setup-btn--primary perm-btn" onClick={() => invoke('open_accessibility_settings').catch(() => {})}>
                      Open Settings
                    </button>
                  : <button className="setup-btn setup-btn--primary perm-btn" onClick={requestMic} disabled={micRequesting}>
                      {micRequesting ? 'Requesting…' : 'Grant Access'}
                    </button>
            }
          </div>
        </div>

        {/* ── Accessibility / Input Monitoring ────────── */}
        <div className={`perm-row${accOk ? ' perm-row--granted' : ''}`}>
          <div className="perm-icon">
            {accOk
              ? <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
              : <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><circle cx="12" cy="12" r="10" /><line x1="12" y1="8" x2="12" y2="12" /><line x1="12" y1="16" x2="12.01" y2="16" /></svg>
            }
          </div>
          <div className="perm-info">
            <div className="perm-name">Accessibility</div>
            <div className="perm-desc">Required for the global hotkey to work in all apps</div>
          </div>
          <div className="perm-action">
            {accOk
              ? <span className="perm-badge perm-badge--ok">Granted</span>
              : <button className="setup-btn setup-btn--primary perm-btn" onClick={openAccessibility}>
                  Open Settings
                </button>
            }
          </div>
        </div>
      </div>

      {restartNeeded && (
        <div className="perm-restart-notice">
          <strong>Restart required.</strong> Accessibility was granted — restart so the hotkey activates.
          <button className="setup-btn setup-btn--primary perm-restart-btn" onClick={relaunchApp}>
            Restart Now
          </button>
        </div>
      )}

      <div className="setup-nav setup-nav--spread">
        <button className="setup-btn setup-btn--ghost" onClick={onBack}>← Back</button>
        <button className="setup-btn setup-btn--primary" onClick={onNext}>
          {micOk && accOk ? 'Continue →' : 'Skip for now →'}
        </button>
      </div>
    </>
  );
}

// ─────────────────────────────────────────────────────────────────
// STEP 5 — READY
// ─────────────────────────────────────────────────────────────────
function StepReady({
  onComplete,
  platform,
  recommendation,
  useCase,
}: {
  onComplete: (result: { openSettings: boolean; useCase: OnboardingUseCase }) => void;
  platform: string;
  recommendation: ReturnType<typeof computeModelRecommendation>;
  useCase: OnboardingUseCase;
}) {
  const isMac = platform === 'macos';
  const comboLabel = isMac ? 'Ctrl + Option' : 'Ctrl + Win';

  return (
    <>
      <p className="setup-eyebrow">All done</p>
      <h2 className="setup-heading">Ready.</h2>

      <ul className="ready-checks">
        {[
          'Hardware detected and configured',
          `Starting profile tuned for ${recommendation.useCaseLabel.toLowerCase()}`,
          `Recommended engine: ${recommendation.primaryEngineLabel}`,
          `Global hotkey active: ${comboLabel}`,
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
        Start with <strong>{recommendation.primaryLabel}</strong>
        {recommendation.backupLabel ? <> and keep <strong>{recommendation.backupLabel}</strong> as your fallback.</> : <> in Settings → Models.</>}
      </p>

      <div className="setup-nav--ready">
        <button
          className="setup-btn setup-btn--primary setup-btn--full"
          onClick={() => onComplete({ openSettings: true, useCase })}
        >
          Open Models & Download Recommended
        </button>
        <button
          className="setup-btn setup-btn--settings setup-btn--full"
          onClick={() => onComplete({ openSettings: false, useCase })}
        >
          Launch App
        </button>
      </div>
    </>
  );
}
