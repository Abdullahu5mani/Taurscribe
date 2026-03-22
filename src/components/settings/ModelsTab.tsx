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

// The corresponding recommended CoreML encoder for each tier on macOS.
const TIER_COREML_RECOMMENDED: Record<WhisperTier, string> = {
    Tiny: 'whisper-tiny-en-coreml',
    Base: 'whisper-base-en-coreml',
    Small: 'whisper-small-en-coreml',
    Medium: 'whisper-medium-en-coreml',
    Large: 'whisper-large-v3-turbo-coreml',
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
}

export function ModelsTab({ models, downloadProgress, onDownload, onDelete, onCancelDownload }: ModelsTabProps) {
    const [activeTier, setActiveTier] = useState<WhisperTier>('Small');
    const [platform, setPlatform] = useState('');
    const [isAppleSilicon, setIsAppleSilicon] = useState(false);
    const [sysInfo, setSysInfo] = useState<SystemInfo | null>(null);
    const [useCase, setUseCase] = useState<OnboardingUseCase>('quick_notes');
    const hydratedTierRef = useRef(false);

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
    const rowProps = { downloadProgress, onDownload, onDelete, onCancelDownload };
    const recommendation = useMemo(
        () => computeModelRecommendation({ sysInfo, isAppleSilicon, useCase }),
        [sysInfo, isAppleSilicon, useCase],
    );

    const parakeetModels = models.filter(m => m.type === 'Parakeet');
    const graniteModels = models.filter(m => m.type === 'GraniteSpeech');
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

    const recommendationBadge = (modelId: string) => {
        if (modelId === recommendation.primaryModelId) return 'Primary';
        if (modelId === recommendation.backupModelId) return 'Backup';
        return null;
    };

    const recommendedModels = Array.from(
        new Map(
            [recommendation.primaryModelId, recommendation.backupModelId]
                .filter((id): id is string => Boolean(id))
                .map((id) => {
                    const model = models.find((entry) => entry.id === id);
                    return model ? [id, model] : null;
                })
                .filter((entry): entry is [string, DownloadableModel] => entry !== null),
        ).values(),
    );

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

    const recommendedId = isAppleSilicon ? TIER_RECOMMENDED_ANS[activeTier] : TIER_RECOMMENDED[activeTier];
    const coremlRecommendedId = TIER_COREML_RECOMMENDED[activeTier];

    return (
        <div className="models-tab">
            {recommendedModels.length > 0 && (
                <div className="model-group model-group--recommended">
                    <div className="model-group-header">
                        <h3 className="settings-section-title">Recommended For You</h3>
                        <span className="model-group-badge">{recommendation.useCaseLabel}</span>
                    </div>
                    <p className="model-group-desc model-group-desc--highlight">
                        {recommendation.summary}
                    </p>
                    <p className="model-group-desc">
                        {recommendation.hardwareLine}
                    </p>
                    <div className="model-list">
                        {recommendedModels.map((model) => {
                            const badge = recommendationBadge(model.id);
                            return (
                                <div
                                    key={model.id}
                                    className={`model-item-wrapper model-item-wrapper--rec${badge === 'Backup' ? ' model-item-wrapper--rec-secondary' : ''}`}
                                >
                                    {badge && <span className={`badge-rec${badge === 'Backup' ? ' badge-rec--secondary' : ''}`}>{badge}</span>}
                                    <ModelRow model={model} {...rowProps} />
                                </div>
                            );
                        })}
                    </div>
                </div>
            )}

            {/* ── Whisper ──────────────────────────────────────────── */}
            <div className="model-group">
                <div className="model-group-header">
                    <h3 className="settings-section-title">Whisper</h3>
                    <span className="model-group-sub">by OpenAI · multilingual · any hardware</span>
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
                            className={`model-item-wrapper ${m.id === recommendedId || recommendationBadge(m.id) ? 'model-item-wrapper--rec' : ''}${recommendationBadge(m.id) === 'Backup' ? ' model-item-wrapper--rec-secondary' : ''}`}
                        >
                            {(recommendationBadge(m.id) || (m.id === recommendedId ? 'Recommended' : null)) && (
                                <span className={`badge-rec${recommendationBadge(m.id) === 'Backup' ? ' badge-rec--secondary' : ''}`}>
                                    {recommendationBadge(m.id) || 'Recommended'}
                                </span>
                            )}
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
                            <div key={m.id} className={`model-item-wrapper ${m.id === coremlRecommendedId ? 'model-item-wrapper--rec' : ''}`}>
                                {m.id === coremlRecommendedId && <span className="badge-rec">Recommended</span>}
                                <ModelRow model={m} {...rowProps} />
                            </div>
                        ))}
                    </div>
                </div>
            )}

            {/* ── Parakeet ─────────────────────────────────────────── */}
            <div className="model-group">
                <div className="model-group-header">
                    <h3 className="settings-section-title">Parakeet</h3>
                    <span className="model-group-sub">by NVIDIA Nemotron · English only</span>
                </div>
                <div className="model-list">
                    {parakeetModels.map(m => (
                        <div key={m.id} className="model-item-wrapper">
                            <ModelRow model={m} {...rowProps} />
                        </div>
                    ))}
                </div>
            </div>

            {/* ── Granite Speech ───────────────────────────────────── */}
            <div className="model-group">
                <div className="model-group-header">
                    <h3 className="settings-section-title">Granite Speech</h3>
                    <span className="model-group-sub">by IBM · English · ONNX</span>
                </div>
                <div className="model-list">
                    {graniteModels.map(m => (
                        <div
                            key={m.id}
                            className={`model-item-wrapper ${recommendationBadge(m.id) ? 'model-item-wrapper--rec' : ''}`}
                        >
                            {recommendationBadge(m.id) && <span className="badge-rec">{recommendationBadge(m.id)}</span>}
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
