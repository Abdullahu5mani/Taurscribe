import { useState } from "react";
import type { DownloadableModel, DownloadProgress } from "./types";
import { ModelRow } from "./ModelRow";

interface DownloadsTabProps {
    models: DownloadableModel[];
    downloadProgress: Record<string, DownloadProgress>;
    onDownload: (id: string, name: string) => void;
    onDelete: (id: string, name: string) => void;
    onVerify: (id: string, name: string) => void;
}

const PRIORITY_WHISPER_IDS = [
    'whisper-tiny-en-q5_1',
    'whisper-base-en-q5_1',
    'whisper-small-en-q5_1',
    'whisper-large-v3-turbo-q5_0',
];

const SIZE_ORDER = ['Tiny', 'Base', 'Small', 'Large'];

export function DownloadsTab({ models, downloadProgress, onDownload, onDelete, onVerify }: DownloadsTabProps) {
    const [isWhisperExpanded, setIsWhisperExpanded] = useState(false);

    const whisperModels = models.filter(m => m.type === 'Whisper');
    const parakeetModels = models.filter(m => m.type === 'Parakeet');
    const llmModels = models.filter(m => m.type === 'LLM');
    const utilityModels = models.filter(m => m.type === 'Utility');

    const visibleWhisper = whisperModels
        .filter(m => PRIORITY_WHISPER_IDS.includes(m.id))
        .sort((a, b) => {
            const aIndex = SIZE_ORDER.findIndex(s => a.name.includes(s));
            const bIndex = SIZE_ORDER.findIndex(s => b.name.includes(s));
            return aIndex - bIndex;
        });
    const hiddenWhisper = whisperModels.filter(m => !PRIORITY_WHISPER_IDS.includes(m.id));

    const rowProps = { downloadProgress, onDownload, onDelete, onVerify };

    return (
        <div className="download-manager">
            <h3 className="settings-section-title">Model Library</h3>
            <div style={{ marginBottom: '16px', fontSize: '0.9rem', color: '#94a3b8', background: 'rgba(59, 130, 246, 0.1)', padding: '12px', borderRadius: '8px', border: '1px solid rgba(59, 130, 246, 0.2)' }}>
                📂 <strong>Storage Location:</strong> <code style={{ fontFamily: 'monospace', color: '#e2e8f0' }}>%AppData%\Taurscribe\models</code>
            </div>

            <div className="model-list">
                {parakeetModels.length > 0 && (
                    <div style={{ marginBottom: '24px' }}>
                        <h4 style={{ color: '#fff', borderBottom: '1px solid #334155', paddingBottom: '8px', marginBottom: '12px', display: 'flex', alignItems: 'center', gap: '8px' }}>
                            🦜 Real-Time Streaming
                        </h4>
                        {parakeetModels.map(m => <ModelRow key={m.id} model={m} {...rowProps} />)}
                    </div>
                )}

                {llmModels.length > 0 && (
                    <div style={{ marginBottom: '24px' }}>
                        <h4 style={{ color: '#fff', borderBottom: '1px solid #334155', paddingBottom: '8px', marginBottom: '12px', display: 'flex', alignItems: 'center', gap: '8px' }}>
                            🧠 AI Assistants
                        </h4>
                        {llmModels.map(m => <ModelRow key={m.id} model={m} {...rowProps} />)}
                    </div>
                )}

                <div style={{ marginBottom: '24px' }}>
                    <h4 style={{ color: '#fff', borderBottom: '1px solid #334155', paddingBottom: '8px', marginBottom: '12px', display: 'flex', alignItems: 'center', gap: '8px' }}>
                        📝 Speech Recognition (Whisper)
                    </h4>
                    {visibleWhisper.map(m => <ModelRow key={m.id} model={m} {...rowProps} />)}

                    <div style={{ marginTop: '12px', textAlign: 'center' }}>
                        <button
                            onClick={() => setIsWhisperExpanded(!isWhisperExpanded)}
                            style={{
                                background: 'transparent',
                                border: '1px solid #475569',
                                color: '#94a3b8',
                                padding: '8px 16px',
                                borderRadius: '6px',
                                cursor: 'pointer',
                                fontSize: '0.85rem'
                            }}
                        >
                            {isWhisperExpanded ? 'Show Less Models' : `Show ${hiddenWhisper.length} More Models...`}
                        </button>
                    </div>

                    {isWhisperExpanded && (
                        <div style={{ marginTop: '12px', paddingLeft: '12px', borderLeft: '2px solid #334155' }}>
                            {hiddenWhisper.map(m => <ModelRow key={m.id} model={m} {...rowProps} />)}
                        </div>
                    )}
                </div>

                {utilityModels.length > 0 && (
                    <div style={{ marginBottom: '24px' }}>
                        <h4 style={{ color: '#fff', borderBottom: '1px solid #334155', paddingBottom: '8px', marginBottom: '12px', display: 'flex', alignItems: 'center', gap: '8px' }}>
                            🛠️ Utilities
                        </h4>
                        {utilityModels.map(m => <ModelRow key={m.id} model={m} {...rowProps} />)}
                    </div>
                )}
            </div>
        </div>
    );
}
