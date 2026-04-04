import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { TitleBar } from './TitleBar';
import {
  computeModelRecommendation,
  type OnboardingUseCase,
  type SystemInfo,
} from '../modelRecommendations';
import type { DownloadableModel, DownloadProgress } from './settings/types';
import './SetupWizard.css';

interface Props {
  onComplete: (result: { openSettings: boolean; useCase: OnboardingUseCase }) => void;
  handleDownload: (id: string, name: string) => void;
  handleCancelDownload: (id: string) => void;
  downloadProgress: Record<string, DownloadProgress>;
  settingsModels: DownloadableModel[];
  enableDenoise: boolean;
  setEnableDenoise: (val: boolean) => void;
  enableOverlay: boolean;
  setEnableOverlay: (val: boolean) => void;
  muteBackgroundAudio: boolean;
  setMuteBackgroundAudio: (val: boolean) => void;
}

// Step entry tracks which step and which direction it entered from
interface StepEntry {
  idx: number;
  enterDir: 'left' | 'right';
  key: number;
}

type SetupEngineId = 'whisper' | 'parakeet' | 'cohere';

const ENGINE_CAROUSEL_SLIDES: Array<{
  id: SetupEngineId;
  title: string;
  subtitle: string;
  goodAt: string[];
  usage: string[];
}> = [
  {
    id: 'whisper',
    title: 'Whisper',
    subtitle: 'Most accurate all-rounder. Best default for mixed audio.',
    goodAt: [
      'Meetings, interviews, and long-form dictation.',
      'Multilingual transcription and mixed accents.',
      'When you want fewer corrections over raw speed.',
    ],
    usage: [
      'Start with Small or Medium for daily use.',
      'Use multilingual variants when language may vary.',
      'Pick quantized models on lower-memory machines.',
    ],
  },
  {
    id: 'parakeet',
    title: 'Parakeet',
    subtitle: 'Fastest live captions. Tuned for English streaming.',
    goodAt: [
      'Real-time dictation where latency matters most.',
      'Short commands and rapid back-to-back notes.',
      'Power users who prioritize immediate feedback.',
    ],
    usage: [
      'Use for active typing sessions and coding flow.',
      'Best with a stable microphone and clear speech.',
      'Switch to Whisper for harder audio or multilingual content.',
    ],
  },
  {
    id: 'cohere',
    title: 'Cohere Speech',
    subtitle: 'English ONNX path for specific hardware workflows.',
    goodAt: [
      'English transcription with the Cohere runtime path.',
      'Users validating ONNX backend behavior.',
      'Controlled comparisons against Whisper/Parakeet.',
    ],
    usage: [
      'Treat as a specialized option, not first choice.',
      'Use when you specifically want Cohere engine output.',
      'For general daily use, Whisper or Parakeet is usually better.',
    ],
  },
];

const SETUP_WELCOME_LOGOS = [
  '/logos/animated_logo_assemble.svg',
  '/logos/animated_logo_blueprint.svg',
  '/logos/animated_logo_bottom_spin.svg',
  '/logos/animated_logo_bounce.svg',
  '/logos/animated_logo_breathe.svg',
  '/logos/animated_logo_coaster.svg',
  '/logos/animated_logo_crt.svg',
  '/logos/animated_logo_debris.svg',
  '/logos/animated_logo_flare.svg',
  '/logos/animated_logo_flip.svg',
  '/logos/animated_logo_focus.svg',
  '/logos/animated_logo_glitch.svg',
  '/logos/animated_logo_grow.svg',
  '/logos/animated_logo_handwrite.svg',
  '/logos/animated_logo_heartbeat.svg',
  '/logos/animated_logo_hologram.svg',
  '/logos/animated_logo_laser_trace.svg',
  '/logos/animated_logo_liquid.svg',
  '/logos/animated_logo_orbit.svg',
  '/logos/animated_logo_pulse_reveal.svg',
  '/logos/animated_logo_quantum_flip.svg',
  '/logos/animated_logo_ripple.svg',
  '/logos/animated_logo_rubberband.svg',
  '/logos/animated_logo_scan_reveal.svg',
  '/logos/animated_logo_shockwave.svg',
  '/logos/animated_logo_slice.svg',
  '/logos/animated_logo_spiral.svg',
  '/logos/animated_logo_split_door.svg',
  '/logos/animated_logo_stomp.svg',
  '/logos/animated_logo_swing.svg',
  '/logos/animated_logo_thin_air.svg',
  '/logos/animated_logo_wiper.svg',
  '/logos/animated_logo_write.svg',
  '/logos/animated_logo_zigzag.svg',
];

