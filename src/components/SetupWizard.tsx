import React, { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { TitleBar } from './TitleBar';
import { Logo } from './Logo';
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

type SetupEngineId = 'whisper' | 'granite' | 'qwen3';

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
    subtitle: 'The all-rounder: 99 languages, runs on any computer.',
    goodAt: [
      'Mixed or non-English audio, accents and code-switching.',
      'Older or low-memory machines (small quantized versions).',
      'Macs: the encoder runs on the Neural Engine for speed.',
    ],
    usage: [
      'Base or Small for daily dictation; Large V3 Turbo for hard audio.',
      'Pick the English-only version if you only speak English.',
      'Quantized versions download faster and use less memory.',
    ],
  },
  {
    id: 'granite',
    title: 'Granite Speech 5',
    subtitle: 'The fastest English model: a 30-second note in a blink.',
    goodAt: [
      'English dictation where speed matters most.',
      'Quick notes, messages and coding flow.',
      'Laptops without a strong GPU (fast even on the CPU).',
    ],
    usage: [
      'Writes plain lowercase text; turn on FlowScribe for punctuation and capitals.',
      'English only; use Whisper or Qwen3 for other languages.',
      'IBM weights for non-commercial use.',
    ],
  },
  {
    id: 'qwen3',
    title: 'Qwen3-ASR',
    subtitle: 'The most accurate: punctuation and casing built in.',
    goodAt: [
      'Meetings, interviews and files where every word counts.',
      'Multilingual audio and difficult recordings.',
      'Finished text straight away, no clean-up pass needed.',
    ],
    usage: [
      '1.7B needs about 5 GB of memory; choose 0.6B on smaller machines.',
      'Best with a GPU (Metal, CUDA or Vulkan); slower on CPU only.',
      'For quick English notes, Granite is lighter.',
    ],
  },
];


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
  const useCase: OnboardingUseCase = 'quick_notes';
  const [current, setCurrent] = useState<StepEntry>({ idx: 0, enterDir: 'right', key: 0 });
  const [exiting, setExiting] = useState<{ idx: number; exitDir: 'left' | 'right'; key: number } | null>(null);
  const transitioning = useRef(false);
  const recommendation = computeModelRecommendation({ sysInfo, isAppleSilicon, useCase });
  const totalSteps = platform === 'macos' ? 9 : 8;

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
      case 1: return <StepHardware sysInfo={sysInfo} platform={platform} onNext={next} onBack={back} totalSteps={totalSteps} />;
      case 2: return (
        <StepEngines
          onNext={next}
          onBack={back}
          totalSteps={totalSteps}
        />
      );
      case 3: return <StepFeatures onNext={next} onBack={back} totalSteps={totalSteps} platform={platform} />;
      case 4: return <StepFlowScribe onNext={next} onBack={back} totalSteps={totalSteps} />;
      case 5: return <StepHotkey onNext={next} onBack={back} platform={platform} totalSteps={totalSteps} />;
      case 6: return (
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
      case 7:
        // Skip permissions step on non-macOS platforms
        if (platform !== 'macos') {
          return <StepReady onComplete={onComplete} platform={platform} recommendation={recommendation} useCase={useCase} handleDownload={handleDownload} handleCancelDownload={handleCancelDownload} downloadProgress={downloadProgress} settingsModels={settingsModels} />;
        }
        return <StepPermissions onNext={next} onBack={back} platform={platform} totalSteps={totalSteps} />;
      case 8: return <StepReady onComplete={onComplete} platform={platform} recommendation={recommendation} useCase={useCase} handleDownload={handleDownload} handleCancelDownload={handleCancelDownload} downloadProgress={downloadProgress} settingsModels={settingsModels} />;
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
function StepWelcome({ onNext }: { onNext: () => void }) {
  return (
    <>
      <div className="welcome-mark">
        <span className="welcome-mark-glow" aria-hidden="true" />
        <Logo size={88} animate />
      </div>
      <h1 className="welcome-logo">Taurscribe</h1>
      <p className="welcome-tagline">Private speech-to-text that runs on your computer</p>

      <ul className="welcome-features">
        {[
          'Works offline: your audio never leaves this machine',
          'Dictate into any app with one hotkey',
          'Records meetings and tells the speakers apart',
        ].map((text) => (
          <li className="welcome-feature" key={text}>
            <svg className="welcome-feature-check" width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
              <path d="M2.5 7.5l3 3 6-7" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
            {text}
          </li>
        ))}
      </ul>

      <div className="setup-nav">
        <button
          type="button"
          id="wizard-welcome-begin-btn"
          data-testid="wizard-welcome-begin-btn"
          className="setup-btn setup-btn--primary"
          onClick={onNext}
          aria-label="Begin setup"
        >
          Get started →
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
      return <p className="hw-verdict"><strong>GPU acceleration ready.</strong> Whisper and Granite both run at full speed.</p>;
    }
    if (isMac && sysInfo.backend_hint === 'Metal') {
      return <p className="hw-verdict"><strong>GPU acceleration ready.</strong> Models run on the GPU through Metal.</p>;
    }
    if (hasGpu) {
      return <p className="hw-verdict"><strong className="amber">GPU detected (no CUDA).</strong> Whisper via CPU — consider downloading a smaller model.</p>;
    }
    return <p className="hw-verdict">No GPU detected. Transcription will use the CPU — choose a small Whisper model for best performance.</p>;
  };

  return (
    <>
      <p className="setup-eyebrow">Step 2 of {totalSteps}</p>
      <h2 className="setup-heading">Your hardware</h2>
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
            <span className="hw-value">{hasGpu ? sysInfo!.gpu_name : 'Not detected'}</span>
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
        <button
          type="button"
          id="wizard-hardware-back-btn"
          data-testid="wizard-hardware-back-btn"
          className="setup-btn setup-btn--ghost"
          onClick={onBack}
          aria-label="Back to welcome step"
        >← Back</button>
        <button
          type="button"
          id="wizard-hardware-next-btn"
          data-testid="wizard-hardware-next-btn"
          className="setup-btn setup-btn--primary"
          onClick={onNext}
          disabled={loading}
          aria-label="Continue to engines step"
        >
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
  const [visitedSlides, setVisitedSlides] = useState<boolean[]>(() =>
    ENGINE_CAROUSEL_SLIDES.map((_, index) => index === 0),
  );
  const slide = ENGINE_CAROUSEL_SLIDES[activeSlide];
  const viewedCount = visitedSlides.filter(Boolean).length;
  const hasViewedAllSlides = viewedCount === ENGINE_CAROUSEL_SLIDES.length;

  useEffect(() => {
    setVisitedSlides((prev) => {
      if (prev[activeSlide]) {
        return prev;
      }
      const nextVisited = [...prev];
      nextVisited[activeSlide] = true;
      return nextVisited;
    });
  }, [activeSlide]);

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
      <h2 className="setup-heading">Meet the engines</h2>
      <p className="setup-sub">Swipe through each engine to learn where it shines and when to use it. Continue unlocks after all cards are viewed.</p>

      <div className="setup-engine-carousel" aria-live="polite">
        <div className={`setup-engine-carousel-card setup-engine-carousel-card--${slide.id}`}>
          <div className={`setup-engine-carousel-bg setup-engine-carousel-bg--whisper${slide.id === 'whisper' ? ' is-active' : ''}`} />
          <div className={`setup-engine-carousel-bg setup-engine-carousel-bg--granite${slide.id === 'granite' ? ' is-active' : ''}`} />
          <div className={`setup-engine-carousel-bg setup-engine-carousel-bg--qwen3${slide.id === 'qwen3' ? ' is-active' : ''}`} />

          <div key={`${slide.id}-${activeSlide}`} className={`setup-engine-carousel-content setup-engine-carousel-content--${navDirection}`}>
            <div className="setup-engine-carousel-topline">
              <span className={`setup-engine-chip setup-engine-chip--${slide.id}`}>{slide.title}</span>
              <span className="setup-engine-slide-index">{activeSlide + 1} / {ENGINE_CAROUSEL_SLIDES.length}</span>
            </div>
            <p className="setup-engine-carousel-subtitle">{slide.subtitle}</p>

            <div className="setup-engine-carousel-grid">
              <div className="setup-engine-carousel-column">
                <p className="setup-engine-column-title">Good at</p>
                <ul className="setup-engine-list">
                  {slide.goodAt.map((item) => (
                    <li key={item}>{item}</li>
                  ))}
                </ul>
              </div>
              <div className="setup-engine-carousel-column">
                <p className="setup-engine-column-title">How to use</p>
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
          <button
            type="button"
            id="wizard-engine-prev-btn"
            data-testid="wizard-engine-prev-btn"
            className="setup-engine-carousel-btn"
            onClick={goPrev}
            aria-label="Previous engine slide"
          >
            ← Prev
          </button>
          <div
            id="wizard-engine-carousel-dots"
            data-testid="wizard-engine-carousel-dots"
            className="setup-engine-carousel-dots"
            role="tablist"
            aria-label="Engine slides"
          >
            {ENGINE_CAROUSEL_SLIDES.map((engine, index) => (
              <button
                key={engine.id}
                type="button"
                id={`wizard-engine-dot-${engine.id}`}
                data-testid={`wizard-engine-dot-${engine.id}`}
                role="tab"
                aria-selected={index === activeSlide}
                aria-label={`Engine slide: ${engine.title}`}
                className={`setup-engine-carousel-dot${index === activeSlide ? ' setup-engine-carousel-dot--active' : ''}`}
                onClick={() => goToSlide(index, index >= activeSlide ? 'next' : 'prev')}
                title={engine.title}
              />
            ))}
          </div>
          <button
            type="button"
            id="wizard-engine-next-btn"
            data-testid="wizard-engine-next-btn"
            className="setup-engine-carousel-btn"
            onClick={goNext}
            aria-label="Next engine slide"
          >
            Next →
          </button>
        </div>
      </div>

      <p className="engines-note">
        {hasViewedAllSlides
          ? 'All engines reviewed. You can continue.'
          : `Review every engine card to continue (${viewedCount}/${ENGINE_CAROUSEL_SLIDES.length}).`}
      </p>

      <div className="setup-nav setup-nav--spread">
        <button
          type="button"
          id="wizard-engines-back-btn"
          data-testid="wizard-engines-back-btn"
          className="setup-btn setup-btn--ghost"
          onClick={onBack}
          aria-label="Back to hardware step"
        >← Back</button>
        <button
          type="button"
          id="wizard-engines-next-btn"
          data-testid="wizard-engines-next-btn"
          className="setup-btn setup-btn--primary"
          onClick={onNext}
          disabled={!hasViewedAllSlides}
          aria-label="Continue to features step"
        >Continue →</button>
      </div>
    </>
  );
}

