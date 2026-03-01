interface PostProcessingTabProps {
    enableGrammarLM: boolean;
    setEnableGrammarLM: (val: boolean) => void;
    llmStatus: string;
    llmBackend: 'gpu' | 'cpu';
    setLlmBackend: (val: 'gpu' | 'cpu') => void;
    transcriptionStyle: string;
    setTranscriptionStyle: (val: string) => void;
    enableSpellCheck: boolean;
    setEnableSpellCheck: (val: boolean) => void;
    spellCheckStatus: string;
}

const STYLES = [
    { value: 'Auto',         label: 'Auto',         desc: 'Let the model decide' },
    { value: 'Casual',       label: 'Casual',        desc: 'Relaxed, conversational tone' },
    { value: 'Verbatim',     label: 'Verbatim',      desc: 'Minimal changes, preserve speech' },
    { value: 'Enthusiastic', label: 'Enthusiastic',  desc: 'Energetic and expressive' },
    { value: 'Software_Dev', label: 'Software Dev',  desc: 'Technical language, code terms' },
    { value: 'Professional', label: 'Professional',  desc: 'Formal and polished' },
];

function statusColor(status: string, enabled: boolean): string {
    if (!enabled) return '#4b4b55';
    if (status === 'Loaded') return '#3ecfa5';
    if (status === 'Loading...') return '#e09f3e';
    return '#ef4444';
}

export function PostProcessingTab({
    enableGrammarLM, setEnableGrammarLM, llmStatus, llmBackend, setLlmBackend,
    transcriptionStyle, setTranscriptionStyle,
    enableSpellCheck, setEnableSpellCheck, spellCheckStatus,
}: PostProcessingTabProps) {
    const llmLoading = llmStatus === 'Loading...';
    const llmLoaded = llmStatus === 'Loaded';

    return (
        <div className="pp-tab">

            {/* ── Grammar Correction ──────────────────────────────── */}
            <h3 className="settings-section-title">Grammar Correction</h3>

            <div className="setting-card">
                <div className="setting-card-header">
                    <div className="setting-card-label">
                        <span className="status-dot" style={{ background: statusColor(llmStatus, enableGrammarLM) }} />
                        <span>Grammar LLM</span>
                        <span className="setting-card-meta">Qwen 2.5 0.5B · GGUF</span>
                    </div>
                    <label className={`switch ${llmLoading ? 'switch--disabled' : ''}`}>
                        <input
                            type="checkbox"
                            checked={enableGrammarLM}
                            onChange={e => setEnableGrammarLM(e.target.checked)}
                            disabled={llmLoading}
                        />
                        <span className="slider round" />
                    </label>
                </div>
                <p className="setting-card-desc">
                    Runs a local LLM after each recording to fix grammar, punctuation, and formatting.
                </p>

                <div className="setting-row">
                    <span className="setting-row-label">Status</span>
                    <span className="status-badge" style={{ color: statusColor(llmStatus, true) }}>{llmStatus}</span>
                </div>

                <div className="setting-row">
                    <span className="setting-row-label">Backend</span>
                    <select
                        className="select-input"
                        value={llmBackend}
                        onChange={e => setLlmBackend(e.target.value as 'gpu' | 'cpu')}
                        disabled={llmLoading || llmLoaded}
                        title={llmLoaded ? 'Turn off grammar LLM to change backend' : undefined}
                    >
                        <option value="gpu">Auto / GPU</option>
                        <option value="cpu">CPU Only</option>
                    </select>
                </div>
            </div>

            {/* ── Transcription Style ─────────────────────────────── */}
            <div className="setting-card" style={{ marginTop: '12px' }}>
                <div className="setting-card-header">
                    <span className="setting-card-label-plain">Transcription Style</span>
                    {!llmLoaded && (
                        <span className="setting-card-meta">requires Grammar LLM</span>
                    )}
                </div>
                <p className="setting-card-desc">
                    Controls the tone the LLM applies when cleaning up the transcript.
                </p>
                <div className="style-grid">
                    {STYLES.map(s => (
                        <button
                            key={s.value}
                            className={`style-btn ${transcriptionStyle === s.value ? 'active' : ''}`}
                            onClick={() => setTranscriptionStyle(s.value)}
                            disabled={!llmLoaded}
                            title={s.desc}
                        >
                            {s.label}
                        </button>
                    ))}
                </div>
            </div>

            {/* ── Spell Check ─────────────────────────────────────── */}
            <h3 className="settings-section-title" style={{ marginTop: '32px' }}>Spell Check</h3>

            <div className="setting-card">
                <div className="setting-card-header">
                    <div className="setting-card-label">
                        <span className="status-dot" style={{ background: statusColor(spellCheckStatus, enableSpellCheck) }} />
                        <span>SymSpell Dictionary</span>
                        <span className="setting-card-meta">English · 82k words</span>
                    </div>
                    <label className={`switch ${spellCheckStatus === 'Loading...' ? 'switch--disabled' : ''}`}>
                        <input
                            type="checkbox"
                            checked={enableSpellCheck}
                            onChange={e => setEnableSpellCheck(e.target.checked)}
                            disabled={spellCheckStatus === 'Loading...'}
                        />
                        <span className="slider round" />
                    </label>
                </div>
                <p className="setting-card-desc">
                    Fast dictionary-based spelling correction applied after transcription. Runs before the grammar LLM.
                </p>
                <div className="setting-row">
                    <span className="setting-row-label">Status</span>
                    <span className="status-badge" style={{ color: statusColor(spellCheckStatus, true) }}>{spellCheckStatus}</span>
                </div>
            </div>

            <p className="pp-tab-note">
                Download the required models from the <strong>Models</strong> tab.
            </p>
        </div>
    );
}
