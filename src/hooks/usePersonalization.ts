import { useState, useEffect, useRef, useCallback } from "react";
import { Store } from "@tauri-apps/plugin-store";

// ── Types ────────────────────────────────────────────────────────────────────

export interface DictEntry {
    id: string;
    soundsLike: string;
    correct: string;
}

export interface SnippetEntry {
    id: string;
    trigger: string;
    expansion: string;
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/** Escape special regex chars so user-entered text doesn't break patterns */
function escapeRegex(str: string): string {
    return str.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

let _idCounter = 0;
export function genId(): string {
    return `${Date.now()}_${++_idCounter}`;
}

// ── Core replacement functions ───────────────────────────────────────────────

/**
 * Apply custom dictionary substitutions (case-insensitive, whole-word).
 * Runs early in the pipeline to fix proper nouns before grammar correction.
 */
export function applyDictionary(text: string, dict: DictEntry[]): string {
    if (!dict.length) return text;
    let result = text;
    for (const entry of dict) {
        if (!entry.soundsLike.trim() || !entry.correct.trim()) continue;
        const regex = new RegExp(`\\b${escapeRegex(entry.soundsLike)}\\b`, "gi");
        result = result.replace(regex, entry.correct);
    }
    return result;
}

/**
 * Expand text snippets (case-insensitive, whole-word).
 * Runs last in the pipeline so the LLM doesn't mangle expanded text.
 */
export function applySnippets(text: string, snippets: SnippetEntry[]): string {
    if (!snippets.length) return text;
    let result = text;
    for (const snippet of snippets) {
        if (!snippet.trigger.trim() || !snippet.expansion.trim()) continue;
        const regex = new RegExp(`\\b${escapeRegex(snippet.trigger)}\\b`, "gi");
        result = result.replace(regex, snippet.expansion);
    }
    return result;
}

// ── Vocabulary Presets ────────────────────────────────────────────────────────

export const VOCAB_PRESETS = {
    developer: [
        "Taurscribe", "TypeScript", "JavaScript", "Rust", "GitHub", "API",
        "GraphQL", "async", "await", "useCallback", "useEffect", "Docker",
        "Kubernetes", "PostgreSQL", "TailwindCSS"
    ],
    medical: [
        "hypertension", "tachycardia", "myocardial", "infarction", "dyspnea",
        "erythema", "acetaminophen", "ibuprofen", "amoxicillin", "metformin",
        "lisinopril", "hypoglycemia", "electrocardiogram"
    ],
    legal: [
        "affidavit", "indemnification", "jurisdiction", "force majeure", "subpoena",
        "plaintiff", "defendant", "liability", "arbitration", "confidentiality",
        "intellectual property", "severability"
    ]
} as const;

// ── Hook ─────────────────────────────────────────────────────────────────────

/**
 * Manages custom dictionary entries, text snippets, and custom vocabulary jargon injection.
 * Persisted to settings.json and restored on startup.
 *
 * Persisted keys:
 *   custom_dictionary    DictEntry[]
 *   snippets             SnippetEntry[]
 *   custom_vocabulary    string[]
 *   context_bias_enabled boolean
 */
export function usePersonalization() {
    const [dictionary, setDictionaryState] = useState<DictEntry[]>([]);
    const [snippets, setSnippetsState] = useState<SnippetEntry[]>([]);
    const [customVocabulary, setCustomVocabularyState] = useState<string[]>([]);
    const [contextBiasEnabled, setContextBiasEnabledState] = useState<boolean>(true);
    const [loaded, setLoaded] = useState(false);

    // Refs for use in the recording pipeline (avoids stale closure issues)
    const dictionaryRef = useRef<DictEntry[]>(dictionary);
    const snippetsRef = useRef<SnippetEntry[]>(snippets);

    useEffect(() => { dictionaryRef.current = dictionary; }, [dictionary]);
    useEffect(() => { snippetsRef.current = snippets; }, [snippets]);

    const storeRef = useRef<Store | null>(null);

    // ── Load from store on mount ─────────────────────────────────────────
    useEffect(() => {
        Store.load("settings.json")
            .then(async (store) => {
                storeRef.current = store;

                const savedDict = await store.get<DictEntry[]>("custom_dictionary");
                const savedSnippets = await store.get<SnippetEntry[]>("snippets");
                const savedVocab = await store.get<string[]>("custom_vocabulary");
                const savedBias = await store.get<boolean>("context_bias_enabled");

                if (savedDict && Array.isArray(savedDict)) setDictionaryState(savedDict);
                if (savedSnippets && Array.isArray(savedSnippets)) setSnippetsState(savedSnippets);
                if (savedVocab && Array.isArray(savedVocab)) setCustomVocabularyState(savedVocab);
                if (savedBias !== null && savedBias !== undefined) setContextBiasEnabledState(Boolean(savedBias));

                setLoaded(true);
            })
            .catch((err) => {
                console.error("Failed to load personalization settings:", err);
                setLoaded(true);
            });
    }, []);

    // ── Persist helper ───────────────────────────────────────────────────
    const persist = useCallback((key: string, value: unknown) => {
        if (!storeRef.current) return;
        storeRef.current
            .set(key, value)
            .then(() => storeRef.current?.save())
            .catch(console.error);
    }, []);

    // ── Dictionary operations ────────────────────────────────────────────
    const addDictEntry = useCallback((entry: Omit<DictEntry, "id">) => {
        setDictionaryState((prev) => {
            const next = [...prev, { ...entry, id: genId() }];
            persist("custom_dictionary", next);
            return next;
        });
    }, [persist]);

    const updateDictEntry = useCallback((id: string, updates: Partial<Omit<DictEntry, "id">>) => {
        setDictionaryState((prev) => {
            const next = prev.map((e) => (e.id === id ? { ...e, ...updates } : e));
            persist("custom_dictionary", next);
            return next;
        });
    }, [persist]);

    const removeDictEntry = useCallback((id: string) => {
        setDictionaryState((prev) => {
            const next = prev.filter((e) => e.id !== id);
            persist("custom_dictionary", next);
            return next;
        });
    }, [persist]);

    // ── Snippet operations ───────────────────────────────────────────────
    const addSnippet = useCallback((entry: Omit<SnippetEntry, "id">) => {
        setSnippetsState((prev) => {
            const next = [...prev, { ...entry, id: genId() }];
            persist("snippets", next);
            return next;
        });
    }, [persist]);

    const updateSnippet = useCallback((id: string, updates: Partial<Omit<SnippetEntry, "id">>) => {
        setSnippetsState((prev) => {
            const next = prev.map((e) => (e.id === id ? { ...e, ...updates } : e));
            persist("snippets", next);
            return next;
        });
    }, [persist]);

    const removeSnippet = useCallback((id: string) => {
        setSnippetsState((prev) => {
            const next = prev.filter((e) => e.id !== id);
            persist("snippets", next);
            return next;
        });
    }, [persist]);

    // ── Custom Vocabulary operations ────────────────────────────────────
    const addVocabWord = useCallback((rawWord: string) => {
        const word = rawWord.trim();
        if (!word) return;
        setCustomVocabularyState((prev) => {
            if (prev.some((w) => w.toLowerCase() === word.toLowerCase())) return prev;
            const next = [...prev, word];
            persist("custom_vocabulary", next);
            return next;
        });
    }, [persist]);

    const removeVocabWord = useCallback((wordToRemove: string) => {
        setCustomVocabularyState((prev) => {
            const next = prev.filter((w) => w !== wordToRemove);
            persist("custom_vocabulary", next);
            return next;
        });
    }, [persist]);

    const addVocabPreset = useCallback((category: keyof typeof VOCAB_PRESETS) => {
        const presetWords = VOCAB_PRESETS[category] || [];
        setCustomVocabularyState((prev) => {
            const existingLower = new Set(prev.map((w) => w.toLowerCase()));
            const toAdd = presetWords.filter((w) => !existingLower.has(w.toLowerCase()));
            if (toAdd.length === 0) return prev;
            const next = [...prev, ...toAdd];
            persist("custom_vocabulary", next);
            return next;
        });
    }, [persist]);

    const clearVocab = useCallback(() => {
        setCustomVocabularyState([]);
        persist("custom_vocabulary", []);
    }, [persist]);

    const setContextBiasEnabled = useCallback((enabled: boolean) => {
        setContextBiasEnabledState(enabled);
        persist("context_bias_enabled", enabled);
    }, [persist]);

    return {
        // Dictionary
        dictionary,
        dictionaryRef,
        addDictEntry,
        updateDictEntry,
        removeDictEntry,

        // Snippets
        snippets,
        snippetsRef,
        addSnippet,
        updateSnippet,
        removeSnippet,

        // Custom Vocabulary & Context Jargon Injection
        customVocabulary,
        contextBiasEnabled,
        addVocabWord,
        removeVocabWord,
        addVocabPreset,
        clearVocab,
        setContextBiasEnabled,

        loaded,
    };
}