// ─────────────────────────────────────────────────────────────────
// STEP 3 — WHAT IT DOES
// ─────────────────────────────────────────────────────────────────
const FEATURE_CARDS: Array<{ id: string; title: string; body: string; icon: React.ReactNode }> = [
  {
    id: 'meetings',
    title: 'Meetings, no bot',
    body: 'Spots Zoom, Meet, Teams, Slack and Discord calls and records you and the call on separate channels. Nothing joins the call.',
    icon: <path d="M3 7h11a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H3zM16 11l5-3v8l-5-3" />,
  },
  {
    id: 'speakers',
    title: 'Who said what',
    body: 'Separates up to 8 people on a call and remembers the ones you name, so they are recognised in later meetings.',
    icon: <><circle cx="8" cy="8" r="3" /><circle cx="17" cy="9" r="2.5" /><path d="M2.5 19c.8-3 3-4.5 5.5-4.5s4.7 1.5 5.5 4.5M14 18.5c.5-2 1.8-3 3.5-3s3 1 3.5 3" /></>,
  },
  {
    id: 'files',
    title: 'Audio files',
    body: 'Drop recordings in the Files tab and pick the model for each one. Transcripts are saved alongside your dictations.',
    icon: <path d="M6 3h8l4 4v14H6zM14 3v4h4M9 13h6M9 17h6" />,
  },
  {
    id: 'tray',
    title: 'Always at a glance',
    body: 'The menu-bar icon shows what Taurscribe is doing: recording, processing, a detected call, with details on hover.',
    icon: <><rect x="3" y="4" width="18" height="4" rx="1.5" /><circle cx="17" cy="6" r="0.9" fill="currentColor" /><path d="M7 12h10M7 16h6" /></>,
  },
  {
    id: 'llm',
    title: 'Ask your AI',
    body: 'Optionally let Claude, ChatGPT or Cursor search your transcripts (read-only). Off until you turn it on.',
    icon: <path d="M12 3l1.8 4.7L18.5 9.5l-4.7 1.8L12 16l-1.8-4.7L5.5 9.5l4.7-1.8zM18 15l.9 2.1L21 18l-2.1.9L18 21l-.9-2.1L15 18l2.1-.9z" />,
  },
  {
    id: 'private',
    title: 'Stays on this computer',
    body: 'Every model runs locally. Audio and transcripts never leave your machine unless you choose to share them.',
    icon: <path d="M12 3l7 3v5c0 4.5-3 8.3-7 10-4-1.7-7-5.5-7-10V6zM9 12l2 2 4-4" />,
  },
];

