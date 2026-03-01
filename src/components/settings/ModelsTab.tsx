import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { DownloadableModel, DownloadProgress } from './types';
import { ModelRow } from './ModelRow';

type WhisperTier = 'Tiny' | 'Base' | 'Small' | 'Medium' | 'Large';

const TIERS: WhisperTier[] = ['Tiny', 'Base', 'Small', 'Medium', 'Large'];

const TIER_DESCRIPTIONS: Record<WhisperTier, string> = {
    Tiny:   'Fastest · lowest accuracy · great for quick dictation on any hardware',
    Base:   'Good balance of speed and accuracy · solid starting point',
    Small:  'High accuracy · reasonable speed · best all-rounder',
    Medium: 'Very high accuracy · slower · needs 8 GB RAM',
    Large:  'Best possible accuracy · slowest · requires 10 GB+ RAM/VRAM',
};

const TIER_MODEL_IDS: Record<WhisperTier, string[]> = {
    Tiny:   ['whisper-tiny-en-q5_1', 'whisper-tiny-en', 'whisper-tiny-q5_1', 'whisper-tiny'],
    Base:   ['whisper-base-en-q5_1', 'whisper-base-en', 'whisper-base-q5_1', 'whisper-base'],
    Small:  ['whisper-small-en-q5_1', 'whisper-small-en', 'whisper-small-q5_1', 'whisper-small'],
    Medium: ['whisper-medium-en-q5_0', 'whisper-medium-en', 'whisper-medium-q5_0', 'whisper-medium'],
    Large:  ['whisper-large-v3-turbo-q5_0', 'whisper-large-v3-turbo', 'whisper-large-v3-q5_0', 'whisper-large-v3'],
};

const TIER_RECOMMENDED: Record<WhisperTier, string> = {
    Tiny:   'whisper-tiny-en-q5_1',
    Base:   'whisper-base-en-q5_1',
    Small:  'whisper-small-en-q5_1',
    Medium: 'whisper-medium-en-q5_0',
    Large:  'whisper-large-v3-turbo-q5_0',
};

const TIER_COREML_IDS: Record<WhisperTier, string[]> = {
    Tiny:   ['whisper-tiny-en-coreml', 'whisper-tiny-coreml'],
    Base:   ['whisper-base-en-coreml', 'whisper-base-coreml'],
    Small:  ['whisper-small-en-coreml', 'whisper-small-coreml'],
    Medium: ['whisper-medium-en-coreml', 'whisper-medium-coreml'],
    Large:  ['whisper-large-v3-turbo-coreml', 'whisper-large-v3-coreml'],
};

interface ModelsTabProps {
    models: DownloadableModel[];
    downloadProgress: Record<string, DownloadProgress>;
    onDownload: (id: string, name: string) => void;
    onDelete: (id: string, name: string) => void;
    onVerify: (id: string, name: string) => void;
}

export function ModelsTab({ models, downloadProgress, onDownload, onDelete, onVerify }: ModelsTabProps) {
    const [activeTier, setActiveTier] = useState<WhisperTier>('Small');
    const [platform, setPlatform] = useState('');

    useEffect(() => {
        invoke<string>('get_platform').then(setPlatform).catch(() => {});
    }, []);

    const isMac = platform === 'macos';
    const rowProps = { downloadProgress, onDownload, onDelete, onVerify };

    const parakeetModels = models.filter(m => m.type === 'Parakeet');
    const llmModels = models.filter(m => m.type === 'LLM');
    const utilityModels = models.filter(m => m.type === 'Utility');
    const coremlModels = models.filter(m => m.type === 'CoreML');

    const tierModels = TIER_MODEL_IDS[activeTier]
        .map(id => models.find(m => m.id === id))
        .filter((m): m is DownloadableModel => m !== undefined);

    const tierCoremlModels = TIER_COREML_IDS[activeTier]
        .map(id => coremlModels.find(m => m.id === id))
        .filter((m): m is DownloadableModel => m !== undefined);

    const recommendedId = TIER_RECOMMENDED[activeTier];

    return (
        <div className="models-tab">

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

                <p className="tier-description">{TIER_DESCRIPTIONS[activeTier]}</p>

                <div className="model-list">
                    {tierModels.map(m => (
                        <div key={m.id} className={`model-item-wrapper ${m.id === recommendedId ? 'model-item-wrapper--rec' : ''}`}>
                            {m.id === recommendedId && <span className="badge-rec">Recommended</span>}
                            <ModelRow model={m} {...rowProps} />
                        </div>
                    ))}
                </div>
            </div>

            {/* ── CoreML — macOS only, contextual to active tier ───── */}
            {isMac && tierCoremlModels.length > 0 && (
                <div className="model-group">
                    <div className="model-group-header">
                        <h3 className="settings-section-title">CoreML Encoders</h3>
                        <span className="model-group-badge">Apple Silicon</span>
                    </div>
                    <p className="model-group-desc">
                        Offloads the {activeTier} encoder to the Neural Engine — faster and lower power.
                        Download the encoder that matches your Whisper model above.
                    </p>
                    <div className="model-list">
                        {tierCoremlModels.map(m => <ModelRow key={m.id} model={m} {...rowProps} />)}
                    </div>
                </div>
            )}

            {/* ── Parakeet ─────────────────────────────────────────── */}
            <div className="model-group">
                <div className="model-group-header">
                    <h3 className="settings-section-title">Parakeet</h3>
                    <span className="model-group-sub">by NVIDIA Nemotron · English only · NVIDIA GPU required</span>
                </div>
                <div className="model-list">
                    {parakeetModels.map(m => <ModelRow key={m.id} model={m} {...rowProps} />)}
                </div>
            </div>

            {/* ── Post-Processing Models ────────────────────────────── */}
            <div className="model-group">
                <div className="model-group-header">
                    <h3 className="settings-section-title">Post-Processing</h3>
                    <span className="model-group-sub">optional · grammar correction &amp; spell check</span>
                </div>
                <div className="model-list">
                    {llmModels.map(m => <ModelRow key={m.id} model={m} {...rowProps} />)}
                    {utilityModels.map(m => <ModelRow key={m.id} model={m} {...rowProps} />)}
                </div>
            </div>

        </div>
    );
}
