interface GeneralTabProps {
    enableSpellCheck: boolean;
    setEnableSpellCheck: (val: boolean) => void;
    spellCheckStatus: string;
}

export function GeneralTab({
    enableSpellCheck,
    setEnableSpellCheck,
    spellCheckStatus,
}: GeneralTabProps) {
    return (
        <div className="general-settings">
            <h3 className="settings-section-title">General Settings</h3>

            <div className="setting-card" style={{ marginTop: '0', background: 'rgba(30, 41, 59, 0.4)', padding: '20px', borderRadius: '12px', border: '1px solid rgba(148, 163, 184, 0.1)' }}>
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
