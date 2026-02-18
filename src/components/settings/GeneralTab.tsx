interface GeneralTabProps {
    enableGrammarLM: boolean;
    setEnableGrammarLM: (val: boolean) => void;
    llmStatus: string;
    enableSpellCheck: boolean;
    setEnableSpellCheck: (val: boolean) => void;
    spellCheckStatus: string;
}

export function GeneralTab({
    enableGrammarLM,
    setEnableGrammarLM,
    llmStatus,
    enableSpellCheck,
    setEnableSpellCheck,
    spellCheckStatus,
}: GeneralTabProps) {
    return (
        <div className="general-settings">
            <h3 className="settings-section-title">General Settings</h3>

            <div className="setting-card" style={{ background: 'rgba(30, 41, 59, 0.4)', padding: '20px', borderRadius: '12px', border: '1px solid rgba(148, 163, 184, 0.1)' }}>
                <div className="setting-header" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '12px' }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                        <div className="status-dot" style={{
                            backgroundColor: !enableGrammarLM ? "#ef4444" : (llmStatus === "Loading..." ? "#f59e0b" : (llmStatus === "Loaded" ? "#22c55e" : "#ef4444"))
                        }} />
                        <h4 style={{ margin: 0 }}>Grammar Correction (LLM)</h4>
                    </div>
                    <label className={`switch ${llmStatus === "Loading..." ? "switch--disabled" : ""}`} title={llmStatus === "Loading..." ? "Loading… please wait" : undefined}>
                        <input
                            type="checkbox"
                            checked={enableGrammarLM}
                            onChange={(e) => setEnableGrammarLM(e.target.checked)}
                            disabled={llmStatus === "Loading..."}
                        />
                        <span className="slider round"></span>
                    </label>
                </div>
                <p style={{ margin: 0, fontSize: '0.9rem', color: '#94a3b8' }}>
                    Uses local Qwen 2.5 0.5B (GPU safetensors or GGUF) to format and clean up transcripts.
                </p>
                <div className="status-badge" style={{ marginTop: '12px', display: 'inline-block', padding: '4px 8px', borderRadius: '4px', background: 'rgba(255,255,255,0.05)', fontSize: '0.8rem' }}>
                    Status: <span style={{ color: llmStatus === "Loaded" ? "#22c55e" : (llmStatus === "Loading..." ? "#f59e0b" : "#f43f5e") }}>{llmStatus}</span>
                </div>
            </div>

            <div className="setting-card" style={{ marginTop: '16px', background: 'rgba(30, 41, 59, 0.4)', padding: '20px', borderRadius: '12px', border: '1px solid rgba(148, 163, 184, 0.1)' }}>
                <div className="setting-header" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '12px' }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                        <div className="status-dot" style={{
                            backgroundColor: !enableSpellCheck ? "#ef4444" : (spellCheckStatus === "Loading..." ? "#f59e0b" : (spellCheckStatus === "Loaded" ? "#22c55e" : "#ef4444"))
                        }} />
                        <h4 style={{ margin: 0 }}>Spell Check (SymSpell)</h4>
                    </div>
                    <label className={`switch ${spellCheckStatus === "Loading..." ? "switch--disabled" : ""}`} title={spellCheckStatus === "Loading..." ? "Loading… please wait" : undefined}>
                        <input
                            type="checkbox"
                            checked={enableSpellCheck}
                            onChange={(e) => setEnableSpellCheck(e.target.checked)}
                            disabled={spellCheckStatus === "Loading..."}
                        />
                        <span className="slider round"></span>
                    </label>
                </div>
                <p style={{ margin: 0, fontSize: '0.9rem', color: '#94a3b8' }}>
                    Fast dictionary-based spelling correction.
                </p>
            </div>
        </div>
    );
}