function StepFeatures({ onNext, onBack, totalSteps, platform }: { onNext: () => void; onBack: () => void; totalSteps: number; platform: string }) {
  const trayWord = platform === 'macos' ? 'menu-bar' : 'tray';
  return (
    <>
      <p className="setup-eyebrow">Step 4 of {totalSteps}</p>
      <h2 className="setup-heading">More than dictation</h2>
      <p className="setup-sub">Everything below works out of the box and runs entirely on your computer.</p>

      <div className="setup-feature-grid">
        {FEATURE_CARDS.map((f, i) => (
          <div className={`setup-feature-card setup-feature-card--${f.id}`} key={f.id} style={{ animationDelay: `${80 + i * 55}ms` }}>
            <svg className="setup-feature-icon" width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
              {f.icon}
            </svg>
            <div>
              <div className="setup-feature-title">{f.title}</div>
              <p className="setup-feature-body">{f.id === 'tray' ? f.body.replace('menu-bar', trayWord) : f.body}</p>
            </div>
          </div>
        ))}
      </div>

      <div className="setup-nav setup-nav--spread">
        <button
          type="button"
          id="wizard-features-back-btn"
          data-testid="wizard-features-back-btn"
          className="setup-btn setup-btn--ghost"
          onClick={onBack}
          aria-label="Back to engines step"
        >← Back</button>
        <button
          type="button"
          id="wizard-features-next-btn"
          data-testid="wizard-features-next-btn"
          className="setup-btn setup-btn--primary"
          onClick={onNext}
          aria-label="Continue to FlowScribe step"
        >Continue →</button>
      </div>
    </>
  );
}

