import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { DictEntry, SnippetEntry } from "../../hooks/usePersonalization";
import { IconBook, IconFileLightning, IconTrash, IconBolt, IconX } from "../Icons";

interface TextTabProps {
    dictionary: DictEntry[];
    addDictEntry: (entry: Omit<DictEntry, "id">) => void;
    updateDictEntry: (id: string, updates: Partial<Omit<DictEntry, "id">>) => void;
    removeDictEntry: (id: string) => void;
    snippets: SnippetEntry[];
    addSnippet: (entry: Omit<SnippetEntry, "id">) => void;
    updateSnippet: (id: string, updates: Partial<Omit<SnippetEntry, "id">>) => void;
    removeSnippet: (id: string) => void;
    // Custom Vocabulary & Jargon
    customVocabulary?: string[];
    contextBiasEnabled?: boolean;
    addVocabWord?: (word: string) => void;
    removeVocabWord?: (word: string) => void;
    addVocabPreset?: (category: "developer" | "medical" | "legal") => void;
    clearVocab?: () => void;
    setContextBiasEnabled?: (enabled: boolean) => void;
}

export function TextTab({
    dictionary, addDictEntry, updateDictEntry, removeDictEntry,
    snippets, addSnippet, updateSnippet, removeSnippet,
    customVocabulary = [], contextBiasEnabled = true,
    addVocabWord, removeVocabWord, addVocabPreset, clearVocab, setContextBiasEnabled,
}: TextTabProps) {
    const [newSoundsLike, setNewSoundsLike] = useState("");
    const [newCorrect, setNewCorrect] = useState("");
    const [newTrigger, setNewTrigger] = useState("");
    const [newExpansion, setNewExpansion] = useState("");
    const [newVocabTerm, setNewVocabTerm] = useState("");
    const [previewData, setPreviewData] = useState<{ active_window: string | null; assembled_prompt: string | null } | null>(null);
    const [loadingPreview, setLoadingPreview] = useState(false);

    const handleAddVocab = () => {
        const term = newVocabTerm.trim();
        if (!term || !addVocabWord) return;
        addVocabWord(term);
        setNewVocabTerm("");
    };

    const handleRefreshPreview = async () => {
        setLoadingPreview(true);
        try {
            const res = await invoke<{
                active_window: string | null;
                assembled_prompt: string | null;
                custom_vocab_count: number;
            }>("get_active_context_preview", {
                customVocab: customVocabulary,
                includeWindow: contextBiasEnabled,
            });
            setPreviewData(res);
        } catch (err) {
            console.error("Failed to load context preview:", err);
        } finally {
            setLoadingPreview(false);
        }
    };

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

            {/* ── Custom Vocabulary & Context Jargon Injection ─────────── */}
            <div className="vocab-header-row">
                <h3 className="settings-section-title" style={{ margin: 0 }}>
                    Custom Vocabulary & Decoder Jargon
                </h3>
                <span className="vocab-count-badge">
                    {customVocabulary.length} {customVocabulary.length === 1 ? "term" : "terms"}
                </span>
            </div>

            <div className="setting-card">
                <p className="setting-card-desc">
                    Biases the acoustic speech decoder toward specialized names, acronyms, and technical jargon.
                    Eliminates phonetic misspellings at the source before transcripts are generated.
                </p>

                {/* Context Bias Toggle */}
                <div className="setting-row" style={{ padding: "8px 0 16px 0", borderBottom: "1px solid rgba(255,255,255,0.06)" }}>
                    <div className="setting-info">
                        <span className="setting-label">Active App Contextual Biasing</span>
                        <span className="setting-desc">
                            Automatically infer domain keywords from your currently focused window (IDEs, Slack, Zoom, medical software).
                        </span>
                    </div>
                    <label className="toggle-switch">
                        <input
                            type="checkbox"
                            id="context-bias-toggle"
                            data-testid="context-bias-toggle"
                            checked={contextBiasEnabled}
                            onChange={(e) => setContextBiasEnabled?.(e.target.checked)}
                            aria-label="Toggle active app contextual biasing"
                        />
                        <span className="toggle-slider"></span>
                    </label>
                </div>

                {/* Add Term Input */}
                <div className="dict-add-row" style={{ marginTop: "16px" }}>
                    <div className="dict-field" style={{ flex: 1 }}>
                        <label className="dict-field-label" htmlFor="vocab-input-term">Add Technical Term or Name</label>
                        <input
                            type="text"
                            id="vocab-input-term"
                            data-testid="vocab-input-term"
                            className="dict-input"
                            placeholder="e.g. Taurscribe, Kubernetes, Athenaïs, useCallback"
                            aria-label="Technical term or name"
                            value={newVocabTerm}
                            onChange={(e) => setNewVocabTerm(e.target.value)}
                            onKeyDown={(e) => { if (e.key === "Enter") handleAddVocab(); }}
                        />
                    </div>
                    <button
                        id="vocab-add-btn"
                        data-testid="vocab-add-btn"
                        className="ghost-btn ghost-btn--confirm"
                        onClick={handleAddVocab}
                        disabled={!newVocabTerm.trim()}
                        aria-label="Add custom vocabulary term"
                    >
                        + Add Term
                    </button>
                </div>

                {/* Preset Packs */}
                <div className="vocab-presets-section">
                    <span className="vocab-presets-label">Domain Presets:</span>
                    <div className="vocab-presets-row">
                        <button
                            type="button"
                            id="vocab-preset-developer"
                            data-testid="vocab-preset-developer"
                            className="vocab-preset-btn"
                            onClick={() => addVocabPreset?.("developer")}
                            title="Add Developer keywords (TypeScript, Rust, Docker, etc.)"
                        >
                            + Developer Pack
                        </button>
                        <button
                            type="button"
                            id="vocab-preset-medical"
                            data-testid="vocab-preset-medical"
                            className="vocab-preset-btn"
                            onClick={() => addVocabPreset?.("medical")}
                            title="Add Medical keywords (hypertension, tachycardia, etc.)"
                        >
                            + Medical Pack
                        </button>
                        <button
                            type="button"
                            id="vocab-preset-legal"
                            data-testid="vocab-preset-legal"
                            className="vocab-preset-btn"
                            onClick={() => addVocabPreset?.("legal")}
                            title="Add Legal keywords (affidavit, indemnification, etc.)"
                        >
                            + Legal Pack
                        </button>
                        {customVocabulary.length > 0 && (
                            <button
                                type="button"
                                id="vocab-clear-all"
                                data-testid="vocab-clear-all"
                                className="vocab-preset-btn vocab-preset-btn--clear"
                                onClick={() => clearVocab?.()}
                                title="Remove all custom vocabulary terms"
                            >
                                Clear All
                            </button>
                        )}
                    </div>
                </div>

                {/* Vocabulary Tags Cloud */}
                {customVocabulary.length === 0 ? (
                    <div className="dict-empty" style={{ padding: "16px 0" }}>
                        <span className="dict-empty-icon"><IconBolt size={24} /></span>
                        <span>No custom terms configured. Add terms or pick a domain preset above.</span>
                    </div>
                ) : (
                    <div className="vocab-tags-container">
                        {customVocabulary.map((word) => (
                            <span key={word} className="vocab-tag">
                                <span className="vocab-tag-text">{word}</span>
                                <button
                                    type="button"
                                    id={`vocab-remove-${word}`}
                                    data-testid={`vocab-remove-${word}`}
                                    className="vocab-tag-remove"
                                    onClick={() => removeVocabWord?.(word)}
                                    aria-label={`Remove term ${word}`}
                                    title="Remove term"
                                >
                                    <IconX size={12} />
                                </button>
                            </span>
                        ))}
                    </div>
                )}

                {/* Live Context Prompt Preview */}
                <div className="vocab-preview-container">
                    <div className="vocab-preview-header">
                        <span className="vocab-preview-title">Live Decoder Prompt Preview</span>
                        <button
                            type="button"
                            id="vocab-preview-refresh"
                            data-testid="vocab-preview-refresh"
                            className="vocab-preview-refresh-btn"
                            onClick={handleRefreshPreview}
                            disabled={loadingPreview}
                        >
                            {loadingPreview ? "Reading Context..." : "Inspect Active Decoder Prompt"}
                        </button>
                    </div>
                    {previewData && (
                        <div className="vocab-preview-body">
                            <div className="vocab-preview-item">
                                <span className="vocab-preview-key">Active Window:</span>
                                <span className="vocab-preview-val">{previewData.active_window || "(none detected)"}</span>
                            </div>
                            <div className="vocab-preview-item">
                                <span className="vocab-preview-key">Whisper Initial Prompt:</span>
                                <span className="vocab-preview-prompt">
                                    {previewData.assembled_prompt ? `"${previewData.assembled_prompt}"` : "(none - vocabulary empty & context disabled)"}
                                </span>
                            </div>
                        </div>
                    )}
                </div>
            </div>

            <p className="dict-tab-note">
                <strong>Tip:</strong> Decoder biasing guides beam search probabilities so proper nouns and acronyms are recognized on the first pass.
            </p>

            {/* ── Custom Dictionary ───────────────────────────────── */}
            <h3 className="settings-section-title" style={{ marginTop: '36px' }}>Custom Dictionary</h3>

            <div className="setting-card">
                <p className="setting-card-desc">
                    Fix words the AI keeps getting wrong — phonetic replacements that run <strong>before</strong> grammar correction.
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
