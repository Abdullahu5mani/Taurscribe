import { useState } from "react";
import type { SnippetEntry } from "../../hooks/usePersonalization";

interface SnippetsTabProps {
    snippets: SnippetEntry[];
    addSnippet: (entry: Omit<SnippetEntry, "id">) => void;
    updateSnippet: (id: string, updates: Partial<Omit<SnippetEntry, "id">>) => void;
    removeSnippet: (id: string) => void;
}

export function SnippetsTab({
    snippets,
    addSnippet,
    updateSnippet,
    removeSnippet,
}: SnippetsTabProps) {
    const [newTrigger, setNewTrigger] = useState("");
    const [newExpansion, setNewExpansion] = useState("");

    const handleAdd = () => {
        const trigger = newTrigger.trim();
        const expansion = newExpansion.trim();
        if (!trigger || !expansion) return;
        addSnippet({ trigger, expansion });
        setNewTrigger("");
        setNewExpansion("");
    };

    return (
        <div className="snippets-tab">
            <h3 className="settings-section-title">Text Snippets</h3>

            <div className="setting-card">
                <p className="setting-card-desc">
                    Say a short trigger phrase and it gets replaced with a longer block of text.
                    Snippets expand <strong>after</strong> grammar correction so the LLM won't modify your expansion.
                </p>

                {/* ── Add snippet ─────────────────────────────────────── */}
                <div className="snippet-add-section">
                    <div className="snippet-add-row">
                        <div className="dict-field">
                            <label className="dict-field-label">Trigger phrase</label>
                            <input
                                type="text"
                                className="dict-input"
                                placeholder='e.g. ty'
                                value={newTrigger}
                                onChange={(e) => setNewTrigger(e.target.value)}
                            />
                        </div>
                        <span className="dict-arrow">→</span>
                        <div className="dict-field snippet-expansion-field">
                            <label className="dict-field-label">Expands to</label>
                            <textarea
                                className="snippet-textarea"
                                placeholder="e.g. Thank you for your time!"
                                value={newExpansion}
                                onChange={(e) => setNewExpansion(e.target.value)}
                                rows={2}
                            />
                        </div>
                    </div>
                    <button
                        className="ghost-btn ghost-btn--confirm"
                        onClick={handleAdd}
                        disabled={!newTrigger.trim() || !newExpansion.trim()}
                        title="Add snippet"
                    >
                        + Add
                    </button>
                </div>

                {/* ── Snippet list ────────────────────────────────────── */}
                {snippets.length === 0 ? (
                    <div className="dict-empty">
                        <span className="dict-empty-icon">⚡</span>
                        <span>No snippets yet — create shortcuts for text you repeat often.</span>
                    </div>
                ) : (
                    <div className="snippet-list">
                        {snippets.map((snippet) => (
                            <div key={snippet.id} className="snippet-entry">
                                <div className="snippet-entry-top">
                                    <div className="snippet-trigger-wrap">
                                        <span className="snippet-label">Say:</span>
                                        <input
                                            type="text"
                                            className="dict-input dict-input--inline snippet-trigger-input"
                                            value={snippet.trigger}
                                            onChange={(e) =>
                                                updateSnippet(snippet.id, { trigger: e.target.value })
                                            }
                                            title="Trigger phrase"
                                        />
                                    </div>
                                    <button
                                        className="dict-delete"
                                        onClick={() => removeSnippet(snippet.id)}
                                        title="Remove snippet"
                                        aria-label="Remove"
                                    >
                                        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                                            <polyline points="3 6 5 6 21 6" />
                                            <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                                        </svg>
                                    </button>
                                </div>
                                <div className="snippet-expansion-wrap">
                                    <span className="snippet-label">Get:</span>
                                    <textarea
                                        className="snippet-textarea snippet-textarea--inline"
                                        value={snippet.expansion}
                                        onChange={(e) =>
                                            updateSnippet(snippet.id, { expansion: e.target.value })
                                        }
                                        rows={2}
                                        title="Expansion text"
                                    />
                                </div>
                            </div>
                        ))}
                    </div>
                )}
            </div>

            <p className="dict-tab-note">
                <strong>Tip:</strong> Use short, unique triggers that you wouldn't say in normal speech.
            </p>
        </div>
    );
}
