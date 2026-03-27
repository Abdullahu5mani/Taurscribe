import { useState, useEffect, useMemo, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Store } from '@tauri-apps/plugin-store';
import type { DownloadableModel, DownloadProgress } from './types';
import { ModelRow } from './ModelRow';
import {
    computeModelRecommendation,
    getWhisperTierFromModelId,
    type OnboardingUseCase,
    type SystemInfo,
} from '../../modelRecommendations';

type WhisperTier = 'Tiny' | 'Base' | 'Small' | 'Medium' | 'Large';

const TIERS: WhisperTier[] = ['Tiny', 'Base', 'Small', 'Medium', 'Large'];

const TIER_DESCRIPTIONS: Record<WhisperTier, string> = {
    Tiny: 'Fastest · lowest accuracy · great for quick dictation on any hardware',
    Base: 'Good balance of speed and accuracy · solid starting point',
    Small: 'High accuracy · reasonable speed · best all-rounder',
    Medium: 'Very high accuracy · slower · needs 8 GB RAM',
    Large: 'Best possible accuracy · slowest · requires 10 GB+ RAM/VRAM',
};

const TIER_MODEL_IDS: Record<WhisperTier, string[]> = {
    Tiny: ['whisper-tiny-en-q5_1', 'whisper-tiny-en', 'whisper-tiny-q5_1', 'whisper-tiny'],
    Base: ['whisper-base-en-q5_1', 'whisper-base-en', 'whisper-base-q5_1', 'whisper-base'],
    Small: ['whisper-small-en-q5_1', 'whisper-small-en', 'whisper-small-q5_1', 'whisper-small'],
    Medium: ['whisper-medium-en-q5_0', 'whisper-medium-en', 'whisper-medium-q5_0', 'whisper-medium'],
    Large: ['whisper-large-v3-turbo-q5_0', 'whisper-large-v3-turbo', 'whisper-large-v3-q5_0', 'whisper-large-v3'],
};

const TIER_RECOMMENDED: Record<WhisperTier, string> = {
    Tiny: 'whisper-tiny-en-q5_1',
    Base: 'whisper-base-en-q5_1',
    Small: 'whisper-small-en-q5_1',
    Medium: 'whisper-medium-en-q5_0',
    Large: 'whisper-large-v3-turbo-q5_0',
};

// On Apple Silicon, full-precision models pair with CoreML encoders for best performance.
const TIER_RECOMMENDED_ANS: Record<WhisperTier, string> = {
    Tiny: 'whisper-tiny-en',
    Base: 'whisper-base-en',
    Small: 'whisper-small-en',
    Medium: 'whisper-medium-en',
    Large: 'whisper-large-v3-turbo',
};

const TIER_COREML_IDS: Record<WhisperTier, string[]> = {
    Tiny: ['whisper-tiny-en-coreml', 'whisper-tiny-coreml'],
    Base: ['whisper-base-en-coreml', 'whisper-base-coreml'],
    Small: ['whisper-small-en-coreml', 'whisper-small-coreml'],
    Medium: ['whisper-medium-en-coreml', 'whisper-medium-coreml'],
    Large: ['whisper-large-v3-turbo-coreml', 'whisper-large-v3-coreml'],
};

interface ModelsTabProps {
    models: DownloadableModel[];
    downloadProgress: Record<string, DownloadProgress>;
    onDownload: (id: string, name: string) => void;
    onDelete: (id: string, name: string) => Promise<void>;
    onCancelDownload: (id: string) => void;
    scrollTarget?: string;
    onScrollHandled?: () => void;
}