// ─────────────────────────────────────────────────────────────────
// STEP 4 — FLOWSCRIBE LLM
// ─────────────────────────────────────────────────────────────────
function StepFlowScribe({
  onNext,
  onBack,
  totalSteps,
}: {
  onNext: () => void;
  onBack: () => void;
  totalSteps: number;
}) {
  return (
    <>
      <p className="setup-eyebrow">Step 5 of {totalSteps}</p>
      <h2 className="setup-heading">FlowScribe V3 <span className="setup-beta">Beta</span></h2>
      <p className="setup-sub">FlowScribe is a small on-device model that turns your raw transcript into what you meant: fillers and corrections removed, numbers and emails written properly.</p>

      <div className="fs-processor-rack" aria-label="Example of FlowScribe cleaning a transcript">
        <div className="fs-rack-headers">
          <div className="fs-rack-brand">TAURSCRIBE DSP // FLOWSCRIBE V3 0.8B</div>
          <div className="fs-rack-status">
            <span className="fs-rack-led fs-rack-led--active"></span> ONLINE
          </div>
        </div>

        <div className="fs-rack-io">
          <div className="fs-rack-panel fs-rack-input">
            <div className="fs-panel-label">CH 01 / RAW ASR</div>
            <div className="fs-panel-screen">
              &gt; um hey team can we ship this on thursday no wait friday i think we should test the the payment edge cases first
            </div>
          </div>

          <div className="fs-rack-center">
            <div className="fs-process-steps">
              <div className="fs-p-step">FILLERS</div>
              <div className="fs-p-step">CORRECTIONS</div>
              <div className="fs-p-step">PUNCTUATION</div>
            </div>
            <div className="fs-process-arrows" aria-hidden="true">
              <span className="fs-process-chevron" />
              <span className="fs-process-chevron" />
              <span className="fs-process-chevron" />
            </div>
          </div>

          <div className="fs-rack-panel fs-rack-output">
            <div className="fs-panel-label">CH 02 / POLISHED</div>
            <div className="fs-panel-screen">
              &gt; Hey team, can we ship this Friday? I think we should test payment edge cases first.
            </div>
          </div>
        </div>
      </div>

      <div className="fs-rack-specs">
        <div className="fs-spec-item">
          <span className="fs-spec-num">01</span>
          <div className="fs-spec-content">
            <span className="fs-spec-title">100% LOCAL</span>
            <span className="fs-spec-desc">Runs entirely on-device. Zero cloud telemetry.</span>
          </div>
        </div>
        <div className="fs-spec-item">
          <span className="fs-spec-num">02</span>
          <div className="fs-spec-content">
            <span className="fs-spec-title">COGNITIVE PASS</span>
            <span className="fs-spec-desc">Fixes structural errors the ASR misses.</span>
          </div>
        </div>
        <div className="fs-spec-item">
          <span className="fs-spec-num">03</span>
          <div className="fs-spec-content">
            <span className="fs-spec-title">OPTIONAL</span>
            <span className="fs-spec-desc">Can be fully bypassed in Quick Settings.</span>
          </div>
        </div>
      </div>

      <div className="setup-nav setup-nav--spread">
        <button
          type="button"
          id="wizard-flowscribe-back-btn"
          data-testid="wizard-flowscribe-back-btn"
          className="setup-btn setup-btn--ghost"
          onClick={onBack}
          aria-label="Back to features step"
        >← Back</button>
        <button
          type="button"
          id="wizard-flowscribe-next-btn"
          data-testid="wizard-flowscribe-next-btn"
          className="setup-btn setup-btn--primary"
          onClick={onNext}
          aria-label="Continue to hotkey step"
        >Continue →</button>
      </div>
    </>
  );
}

