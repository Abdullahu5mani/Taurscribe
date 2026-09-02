import { useState } from "react";
import type { DictEntry, SnippetEntry } from "../../hooks/usePersonalization";
import { IconBook, IconFileLightning, IconTrash } from "../Icons";

interface TextTabProps {
    dictionary: DictEntry[];
    addDictEntry: (entry: Omit<DictEntry, "id">) => void;
    updateDictEntry: (id: string, updates: Partial<Omit<DictEntry, "id">>) => void;
    removeDictEntry: (id: string) => void;
    snippets: SnippetEntry[];
    addSnippet: (entry: Omit<SnippetEntry, "id">) => void;
    updateSnippet: (id: string, updates: Partial<Omit<SnippetEntry, "id">>) => void;
    removeSnippet: (id: string) => void;
}

export function TextTab({
    dictionary, addDictEntry, updateDictEntry, removeDictEntry,
    snippets, addSnippet, updateSnippet, removeSnippet,
}: TextTabProps) {
    const [newSoundsLike, setNewSoundsLike] = useState("");
    const [newCorrect, setNewCorrect] = useState("");
    const [newTrigger, setNewTrigger] = useState("");
    const [newExpansion, setNewExpansion] = useState("");

    const handleAddDict = () => {
        const sl = newSoundsLike.trim();
        const co = newCorrect.trim();
        if (!sl || !co) return;
        addDictEntry({ soundsLike: sl, correct: co });
        setNewSoundsLike(""); setNewCorrect("");
    };

    const handleAddSnippet = () => {
        const trigger = newTrigger.trim();
        const expansion = newExpansion.trim();
        if (!trigger || !expansion) return;
        addSnippet({ trigger, expansion });
        setNewTrigger(""); setNewExpansion("");
    };

    return (
        <div className="text-tab">

            {/* ── Custom Dictionary ───────────────────────────────── */}
            <h3 className="settings-section-title">Custom Dictionary</h3>

            <div className="setting-card">
                <p className="setting-card-desc">
                    Fix words the AI keeps getting wrong — proper nouns, names, technical terms.
                    Replacements run <strong>before</strong> grammar correction.
                </p>

                <div className="dict-add-row">
                    <div className="dict-add-fields">
                        <div className="dict-field">
                            <label className="dict-field-label" htmlFor="dict-input-sounds-like">Sounds like</label>
                            <input
                                type="text"
                                id="dict-input-sounds-like"
                                data-testid="dict-input-sounds-like"
                                className="dict-input"
                                placeholder="e.g. tor scribe"
                                aria-label="Sounds like word or phrase"
                                value={newSoundsLike}
                                onChange={e => setNewSoundsLike(e.target.value)}
                                onKeyDown={e => { if (e.key === 'Enter') handleAddDict(); }}
                            />
                        </div>
                        <span className="dict-arrow">→</span>
                        <div className="dict-field">
                            <label className="dict-field-label" htmlFor="dict-input-correct">Correct spelling</label>
                            <input
                                type="text"
                                id="dict-input-correct"
                                data-testid="dict-input-correct"
                                className="dict-input"
                                placeholder="e.g. Taurscribe"
                                aria-label="Correct spelling word or phrase"
                                value={newCorrect}
                                onChange={e => setNewCorrect(e.target.value)}
                                onKeyDown={e => { if (e.key === 'Enter') handleAddDict(); }}
                            />
                        </div>
                    </div>
                    <button
                        id="dict-add-btn"
                        data-testid="dict-add-btn"
                        className="ghost-btn ghost-btn--confirm"
                        onClick={handleAddDict}
                        disabled={!newSoundsLike.trim() || !newCorrect.trim()}
                        aria-label="Add dictionary entry"
                    >+ Add</button>
                </div>

                {dictionary.length === 0 ? (
                    <div className="dict-empty">
                        <span className="dict-empty-icon"><IconBook size={28} /></span>
                        <span>No entries yet — add words the AI keeps getting wrong.</span>
                    </div>
                ) : (
                    <div className="dict-list">
                        {dictionary.map(entry => (
                            <div key={entry.id} className="dict-entry" id={`dict-entry-${entry.id}`} data-testid={`dict-entry-${entry.id}`}>
                                <input
                                    type="text"
                                    id={`dict-entry-sounds-like-${entry.id}`}
                                    data-testid={`dict-entry-sounds-like-${entry.id}`}
                                    className="dict-input dict-input--inline"
                                    value={entry.soundsLike}
                                    onChange={e => updateDictEntry(entry.id, { soundsLike: e.target.value })}
                                    aria-label={`Sounds like for ${entry.soundsLike}`}
                                    title="What it sounds like"
                                />
                                <span className="dict-arrow-sm">→</span>
                                <input
                                    type="text"
                                    id={`dict-entry-correct-${entry.id}`}
                                    data-testid={`dict-entry-correct-${entry.id}`}
                                    className="dict-input dict-input--inline"
                                    value={entry.correct}
                                    onChange={e => updateDictEntry(entry.id, { correct: e.target.value })}
                                    aria-label={`Correct spelling for ${entry.correct}`}
                                    title="Correct spelling"
                                />
                                <button
                                    id={`dict-delete-btn-${entry.id}`}
                                    data-testid={`dict-delete-btn-${entry.id}`}
                                    className="dict-delete"
                                    onClick={() => removeDictEntry(entry.id)}
                                    aria-label={`Remove ${entry.soundsLike} → ${entry.correct}`}
                                >
                                    <IconTrash size={14} />
                                </button>
                            </div>
                        ))}
                    </div>
                )}
            </div>

            <p className="dict-tab-note">
                <strong>Tip:</strong> Enter words exactly how the AI mistranscribes them in the "Sounds like" field.
            </p>

            {/* ── Text Snippets ───────────────────────────────────── */}
            <h3 className="settings-section-title" style={{ marginTop: '36px' }}>Text Snippets</h3>

            <div className="setting-card">
                <p className="setting-card-desc">
                    Say a short trigger phrase and it gets replaced with a longer block of text.
                    Snippets expand <strong>after</strong> grammar correction.
                </p>

                <div className="snippet-add-section">
                    <div className="snippet-add-row">
                        <div className="dict-field">
                            <label className="dict-field-label" htmlFor="snippet-input-trigger">Trigger phrase</label>
                            <input
                                type="text"
                                id="snippet-input-trigger"
                                data-testid="snippet-input-trigger"
                                className="dict-input"
                                placeholder="e.g. ty"
                                aria-label="Snippet trigger phrase"
                                value={newTrigger}
                                onChange={e => setNewTrigger(e.target.value)}
                            />
                        </div>
                        <span className="dict-arrow">→</span>
                        <div className="dict-field snippet-expansion-field">
                            <label className="dict-field-label" htmlFor="snippet-input-expansion">Expands to</label>
                            <textarea
                                id="snippet-input-expansion"
                                data-testid="snippet-input-expansion"
                                className="snippet-textarea"
                                placeholder="e.g. Thank you for your time!"
                                aria-label="Snippet expands to text"
                                value={newExpansion}
                                onChange={e => setNewExpansion(e.target.value)}
                                rows={2}
                            />
                        </div>
                    </div>
                    <button
                        id="snippet-add-btn"
                        data-testid="snippet-add-btn"
                        className="ghost-btn ghost-btn--confirm"
                        onClick={handleAddSnippet}
                        disabled={!newTrigger.trim() || !newExpansion.trim()}
                        aria-label="Add snippet entry"
                    >+ Add</button>
                </div>

                {snippets.length === 0 ? (
                    <div className="dict-empty">
                        <span className="dict-empty-icon"><IconFileLightning size={28} /></span>
                        <span>No snippets yet — create shortcuts for text you repeat often.</span>
                    </div>
                ) : (
                    <div className="snippet-list">
                        {snippets.map(snippet => (
                            <div key={snippet.id} className="snippet-entry" id={`snippet-entry-${snippet.id}`} data-testid={`snippet-entry-${snippet.id}`}>
                                <div className="snippet-entry-top">
                                    <div className="snippet-trigger-wrap">
                                        <span className="snippet-label">Say:</span>
                                        <input
                                            type="text"
                                            id={`snippet-entry-trigger-${snippet.id}`}
                                            data-testid={`snippet-entry-trigger-${snippet.id}`}
                                            className="dict-input dict-input--inline snippet-trigger-input"
                                            value={snippet.trigger}
                                            onChange={e => updateSnippet(snippet.id, { trigger: e.target.value })}
                                            aria-label={`Trigger phrase for ${snippet.trigger}`}
                                            title="Trigger phrase"
                                        />
                                    </div>
                                    <button
                                        id={`snippet-delete-btn-${snippet.id}`}
                                        data-testid={`snippet-delete-btn-${snippet.id}`}
                                        className="dict-delete"
                                        onClick={() => removeSnippet(snippet.id)}
                                        aria-label={`Remove snippet: ${snippet.trigger}`}
                                    >
                                        <IconTrash size={14} />
                                    </button>
                                </div>
                                <div className="snippet-expansion-wrap">
                                    <span className="snippet-label">Get:</span>
                                    <textarea
                                        id={`snippet-entry-expansion-${snippet.id}`}
                                        data-testid={`snippet-entry-expansion-${snippet.id}`}
                                        className="snippet-textarea snippet-textarea--inline"
                                        value={snippet.expansion}
                                        onChange={e => updateSnippet(snippet.id, { expansion: e.target.value })}
                                        rows={2}
                                        aria-label={`Expansion text for ${snippet.trigger}`}
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