export function ModelsTab({ models, downloadProgress, onDownload, onDelete, onCancelDownload, scrollTarget, onScrollHandled }: ModelsTabProps) {
    const [activeTier, setActiveTier] = useState<WhisperTier>('Small');
    const [platform, setPlatform] = useState('');
    const [isAppleSilicon, setIsAppleSilicon] = useState(false);
    const [sysInfo, setSysInfo] = useState<SystemInfo | null>(null);
    const [useCase, setUseCase] = useState<OnboardingUseCase>('quick_notes');
    const hydratedTierRef = useRef(false);
    const [pulseModelIds, setPulseModelIds] = useState<Set<string>>(new Set());
    const whisperGroupRef = useRef<HTMLDivElement>(null);
    const parakeetGroupRef = useRef<HTMLDivElement>(null);
    const graniteGroupRef = useRef<HTMLDivElement>(null);

    useEffect(() => {
        invoke<string>('get_platform').then(setPlatform).catch(() => { });
        invoke<boolean>('is_apple_silicon').then(setIsAppleSilicon).catch(() => { });
        invoke<SystemInfo>('get_system_info').then(setSysInfo).catch(() => { });
        Store.load('settings.json')
            .then((store) => store.get<OnboardingUseCase>('onboarding_use_case'))
            .then((savedUseCase) => {
                if (savedUseCase) {
                    setUseCase(savedUseCase);
                }
            })
            .catch(() => { });
    }, []);

    const isMac = platform === 'macos';
    const isWindows = platform === 'windows';
    const rowProps = { downloadProgress, onDownload, onDelete, onCancelDownload };
    const recommendation = useMemo(
        () => computeModelRecommendation({ sysInfo, isAppleSilicon, useCase }),
        [sysInfo, isAppleSilicon, useCase],
    );

    const parakeetModels = models.filter(m => m.type === 'Parakeet');
    const graniteModels = models.filter(
        m => m.type === 'GraniteSpeech'
            && (!m.macosOnly || isMac)
            && (!m.windowsOnly || isWindows),
    );
    const llmModels = models.filter(m => m.type === 'LLM');
    const coremlModels = models.filter(m => m.type === 'CoreML');

    useEffect(() => {
        if (hydratedTierRef.current) return;
        const preferredTier =
            recommendation.whisperTier ??
            getWhisperTierFromModelId(recommendation.primaryModelId) ??
            getWhisperTierFromModelId(recommendation.backupModelId);
        if (preferredTier) {
            setActiveTier(preferredTier);
            hydratedTierRef.current = true;
        }
    }, [recommendation]);

    // Scroll to the target engine section and pulse the first downloadable model
    useEffect(() => {
        if (!scrollTarget) return;

        let groupRef: React.RefObject<HTMLDivElement | null>;
        let targetModelId: string | undefined;

        if (scrollTarget === 'whisper') {
            groupRef = whisperGroupRef;
            // Resolve the best tier from the recommendation so the right tab is active
            const preferredTier: WhisperTier =
                recommendation.whisperTier ??
                getWhisperTierFromModelId(recommendation.primaryModelId) ??
                getWhisperTierFromModelId(recommendation.backupModelId) ??
                activeTier;
            setActiveTier(preferredTier);
            const recId = isAppleSilicon ? TIER_RECOMMENDED_ANS[preferredTier] : TIER_RECOMMENDED[preferredTier];
            targetModelId = models.find(m => m.id === recId && !m.downloaded)?.id
                ?? TIER_MODEL_IDS[preferredTier].map(id => models.find(m => m.id === id && !m.downloaded)).find(Boolean)?.id;
        } else if (scrollTarget === 'parakeet') {
            groupRef = parakeetGroupRef;
            targetModelId = models.find(m => m.type === 'Parakeet' && !m.downloaded)?.id;
        } else if (scrollTarget === 'granite_speech') {
            groupRef = graniteGroupRef;
            targetModelId = models.find(m => m.type === 'GraniteSpeech' && !m.downloaded)?.id;
        } else {
            return;
        }

        const timer = setTimeout(() => {
            const el = groupRef.current;
            if (el) {
                const container = el.closest('.settings-content') as HTMLElement | null;
                if (container) {
                    const start = container.scrollTop;
                    const target = el.getBoundingClientRect().top
                        - container.getBoundingClientRect().top
                        + container.scrollTop
                        - 16;
                    const distance = target - start;
                    const duration = 900;
                    const t0 = performance.now();
                    const step = (now: number) => {
                        const p = Math.min((now - t0) / duration, 1);
                        // ease-in-out cubic
                        const e = p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2;
                        container.scrollTop = start + distance * e;
                        if (p < 1) requestAnimationFrame(step);
                    };
                    requestAnimationFrame(step);
                } else {
                    el.scrollIntoView({ behavior: 'smooth', block: 'start' });
                }
            }
            if (targetModelId) {
                const ids = new Set([targetModelId]);
                setPulseModelIds(ids);
                setTimeout(() => setPulseModelIds(new Set()), 7000);
            }
            onScrollHandled?.();
        }, 300);

        return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [scrollTarget, recommendation]);

    const tierModels = (() => {
        const list = TIER_MODEL_IDS[activeTier]
            .map(id => models.find(m => m.id === id))
            .filter((m): m is DownloadableModel => m !== undefined);
        if (isAppleSilicon) {
            // Put full-precision (CoreML-capable) models first so they're prominent.
            list.sort((a, b) => {
                const aQ = /-q\d/.test(a.id) ? 1 : 0;
                const bQ = /-q\d/.test(b.id) ? 1 : 0;
                return aQ - bQ;
            });
        }
        return list;
    })();

    const tierCoremlModels = TIER_COREML_IDS[activeTier]
        .map(id => coremlModels.find(m => m.id === id))
        .filter((m): m is DownloadableModel => m !== undefined);

    return (
        <div className="models-tab">
            {/* ── Whisper ──────────────────────────────────────────── */}
            <div className="model-group" ref={whisperGroupRef}>
                <div className="model-group-header">
                    <h3 className="settings-section-title">Whisper</h3>
                    <span className="model-group-sub model-group-sub--whisper">by OpenAI · multilingual · any hardware</span>
                </div>

                <div className="tier-tabs">
                    {TIERS.map(tier => {
                        const hasDownloaded = TIER_MODEL_IDS[tier].some(
                            id => models.find(m => m.id === id)?.downloaded
                        );
                        return (
                            <button
                                key={tier}
                                className={`tier-tab ${activeTier === tier ? 'active' : ''}`}
                                onClick={() => setActiveTier(tier)}
                            >
                                {tier}
                                {hasDownloaded && <span className="tier-dot" />}
                            </button>
                        );
                    })}
                </div>

                <p className="tier-description">
                    {isMac ? TIER_DESCRIPTIONS[activeTier].replace('RAM/VRAM', 'RAM') : TIER_DESCRIPTIONS[activeTier]}
                </p>

                <div className="model-list">
                    {tierModels.map(m => (
                        <div
                            key={m.id}
                            className={`model-item-wrapper${pulseModelIds.has(m.id) ? ' model-item-wrapper--pulse' : ''}`}
                        >
                            <ModelRow model={m} {...rowProps} />
                        </div>
                    ))}
                </div>
            </div>

            {/* ── CoreML Encoders — Apple Silicon only, always visible ── */}
            {isMac && tierCoremlModels.length > 0 && (
                <div className="model-group">
                    <div className="model-group-header">
                        <h3 className="settings-section-title">CoreML Encoders</h3>
                        <span className="model-group-badge">Apple Silicon</span>
                    </div>
                    <p className="model-group-desc">
                        Offloads the {activeTier} encoder to the Apple Neural Engine for faster, lower-power transcription.
                        Download the encoder that matches your Whisper model, then select the model as usual.
                    </p>
                    <div className="model-list">
                        {tierCoremlModels.map(m => (
                            <div key={m.id} className="model-item-wrapper">
                                <ModelRow model={m} {...rowProps} />
                            </div>
                        ))}
                    </div>
                </div>
            )}

            {/* ── Parakeet ─────────────────────────────────────────── */}
            <div className="model-group" ref={parakeetGroupRef}>
                <div className="model-group-header">
                    <h3 className="settings-section-title">Parakeet</h3>
                    <span className="model-group-sub model-group-sub--parakeet">by NVIDIA Nemotron · English only</span>
                </div>
                <div className="model-list">
                    {parakeetModels.map(m => (
                        <div key={m.id} className={`model-item-wrapper${pulseModelIds.has(m.id) ? ' model-item-wrapper--pulse' : ''}`}>
                            <ModelRow model={m} {...rowProps} />
                        </div>
                    ))}
                </div>
            </div>

            {/* ── Granite Speech ───────────────────────────────────── */}
            <div className="model-group" ref={graniteGroupRef}>
                <div className="model-group-header">
                    <h3 className="settings-section-title">Granite Speech</h3>
                    <span className="model-group-sub model-group-sub--granite">by IBM · English · ONNX</span>
                </div>
                <div className="model-list">
                    {graniteModels.map(m => (
                        <div
                            key={m.id}
                            className={`model-item-wrapper${pulseModelIds.has(m.id) ? ' model-item-wrapper--pulse' : ''}`}
                        >
                            <ModelRow model={m} {...rowProps} />
                        </div>
                    ))}
                </div>
            </div>

            {/* ── Post-Processing Models ────────────────────────────── */}
            <div className="model-group">
                <div className="model-group-header">
                    <h3 className="settings-section-title">Post-Processing</h3>
                    <span className="model-group-sub">optional · grammar correction</span>
                </div>
                <div className="model-list">
                    {llmModels.map(m => <ModelRow key={m.id} model={m} {...rowProps} />)}
                </div>
            </div>

        </div>
    );
}