// ─────────────────────────────────────────────────────────────────
// STEP 4 — HOTKEY
// ─────────────────────────────────────────────────────────────────
function StepHotkey({ onNext, onBack, platform, totalSteps }: { onNext: () => void; onBack: () => void; platform: string; totalSteps: number }) {
  // macOS default: Ctrl + Option (Cmd is intercepted by the OS)
  // Windows/Linux default: Ctrl + Win/Super
  const isMac = platform === 'macos';
  const modifierLabel = isMac ? 'Option' : 'Win';
  const comboLabel = `Ctrl + ${modifierLabel}`;

  return (
    <>
      <p className="setup-eyebrow">Step 6 of {totalSteps}</p>
      <h2 className="setup-heading">One hotkey</h2>
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
        <button
          type="button"
          id="wizard-hotkey-back-btn"
          data-testid="wizard-hotkey-back-btn"
          className="setup-btn setup-btn--ghost"
          onClick={onBack}
          aria-label="Back to FlowScribe step"
        >← Back</button>
        <button
          type="button"
          id="wizard-hotkey-next-btn"
          data-testid="wizard-hotkey-next-btn"
          className="setup-btn setup-btn--primary"
          onClick={onNext}
          aria-label="Continue to recording settings step"
        >Continue →</button>
      </div>
    </>
  );
}