function pickRandomSetupLogo(): string | null {
  if (SETUP_WELCOME_LOGOS.length === 0) return null;
  const index = Math.floor(Math.random() * SETUP_WELCOME_LOGOS.length);
  return SETUP_WELCOME_LOGOS[index];
}

export function SetupWizard({
  onComplete,
  handleDownload,
  handleCancelDownload,
  downloadProgress,
  settingsModels,
  enableDenoise,
  setEnableDenoise,
  enableOverlay,
  setEnableOverlay,
  muteBackgroundAudio,
  setMuteBackgroundAudio,
}: Props) {
  const [sysInfo, setSysInfo] = useState<SystemInfo | null>(null);
  const [platform, setPlatform] = useState<string>('');
  const [isAppleSilicon, setIsAppleSilicon] = useState(false);
  const [useCase, setUseCase] = useState<OnboardingUseCase>('quick_notes');
  const [welcomeLogoSrc] = useState<string | null>(() => pickRandomSetupLogo());
  const [current, setCurrent] = useState<StepEntry>({ idx: 0, enterDir: 'right', key: 0 });
  const [exiting, setExiting] = useState<{ idx: number; exitDir: 'left' | 'right'; key: number } | null>(null);
  const transitioning = useRef(false);
  const recommendation = computeModelRecommendation({ sysInfo, isAppleSilicon, useCase });
  const totalSteps = platform === 'macos' ? 7 : 6;

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
      case 0: return <StepWelcome onNext={next} logoSrc={welcomeLogoSrc} />;
      case 1: return <StepHardware sysInfo={sysInfo} platform={platform} onNext={next} onBack={back} totalSteps={totalSteps} />;
      case 2: return (
        <StepEngines
          onNext={next}
          onBack={back}
          totalSteps={totalSteps}
        />
      );
      case 3: return <StepHotkey onNext={next} onBack={back} platform={platform} totalSteps={totalSteps} />;
      case 4: return (
        <StepRecordingSettings
          onNext={next}
          onBack={back}
          totalSteps={totalSteps}
          enableDenoise={enableDenoise}
          setEnableDenoise={setEnableDenoise}
          enableOverlay={enableOverlay}
          setEnableOverlay={setEnableOverlay}
          muteBackgroundAudio={muteBackgroundAudio}
          setMuteBackgroundAudio={setMuteBackgroundAudio}
        />
      );
      case 5:
        // Skip permissions step on non-macOS platforms
        if (platform !== 'macos') {
          return <StepReady onComplete={onComplete} platform={platform} recommendation={recommendation} useCase={useCase} handleDownload={handleDownload} handleCancelDownload={handleCancelDownload} downloadProgress={downloadProgress} settingsModels={settingsModels} />;
        }
        return <StepPermissions onNext={next} onBack={back} platform={platform} totalSteps={totalSteps} />;
      case 6: return <StepReady onComplete={onComplete} platform={platform} recommendation={recommendation} useCase={useCase} handleDownload={handleDownload} handleCancelDownload={handleCancelDownload} downloadProgress={downloadProgress} settingsModels={settingsModels} />;
      default: return null;
    }
  };

  return (
    <div className="setup-overlay">
      <TitleBar />
      <div className="setup-dots">
        {Array.from({ length: totalSteps }).map((_, i) => (
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
function StepWelcome({ onNext, logoSrc }: { onNext: () => void; logoSrc: string | null }) {
  return (
    <>
      {logoSrc && (
        <img
          className="setup-welcome-logo-image"
          src={logoSrc}
          alt=""
          aria-hidden="true"
          draggable={false}
        />
      )}
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
          Three local engines: Whisper, Parakeet, and Cohere Speech
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
  totalSteps,
}: {
  sysInfo: SystemInfo | null;
  platform: string;
  onNext: () => void;
  onBack: () => void;
  totalSteps: number;
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
      <p className="setup-eyebrow">Step 2 of {totalSteps}</p>
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
  onNext,
  onBack,
  totalSteps,
}: {
  onNext: () => void;
  onBack: () => void;
  totalSteps: number;
}) {
  const [activeSlide, setActiveSlide] = useState(0);
  const [navDirection, setNavDirection] = useState<'next' | 'prev'>('next');
  const slide = ENGINE_CAROUSEL_SLIDES[activeSlide];

  const goToSlide = (index: number, direction: 'next' | 'prev') => {
    setNavDirection(direction);
    setActiveSlide(index);
  };

  const goPrev = () => {
    const target = activeSlide === 0 ? ENGINE_CAROUSEL_SLIDES.length - 1 : activeSlide - 1;
    goToSlide(target, 'prev');
  };

  const goNext = () => {
    const target = (activeSlide + 1) % ENGINE_CAROUSEL_SLIDES.length;
    goToSlide(target, 'next');
  };

  return (
    <>
      <p className="setup-eyebrow">Step 3 of {totalSteps}</p>
      <h2 className="setup-heading">Meet The Engines</h2>
      <p className="setup-sub">Swipe through each engine to learn where it shines and when to use it.</p>

      <div className="setup-engine-carousel" aria-live="polite">
        <div className={`setup-engine-carousel-card setup-engine-carousel-card--${slide.id}`}>
          <div className={`setup-engine-carousel-bg setup-engine-carousel-bg--whisper${slide.id === 'whisper' ? ' is-active' : ''}`} />
          <div className={`setup-engine-carousel-bg setup-engine-carousel-bg--parakeet${slide.id === 'parakeet' ? ' is-active' : ''}`} />
          <div className={`setup-engine-carousel-bg setup-engine-carousel-bg--cohere${slide.id === 'cohere' ? ' is-active' : ''}`} />

          <div key={`${slide.id}-${activeSlide}`} className={`setup-engine-carousel-content setup-engine-carousel-content--${navDirection}`}>
            <div className="setup-engine-carousel-topline">
              <span className={`setup-engine-chip setup-engine-chip--${slide.id}`}>{slide.title}</span>
              <span className="setup-engine-slide-index">{activeSlide + 1} / {ENGINE_CAROUSEL_SLIDES.length}</span>
            </div>
            <p className="setup-engine-carousel-subtitle">{slide.subtitle}</p>

            <div className="setup-engine-carousel-grid">
              <div className="setup-engine-carousel-column">
                <p className="setup-engine-column-title">Good At</p>
                <ul className="setup-engine-list">
                  {slide.goodAt.map((item) => (
                    <li key={item}>{item}</li>
                  ))}
                </ul>
              </div>
              <div className="setup-engine-carousel-column">
                <p className="setup-engine-column-title">How To Use</p>
                <ul className="setup-engine-list">
                  {slide.usage.map((item) => (
                    <li key={item}>{item}</li>
                  ))}
                </ul>
              </div>
            </div>
          </div>
        </div>

        <div className="setup-engine-carousel-controls">
          <button type="button" className="setup-engine-carousel-btn" onClick={goPrev} aria-label="Previous engine">
            ← Prev
          </button>
          <div className="setup-engine-carousel-dots" role="tablist" aria-label="Engine slides">
            {ENGINE_CAROUSEL_SLIDES.map((engine, index) => (
              <button
                key={engine.id}
                type="button"
                role="tab"
                aria-selected={index === activeSlide}
                className={`setup-engine-carousel-dot${index === activeSlide ? ' setup-engine-carousel-dot--active' : ''}`}
                onClick={() => goToSlide(index, index >= activeSlide ? 'next' : 'prev')}
                title={engine.title}
              />
            ))}
          </div>
          <button type="button" className="setup-engine-carousel-btn" onClick={goNext} aria-label="Next engine">
            Next →
          </button>
        </div>
      </div>

      <p className="engines-note">You can switch engines anytime from the main panel after setup.</p>

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
function StepHotkey({ onNext, onBack, platform, totalSteps }: { onNext: () => void; onBack: () => void; platform: string; totalSteps: number }) {
  // macOS default: Ctrl + Option (Cmd is intercepted by the OS)
  // Windows/Linux default: Ctrl + Win/Super
  const isMac = platform === 'macos';
  const modifierLabel = isMac ? 'Option' : 'Win';
  const comboLabel = `Ctrl + ${modifierLabel}`;

  return (
    <>
      <p className="setup-eyebrow">Step 4 of {totalSteps}</p>
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
// STEP 4 — RECORDING SETTINGS
// ─────────────────────────────────────────────────────────────────
function StepRecordingSettings({
  onNext,
  onBack,
  totalSteps,
  enableDenoise,
  setEnableDenoise,
  enableOverlay,
  setEnableOverlay,
  muteBackgroundAudio,
  setMuteBackgroundAudio,
}: {
  onNext: () => void;
  onBack: () => void;
  totalSteps: number;
  enableDenoise: boolean;
  setEnableDenoise: (val: boolean) => void;
  enableOverlay: boolean;
  setEnableOverlay: (val: boolean) => void;
  muteBackgroundAudio: boolean;
  setMuteBackgroundAudio: (val: boolean) => void;
}) {
  return (
    <>
      <p className="setup-eyebrow">Step 5 of {totalSteps}</p>
      <h2 className="setup-heading">Recording Settings</h2>
      <p className="setup-sub">Set your default behavior now. You can change these anytime in Settings.</p>

      <div className="setup-recording-settings-grid">
        <div className="setup-recording-setting-row">
          <div className="setup-recording-setting-copy">
            <p className="setup-recording-setting-title">Denoise</p>
            <p className="setup-recording-setting-desc">Reduces background noise before transcription.</p>
          </div>
          <button
            type="button"
            className={`setup-recording-toggle ${enableDenoise ? 'setup-recording-toggle--on' : ''}`}
            aria-label="Toggle denoise"
            aria-pressed={enableDenoise}
            onClick={() => setEnableDenoise(!enableDenoise)}
          >
            <span className="setup-recording-toggle-thumb" />
          </button>
        </div>

        <div className="setup-recording-setting-row">
          <div className="setup-recording-setting-copy">
            <p className="setup-recording-setting-title">Live Overlay</p>
            <p className="setup-recording-setting-desc">Shows a floating live transcript while you speak.</p>
          </div>
          <button
            type="button"
            className={`setup-recording-toggle ${enableOverlay ? 'setup-recording-toggle--on' : ''}`}
            aria-label="Toggle live overlay"
            aria-pressed={enableOverlay}
            onClick={() => setEnableOverlay(!enableOverlay)}
          >
            <span className="setup-recording-toggle-thumb" />
          </button>
        </div>

        <div className="setup-recording-setting-row">
          <div className="setup-recording-setting-copy">
            <p className="setup-recording-setting-title">Mute Background Audio</p>
            <p className="setup-recording-setting-desc">Mutes system playback while recording to reduce bleed-in.</p>
          </div>
          <button
            type="button"
            className={`setup-recording-toggle ${muteBackgroundAudio ? 'setup-recording-toggle--on' : ''}`}
            aria-label="Toggle mute background audio"
            aria-pressed={muteBackgroundAudio}
            onClick={() => setMuteBackgroundAudio(!muteBackgroundAudio)}
          >
            <span className="setup-recording-toggle-thumb" />
          </button>
        </div>
      </div>

      <div className="setup-nav setup-nav--spread">
        <button className="setup-btn setup-btn--ghost" onClick={onBack}>← Back</button>
        <button className="setup-btn setup-btn--primary" onClick={onNext}>Continue →</button>
      </div>
    </>
  );
}

// ─────────────────────────────────────────────────────────────────
// STEP 5 — PERMISSIONS
// ─────────────────────────────────────────────────────────────────
function StepPermissions({
  onNext,
  onBack,
  platform,
  totalSteps,
}: {
  onNext: () => void;
  onBack: () => void;
  platform: string;
  totalSteps: number;
}) {
  const isMac = platform === 'macos';

  const [micStatus, setMicStatus] = useState<string>('checking');
  const [accGranted, setAccGranted] = useState<boolean | null>(null);
  const [inputGranted, setInputGranted] = useState<boolean | null>(null);
  const [restartNeeded, setRestartNeeded] = useState(false);
  const [micRequesting, setMicRequesting] = useState(false);
  const [initialCheckDone, setInitialCheckDone] = useState(false);
  const initialAccRef = useRef<boolean | null>(null);
  const initialInputRef = useRef<boolean | null>(null);

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
    try {
      const input = await invoke<boolean>('check_input_monitoring_permission');
      setInputGranted(input);
      if (initialInputRef.current === false && input === true) {
        setRestartNeeded(true);
      }
    } catch { setInputGranted(false); }
    setInitialCheckDone(true);
  }, []);

  useEffect(() => {
    if (!isMac) return;
    invoke<boolean>('check_accessibility_permission')
      .then(v => { initialAccRef.current = v; })
      .catch(() => { initialAccRef.current = false; });
    invoke<boolean>('check_input_monitoring_permission')
      .then(v => { initialInputRef.current = v; })
      .catch(() => { initialInputRef.current = false; });
    checkStatuses();
    const timer = setInterval(checkStatuses, 1500);
    return () => clearInterval(timer);
  }, [isMac, checkStatuses]);

  const micOk = micStatus === 'granted';
  const accOk = accGranted === true;
  const inputOk = inputGranted === true;

  // Auto-advance once the initial status check confirms everything is already granted.
  // This avoids showing the permissions step at all on repeat launches.
  useEffect(() => {
    if (initialCheckDone && micOk && accOk && inputOk) {
      onNext();
    }
  }, [initialCheckDone, micOk, accOk, inputOk, onNext]);

  const requestMic = async () => {
    setMicRequesting(true);
    try { await invoke('request_microphone_permission'); } catch {}
    setMicRequesting(false);
    checkStatuses();
  };

  const requestAccessibility = async () => {
    try {
      const granted = await invoke<boolean>('request_accessibility_permission');
      if (!granted) {
        await invoke('open_accessibility_settings');
      }
    } catch {}
    checkStatuses();
  };

  const requestInputMonitoring = async () => {
    try {
      const granted = await invoke<boolean>('request_input_monitoring_permission');
      if (!granted) {
        await invoke('open_input_monitoring_settings');
      }
    } catch {}
    checkStatuses();
  };

  const openMicrophone = async () => {
    try { await invoke('open_microphone_settings'); } catch {}
  };

  const relaunchApp = async () => {
    try { await invoke('relaunch_app'); } catch {}
  };

  // While the first check is in flight, render nothing to avoid a flash of
  // the permissions UI before auto-advancing.
  if (!initialCheckDone) return null;

  return (
    <>
      <p className="setup-eyebrow">Step 6 of {totalSteps}</p>
      <h2 className="setup-heading">Permissions</h2>
      <p className="setup-sub">Three permissions needed for recording, hotkeys, and typing text system-wide.</p>

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
                  ? <button className="setup-btn setup-btn--primary perm-btn" onClick={openMicrophone}>
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
            <div className="perm-desc">Required to type transcribed text back into other apps</div>
          </div>
          <div className="perm-action">
            {accOk
              ? <span className="perm-badge perm-badge--ok">Granted</span>
              : <button className="setup-btn setup-btn--primary perm-btn" onClick={requestAccessibility}>
                  Grant Access
                </button>
            }
          </div>
        </div>

        <div className={`perm-row${inputOk ? ' perm-row--granted' : ''}`}>
          <div className="perm-icon">
            {inputOk
              ? <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
              : <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M5 12h14" /><path d="M12 5v14" /><circle cx="12" cy="12" r="10" /></svg>
            }
          </div>
          <div className="perm-info">
            <div className="perm-name">Input Monitoring</div>
            <div className="perm-desc">Required for the global hotkey to work in all apps</div>
          </div>
          <div className="perm-action">
            {inputOk
              ? <span className="perm-badge perm-badge--ok">Granted</span>
              : <button className="setup-btn setup-btn--primary perm-btn" onClick={requestInputMonitoring}>
                  Grant Access
                </button>
            }
          </div>
        </div>
      </div>

      {restartNeeded && (
        <div className="perm-restart-notice">
          <strong>Restart required.</strong> Permissions changed — restart so the hotkey and text insertion activate.
          <button className="setup-btn setup-btn--primary perm-restart-btn" onClick={relaunchApp}>
            Restart Now
          </button>
        </div>
      )}

      <div className="setup-nav setup-nav--spread">
        <button className="setup-btn setup-btn--ghost" onClick={onBack}>← Back</button>
        <button className="setup-btn setup-btn--primary" onClick={onNext}>
          {micOk && accOk && inputOk ? 'Continue →' : 'Skip for now →'}
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
  handleDownload,
  handleCancelDownload,
  downloadProgress,
  settingsModels,
}: {
  onComplete: (result: { openSettings: boolean; useCase: OnboardingUseCase }) => void;
  platform: string;
  recommendation: ReturnType<typeof computeModelRecommendation>;
  useCase: OnboardingUseCase;
  handleDownload: (id: string, name: string) => void;
  handleCancelDownload: (id: string) => void;
  downloadProgress: Record<string, DownloadProgress>;
  settingsModels: DownloadableModel[];
}) {
  const isMac = platform === 'macos';
  const comboLabel = isMac ? 'Ctrl + Option' : 'Ctrl + Win';

  const modelId = recommendation.primaryModelId;
  const modelEntry = settingsModels.find(m => m.id === modelId);
  const alreadyDownloaded = modelEntry?.downloaded === true;

  const progress = downloadProgress[modelId];
  const activeStatuses = ['starting', 'downloading', 'extracting', 'verifying', 'finalizing'];
  const isDownloading = !!progress && activeStatuses.includes(progress.status);

  const totalFiles = Math.max(1, progress?.total_files ?? 1);
  const currentFile = Math.min(Math.max(1, progress?.current_file ?? 1), totalFiles);
  const isMultiFileDownload = isDownloading && totalFiles > 1;

  const perFileProgressPct = isDownloading && progress.total > 0
    ? Math.min(100, Math.round((progress.bytes / progress.total) * 100))
    : 0;

  // For multi-file bundles (like Nemotron), show monotonic overall progress
  // so the bar does not appear to jump backwards at file boundaries.
  const progressPct = isMultiFileDownload
    ? Math.min(100, Math.round((((currentFile - 1) + (perFileProgressPct / 100)) / totalFiles) * 100))
    : perFileProgressPct;

  const progressLabel =
    progress?.status === 'verifying' ? 'Verifying…' :
    progress?.status === 'extracting' ? 'Extracting…' :
    progress?.status === 'finalizing' ? 'Finalizing…' :
    progress?.status === 'starting' ? 'Starting…' :
    `${progressPct}%`;

  const canLaunch = alreadyDownloaded;

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

      {/* ── Inline model download ───────────────────────────────── */}
      <div className="ready-download-card">
        {alreadyDownloaded ? (
          <div className="ready-download-done">
            <svg width="13" height="13" viewBox="0 0 13 13" fill="none">
              <polyline points="1.5,6.5 5,10 11.5,3" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
            <span>{recommendation.primaryLabel} — ready</span>
          </div>
        ) : isDownloading ? (
          <>
            <div className="ready-download-header">
              <span className="ready-download-name">{recommendation.primaryLabel}</span>
              <span className="ready-download-pct">{progressLabel}</span>
            </div>
            {isMultiFileDownload && (
              <div className="ready-download-bundle-indicator" role="status">
                <span>Bundle files</span>
                <span>File {currentFile} / {totalFiles}</span>
              </div>
            )}
            <div className="ready-download-bar">
              <div
                className="ready-download-bar-fill"
                style={{ width: `${progressPct}%` }}
              />
            </div>
            <button
              className="ready-download-cancel"
              onClick={() => handleCancelDownload(modelId)}
            >
              Cancel
            </button>
          </>
        ) : (
          <>
            <p className="ready-download-prompt">
              Download your recommended model to start recording immediately.
            </p>
            <button
              className="setup-btn setup-btn--primary setup-btn--full"
              onClick={() => handleDownload(modelId, recommendation.primaryLabel)}
            >
              Download {recommendation.primaryLabel}
              {modelEntry?.size ? <span className="ready-download-size"> · {modelEntry.size}</span> : null}
            </button>
          </>
        )}
      </div>

      <div className="setup-nav--ready">
        <button
          className="setup-btn setup-btn--primary setup-btn--full"
          disabled={!canLaunch}
          style={!canLaunch ? { opacity: 0.35, cursor: 'not-allowed' } : undefined}
          onClick={() => canLaunch && onComplete({ openSettings: false, useCase })}
        >
          Launch App →
        </button>
        <button
          className="ready-skip-btn"
          onClick={() => onComplete({ openSettings: !alreadyDownloaded, useCase })}
        >
          {alreadyDownloaded ? 'Open Models tab instead' : 'Skip — set up models manually'}
        </button>
      </div>
    </>
  );
}
