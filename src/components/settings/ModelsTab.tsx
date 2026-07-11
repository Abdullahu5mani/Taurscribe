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
type WhisperLanguage = 'english' | 'multilingual';
type WhisperOptimization = 'quantized' | 'full';

const TIERS: WhisperTier[] = ['Tiny', 'Base', 'Small', 'Medium', 'Large'];
const WHISPER_LANGUAGES: { value: WhisperLanguage; label: string; description: string }[] = [
    { value: 'english', label: 'English', description: 'Best if you only dictate English.' },
    { value: 'multilingual', label: 'Multilingual', description: 'Use this for non-English or mixed-language speech.' },
];
const WHISPER_OPTIMIZATIONS: { value: WhisperOptimization; label: string; description: string }[] = [
    { value: 'quantized', label: 'Quantized', description: 'Smaller download and lower RAM. Slightly less accurate.' },
    { value: 'full', label: 'Full precision', description: 'Largest file. Best quality and best CoreML pairing on Mac.' },
];

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

const WHISPER_MODEL_MATRIX: Record<WhisperTier, Partial<Record<WhisperLanguage, Record<WhisperOptimization, string>>>> = {
    Tiny: {
        english: { quantized: 'whisper-tiny-en-q5_1', full: 'whisper-tiny-en' },
        multilingual: { quantized: 'whisper-tiny-q5_1', full: 'whisper-tiny' },
    },
    Base: {
        english: { quantized: 'whisper-base-en-q5_1', full: 'whisper-base-en' },
        multilingual: { quantized: 'whisper-base-q5_1', full: 'whisper-base' },
    },
    Small: {
        english: { quantized: 'whisper-small-en-q5_1', full: 'whisper-small-en' },
        multilingual: { quantized: 'whisper-small-q5_1', full: 'whisper-small' },
    },
    Medium: {
        english: { quantized: 'whisper-medium-en-q5_0', full: 'whisper-medium-en' },
        multilingual: { quantized: 'whisper-medium-q5_0', full: 'whisper-medium' },
    },
    Large: {
        multilingual: { quantized: 'whisper-large-v3-turbo-q5_0', full: 'whisper-large-v3-turbo' },
    },
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
    const [whisperLanguage, setWhisperLanguage] = useState<WhisperLanguage>('english');
    const [whisperOptimization, setWhisperOptimization] = useState<WhisperOptimization>('quantized');
    const [platform, setPlatform] = useState('');
    const [isAppleSilicon, setIsAppleSilicon] = useState(false);
    const [sysInfo, setSysInfo] = useState<SystemInfo | null>(null);
    const [useCase, setUseCase] = useState<OnboardingUseCase>('quick_notes');
    const hydratedTierRef = useRef(false);
    const [pulseModelIds, setPulseModelIds] = useState<Set<string>>(new Set());
    const whisperGroupRef = useRef<HTMLDivElement>(null);
    const parakeetGroupRef = useRef<HTMLDivElement>(null);
    const cohereGroupRef = useRef<HTMLDivElement>(null);

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
    const cohereModels = models.filter(
        m => m.type === 'Granite'
            && (!m.macosOnly || isMac)
            && (!m.windowsOnly || isWindows),
    );
    const llmModels = models.filter(m => m.type === 'LLM');
    const coremlModels = models.filter(m => m.type === 'CoreML');

    useEffect(() => {
        if (!WHISPER_MODEL_MATRIX[activeTier][whisperLanguage]) {
            setWhisperLanguage('multilingual');
        }
    }, [activeTier, whisperLanguage]);

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
        } else if (scrollTarget === 'granite') {
            groupRef = cohereGroupRef;
            targetModelId = models.find(m => m.type === 'Granite' && !m.downloaded)?.id;
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
    const selectedLanguage = WHISPER_MODEL_MATRIX[activeTier][whisperLanguage] ? whisperLanguage : 'multilingual';
    const selectedWhisperModelId =
        WHISPER_MODEL_MATRIX[activeTier][selectedLanguage]?.[whisperOptimization] ??
        TIER_RECOMMENDED[activeTier];
    const selectedWhisperModel =
        models.find(m => m.id === selectedWhisperModelId) ??
        tierModels[0];
    const selectedCoremlModel = isMac && whisperOptimization === 'full'
        ? tierCoremlModels.find(m => selectedLanguage === 'english' ? m.id.includes('-en-coreml') : !m.id.includes('-en-coreml')) ?? tierCoremlModels[0]
        : undefined;
    const availableLanguages = WHISPER_LANGUAGES.filter(({ value }) => WHISPER_MODEL_MATRIX[activeTier][value]);

    return (
        <div className="models-tab">
            {/* ── Whisper ──────────────────────────────────────────── */}
            <div className="model-group" ref={whisperGroupRef}>
                <div className="model-group-header">
                    <h3 className="settings-section-title">Whisper</h3>
                    <span className="model-group-sub model-group-sub--whisper">by OpenAI · multilingual · any hardware</span>
                </div>

                <div className="whisper-picker-card">
                    <div className="whisper-picker-grid">
                        <label className="whisper-picker-field">
                            <span>Size</span>
                            <select value={activeTier} onChange={(e) => setActiveTier(e.target.value as WhisperTier)}>
                                {TIERS.map(tier => {
                                    const hasDownloaded = TIER_MODEL_IDS[tier].some(
                                        id => models.find(m => m.id === id)?.downloaded
                                    );
                                    return (
                                        <option key={tier} value={tier}>
                                            {tier}{hasDownloaded ? ' - installed' : ''}
                                        </option>
                                    );
                                })}
                            </select>
                        </label>

                        <label className="whisper-picker-field">
                            <span>Language</span>
                            <select
                                value={selectedLanguage}
                                onChange={(e) => setWhisperLanguage(e.target.value as WhisperLanguage)}
                            >
                                {availableLanguages.map(({ value, label }) => (
                                    <option key={value} value={value}>{label}</option>
                                ))}
                            </select>
                        </label>

                        <label className="whisper-picker-field">
                            <span>
                                Quantization
                                <button
                                    type="button"
                                    className="whisper-help-dot"
                                    title="Quantization stores the model in fewer bits. It usually saves RAM, disk, and battery, with a small accuracy tradeoff."
                                >
                                    ?
                                </button>
                            </span>
                            <select
                                value={whisperOptimization}
                                onChange={(e) => setWhisperOptimization(e.target.value as WhisperOptimization)}
                            >
                                {WHISPER_OPTIMIZATIONS.map(({ value, label }) => (
                                    <option key={value} value={value}>{label}</option>
                                ))}
                            </select>
                        </label>
                    </div>

                    <div className="whisper-picker-summary">
                        <p>{isMac ? TIER_DESCRIPTIONS[activeTier].replace('RAM/VRAM', 'RAM') : TIER_DESCRIPTIONS[activeTier]}</p>
                        <p>
                            {WHISPER_LANGUAGES.find(l => l.value === selectedLanguage)?.description}
                            {' '}
                            {WHISPER_OPTIMIZATIONS.find(o => o.value === whisperOptimization)?.description}
                        </p>
                    </div>

                    {selectedWhisperModel && (
                        <div className={`model-item-wrapper${pulseModelIds.has(selectedWhisperModel.id) ? ' model-item-wrapper--pulse' : ''}`}>
                            <ModelRow model={selectedWhisperModel} {...rowProps} />
                        </div>
                    )}

                    {selectedCoremlModel && (
                        <div className="whisper-coreml-match">
                            <div className="coreml-inline-header">
                                <h4 className="settings-section-subtitle">Matching CoreML Encoder</h4>
                                <span className="model-group-badge">Apple Silicon</span>
                            </div>
                            <p className="model-group-desc">
                                Optional Apple Neural Engine encoder for this full-precision Whisper selection.
                            </p>
                            <ModelRow model={selectedCoremlModel} {...rowProps} />
                        </div>
                    )}
                </div>
            </div>

            {/* ── Parakeet ─────────────────────────────────────────── */}
            <div className="model-group" ref={parakeetGroupRef}>
                <div className="model-group-header">
                    <h3 className="settings-section-title">Parakeet</h3>
                    <span className="model-group-sub model-group-sub--parakeet">by NVIDIA · streaming &amp; high-accuracy variants</span>
                </div>
                <div className="model-list">
                    {parakeetModels.map(m => (
                        <div key={m.id} className={`model-item-wrapper${pulseModelIds.has(m.id) ? ' model-item-wrapper--pulse' : ''}`}>
                            <ModelRow model={m} {...rowProps} />
                        </div>
                    ))}
                </div>
            </div>

            {/* ── Granite ─────────────────────────────────────────── */}
            <div className="model-group" ref={cohereGroupRef}>
                <div className="model-group-header">
                    <h3 className="settings-section-title">Granite</h3>
                    <span className="model-group-badge model-group-badge--warn">Experimental</span>
                    <span className="model-group-sub model-group-sub--cohere">by IBM Granite · Multilingual · ONNX</span>
                </div>
                <div className="model-list">
                    {cohereModels.map(m => (
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