// ─────────────────────────────────────────────────────────────────
// STEP 5 — RECORDING SETTINGS
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
      <p className="setup-eyebrow">Step 7 of {totalSteps}</p>
      <h2 className="setup-heading">Recording settings</h2>
      <p className="setup-sub">Set your default behavior now. You can change these anytime in Settings.</p>

      <div className="setup-recording-settings-grid">
        <div className="setup-recording-setting-row">
          <div className="setup-recording-setting-copy">
            <p className="setup-recording-setting-title">Denoise</p>
            <p className="setup-recording-setting-desc">Reduces background noise before transcription.</p>
          </div>
          <button
            type="button"
            id="wizard-toggle-denoise"
            data-testid="wizard-toggle-denoise"
            role="switch"
            className={`setup-recording-toggle ${enableDenoise ? 'setup-recording-toggle--on' : ''}`}
            aria-label="Toggle denoise"
            aria-checked={enableDenoise}
            onClick={() => setEnableDenoise(!enableDenoise)}
          >
            <span className="setup-recording-toggle-thumb" />
          </button>
        </div>

        <div className="setup-recording-setting-row">
          <div className="setup-recording-setting-copy">
            <p className="setup-recording-setting-title">Recording overlay</p>
            <p className="setup-recording-setting-desc">Shows a small capsule with a live waveform and timer while you dictate.</p>
          </div>
          <button
            type="button"
            id="wizard-toggle-overlay"
            data-testid="wizard-toggle-overlay"
            role="switch"
            className={`setup-recording-toggle ${enableOverlay ? 'setup-recording-toggle--on' : ''}`}
            aria-label="Toggle live overlay"
            aria-checked={enableOverlay}
            onClick={() => setEnableOverlay(!enableOverlay)}
          >
            <span className="setup-recording-toggle-thumb" />
          </button>
        </div>

        <div className="setup-recording-setting-row">
          <div className="setup-recording-setting-copy">
            <p className="setup-recording-setting-title">Mute background audio</p>
            <p className="setup-recording-setting-desc">Mutes system playback while recording to reduce bleed-in.</p>
          </div>
          <button
            type="button"
            id="wizard-toggle-mute-bg"
            data-testid="wizard-toggle-mute-bg"
            role="switch"
            className={`setup-recording-toggle ${muteBackgroundAudio ? 'setup-recording-toggle--on' : ''}`}
            aria-label="Toggle mute background audio"
            aria-checked={muteBackgroundAudio}
            onClick={() => setMuteBackgroundAudio(!muteBackgroundAudio)}
          >
            <span className="setup-recording-toggle-thumb" />
          </button>
        </div>
      </div>

      <div className="setup-nav setup-nav--spread">
        <button
          type="button"
          id="wizard-recording-back-btn"
          data-testid="wizard-recording-back-btn"
          className="setup-btn setup-btn--ghost"
          onClick={onBack}
          aria-label="Back to hotkey step"
        >← Back</button>
        <button
          type="button"
          id="wizard-recording-next-btn"
          data-testid="wizard-recording-next-btn"
          className="setup-btn setup-btn--primary"
          onClick={onNext}
          aria-label="Continue to permissions step"
        >Continue →</button>
      </div>
    </>
  );
}

// ─────────────────────────────────────────────────────────────────
// STEP 6 — PERMISSIONS
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
      <p className="setup-eyebrow">Step 8 of {totalSteps}</p>
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
              ? <span id="wizard-perm-mic-ok" data-testid="wizard-perm-mic-ok" className="perm-badge perm-badge--ok" role="status">Granted</span>
              : micStatus === 'restricted'
                ? <span id="wizard-perm-mic-restricted" data-testid="wizard-perm-mic-restricted" className="perm-badge perm-badge--denied" role="status">Restricted by policy</span>
                : micStatus === 'denied'
                  ? <button
                      type="button"
                      id="wizard-perm-mic-settings-btn"
                      data-testid="wizard-perm-mic-settings-btn"
                      className="setup-btn setup-btn--primary perm-btn"
                      onClick={openMicrophone}
                      aria-label="Open Microphone Settings"
                    >
                      Open Settings
                    </button>
                  : <button
                      type="button"
                      id="wizard-perm-mic-grant-btn"
                      data-testid="wizard-perm-mic-grant-btn"
                      className="setup-btn setup-btn--primary perm-btn"
                      onClick={requestMic}
                      disabled={micRequesting}
                      aria-label="Grant Microphone Access"
                    >
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
              ? <span id="wizard-perm-acc-ok" data-testid="wizard-perm-acc-ok" className="perm-badge perm-badge--ok" role="status">Granted</span>
              : <button
                  type="button"
                  id="wizard-perm-accessibility-grant-btn"
                  data-testid="wizard-perm-accessibility-grant-btn"
                  className="setup-btn setup-btn--primary perm-btn"
                  onClick={requestAccessibility}
                  aria-label="Grant Accessibility Access"
                >
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
              ? <span id="wizard-perm-input-ok" data-testid="wizard-perm-input-ok" className="perm-badge perm-badge--ok" role="status">Granted</span>
              : <button
                  type="button"
                  id="wizard-perm-input-grant-btn"
                  data-testid="wizard-perm-input-grant-btn"
                  className="setup-btn setup-btn--primary perm-btn"
                  onClick={requestInputMonitoring}
                  aria-label="Grant Input Monitoring Access"
                >
                  Grant Access
                </button>
            }
          </div>
        </div>
      </div>

      {restartNeeded && (
        <div
          id="wizard-perm-restart-notice"
          data-testid="wizard-perm-restart-notice"
          className="perm-restart-notice"
          role="alert"
        >
          <strong>Restart required.</strong> Permissions changed — restart so the hotkey and text insertion activate.
          <button
            type="button"
            id="wizard-perm-restart-btn"
            data-testid="wizard-perm-restart-btn"
            className="setup-btn setup-btn--primary perm-restart-btn"
            onClick={relaunchApp}
            aria-label="Restart Application Now"
          >
            Restart Now
          </button>
        </div>
      )}

      <div className="setup-nav setup-nav--spread">
        <button
          type="button"
          id="wizard-perm-back-btn"
          data-testid="wizard-perm-back-btn"
          className="setup-btn setup-btn--ghost"
          onClick={onBack}
          aria-label="Back to previous step"
        >← Back</button>
        <button
          type="button"
          id="wizard-perm-next-btn"
          data-testid="wizard-perm-next-btn"
          className="setup-btn setup-btn--primary"
          onClick={onNext}
          aria-label={micOk && accOk && inputOk ? 'Continue to next step' : 'Skip permissions for now'}
        >
          {micOk && accOk && inputOk ? 'Continue →' : 'Skip for now →'}
        </button>
      </div>
    </>
  );
}

