import { useState } from "react";
import type { DictEntry } from "../../hooks/usePersonalization";
import { IconBook, IconTrash } from "../Icons";

interface DictionaryTabProps {
    dictionary: DictEntry[];
    addDictEntry: (entry: Omit<DictEntry, "id">) => void;
    updateDictEntry: (id: string, updates: Partial<Omit<DictEntry, "id">>) => void;
    removeDictEntry: (id: string) => void;
}

export function DictionaryTab({
    dictionary,
    addDictEntry,
    updateDictEntry,
    removeDictEntry,
}: DictionaryTabProps) {
    const [newSoundsLike, setNewSoundsLike] = useState("");
    const [newCorrect, setNewCorrect] = useState("");

    const handleAdd = () => {
        const sl = newSoundsLike.trim();
        const co = newCorrect.trim();
        if (!sl || !co) return;
        addDictEntry({ soundsLike: sl, correct: co });
        setNewSoundsLike("");
        setNewCorrect("");
    };

    const handleKeyDown = (e: React.KeyboardEvent) => {
        if (e.key === "Enter") handleAdd();
    };

    return (
        <div className="dict-tab">
            <h3 className="settings-section-title">Custom Dictionary</h3>

            <div className="setting-card">
                <p className="setting-card-desc">
                    Fix words the AI keeps getting wrong — proper nouns, names, technical terms.
                    These replacements run <strong>before</strong> grammar correction.
                </p>

                {/* ── Add entry row ───────────────────────────────────── */}
                <div className="dict-add-row">
                    <div className="dict-add-fields">
                        <div className="dict-field">
                            <label className="dict-field-label">Sounds like</label>
                            <input
                                type="text"
                                className="dict-input"
                                placeholder="e.g. tor scribe"
                                value={newSoundsLike}
                                onChange={(e) => setNewSoundsLike(e.target.value)}
                                onKeyDown={handleKeyDown}
                            />
                        </div>
                        <span className="dict-arrow">→</span>
                        <div className="dict-field">
                            <label className="dict-field-label">Correct spelling</label>
                            <input
                                type="text"
                                className="dict-input"
                                placeholder="e.g. Taurscribe"
                                value={newCorrect}
                                onChange={(e) => setNewCorrect(e.target.value)}
                                onKeyDown={handleKeyDown}
                            />
                        </div>
                    </div>
                    <button
                        className="ghost-btn ghost-btn--confirm"
                        onClick={handleAdd}
                        disabled={!newSoundsLike.trim() || !newCorrect.trim()}
                        title="Add entry"
                    >
                        + Add
                    </button>
                </div>

                {/* ── Entry list ──────────────────────────────────────── */}
                {dictionary.length === 0 ? (
                    <div className="dict-empty">
                        <span className="dict-empty-icon"><IconBook size={28} /></span>
                        <span>No entries yet — add words the AI keeps getting wrong.</span>
                    </div>
                ) : (
                    <div className="dict-list">
                        {dictionary.map((entry) => (
                            <div key={entry.id} className="dict-entry">
                                <input
                                    type="text"
                                    className="dict-input dict-input--inline"
                                    value={entry.soundsLike}
                                    onChange={(e) =>
                                        updateDictEntry(entry.id, { soundsLike: e.target.value })
                                    }
                                    title="What it sounds like"
                                />
                                <span className="dict-arrow-sm">→</span>
                                <input
                                    type="text"
                                    className="dict-input dict-input--inline"
                                    value={entry.correct}
                                    onChange={(e) =>
                                        updateDictEntry(entry.id, { correct: e.target.value })
                                    }
                                    title="Correct spelling"
                                />
                                <button
                                    className="dict-delete"
                                    onClick={() => removeDictEntry(entry.id)}
                                    title="Remove entry"
                                    aria-label="Remove"
                                >
                                    <IconTrash size={14} />
                                </button>
                            </div>
                        ))}
                    </div>
                )}
            </div>

            <p className="dict-tab-note">
                <strong>Tip:</strong> Enter words exactly how Whisper/Parakeet mistranscribes them in the "Sounds like" field.
            </p>
        </div>
    );
}