// ─────────────────────────────────────────────────────────────────
// FINAL — READY
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
      <h2 className="setup-heading">You're all set</h2>

      <ul className="ready-checks">
        {[
          'Hardware detected and configured',
          `Starting profile tuned for ${recommendation.useCaseLabel.toLowerCase()}`,
          `Recommended engine: ${recommendation.primaryEngineLabel}`,
          `Global hotkey active: ${comboLabel}`,
          'Pastes directly into any app',
          'Meetings detected automatically, speakers separated',
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
              <div
                id="wizard-ready-bundle-indicator"
                data-testid="wizard-ready-bundle-indicator"
                className="ready-download-bundle-indicator"
                role="status"
              >
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
              type="button"
              id="wizard-ready-cancel-download-btn"
              data-testid="wizard-ready-cancel-download-btn"
              className="ready-download-cancel"
              onClick={() => handleCancelDownload(modelId)}
              aria-label="Cancel model download"
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
              type="button"
              id="wizard-ready-download-model-btn"
              data-testid="wizard-ready-download-model-btn"
              className="setup-btn setup-btn--primary setup-btn--full"
              onClick={() => handleDownload(modelId, recommendation.primaryLabel)}
              aria-label={`Download ${recommendation.primaryLabel}`}
            >
              Download {recommendation.primaryLabel}
              {modelEntry?.size ? <span className="ready-download-size"> · {modelEntry.size}</span> : null}
            </button>
          </>
        )}
      </div>

      <div className="setup-nav--ready">
        <button
          type="button"
          id="wizard-ready-launch-btn"
          data-testid="wizard-ready-launch-btn"
          className="setup-btn setup-btn--primary setup-btn--full"
          disabled={!canLaunch}
          style={!canLaunch ? { opacity: 0.35, cursor: 'not-allowed' } : undefined}
          onClick={() => canLaunch && onComplete({ openSettings: false, useCase })}
          aria-label="Launch Taurscribe Application"
        >
          Launch App →
        </button>
        <button
          type="button"
          id="wizard-ready-skip-btn"
          data-testid="wizard-ready-skip-btn"
          className="ready-skip-btn"
          onClick={() => onComplete({ openSettings: !alreadyDownloaded, useCase })}
          aria-label={alreadyDownloaded ? 'Open Models tab instead' : 'Skip and set up models manually'}
        >
          {alreadyDownloaded ? 'Open Models tab instead' : 'Skip — set up models manually'}
        </button>
      </div>
    </>
  );
}
