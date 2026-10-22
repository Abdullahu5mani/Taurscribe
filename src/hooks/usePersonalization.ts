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
 * Case-insensitive whole-word pattern for user text. `\b` is ASCII-only and
 * needs a word character on the inside, so it never matched "café" or
 * triggers like "/sig"; Unicode lookarounds treat any letter/digit as a word
 * character and work whatever the term starts or ends with.
 */
function wholeWordRegex(term: string): RegExp {
    return new RegExp(`(?<![\\p{L}\\p{N}_])${escapeRegex(term)}(?![\\p{L}\\p{N}_])`, "giu");
}

/**
 * Apply custom dictionary substitutions (case-insensitive, whole-word).
 * Runs early in the pipeline to fix proper nouns before grammar correction.
 */
export function applyDictionary(text: string, dict: DictEntry[]): string {
    return replaceWholeWords(text, dict.map(entry => [entry.soundsLike, entry.correct]));
}

/**
 * Replaces every whole-word, case-insensitive occurrence of each term in ONE
 * pass over the original text. Applying entries one after another let a
 * replacement be rewritten again: "jon"→"John" followed by "john"→"Jonathan"
 * turned "jon" into "Jonathan", and a snippet whose expansion contained another
 * trigger was expanded twice. Longer terms win where terms overlap; for a
 * repeated term the first entry wins.
 */
function replaceWholeWords(text: string, pairs: [string, string][]): string {
    const replacements = new Map<string, string>();
    const terms: string[] = [];
    for (const [rawTerm, replacement] of pairs) {
        const term = rawTerm.trim();
        if (!term || !replacement.trim()) continue;
        const key = term.toLowerCase();
        if (replacements.has(key)) continue;
        replacements.set(key, replacement);
        terms.push(term);
    }
    if (!terms.length) return text;
    terms.sort((a, b) => b.length - a.length);
    const pattern = new RegExp(
        `(?<![\\p{L}\\p{N}_])(?:${terms.map(escapeRegex).join("|")})(?![\\p{L}\\p{N}_])`,
        "giu",
    );
    // A replacer function inserts the text as-is ("$&" or "$$" in a string would be expanded).
    return text.replace(pattern, match => {
        const direct = replacements.get(match.toLowerCase());
        if (direct !== undefined) return direct;
        // Case folding that changes length (e.g. "ß"): find the term that matches.
        const term = terms.find(t => wholeWordRegex(t).test(match));
        return term !== undefined ? replacements.get(term.toLowerCase()) ?? match : match;
    });
}

/**
 * Expand text snippets (case-insensitive, whole-word).
 * Runs last in the pipeline so the LLM doesn't mangle expanded text.
 */
export function applySnippets(text: string, snippets: SnippetEntry[]): string {
    return replaceWholeWords(text, snippets.map(snippet => [snippet.trigger, snippet.expansion]));
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

    type Values = {
        custom_dictionary: DictEntry[];
        snippets: SnippetEntry[];
        custom_vocabulary: string[];
        context_bias_enabled: boolean;
    };
    type Key = keyof Values;
    type Edit = (values: Values) => Values;
    const valuesRef = useRef<Values>({ custom_dictionary: [], snippets: [], custom_vocabulary: [], context_bias_enabled: true });
    const pendingRef = useRef<{ key: Key; edit: Edit }[]>([]);
    const readyRef = useRef(false);
    const storeRef = useRef<Store | null>(null);
    const writesRef = useRef<Promise<void>>(Promise.resolve());

    const display = useCallback((values: Values) => {
        valuesRef.current = values;
        dictionaryRef.current = values.custom_dictionary;
        snippetsRef.current = values.snippets;
        setDictionaryState(values.custom_dictionary);
        setSnippetsState(values.snippets);
        setCustomVocabularyState(values.custom_vocabulary);
        setContextBiasEnabledState(values.context_bias_enabled);
    }, []);
    const persist = useCallback((key: Key, value: Values[Key]) => {
        const store = storeRef.current;
        if (!store) return;
        // Serialize writes so a slower earlier save cannot replace a later edit.
        writesRef.current = writesRef.current.then(async () => {
            await store.set(key, value);
            await store.save();
        }).catch(console.error);
    }, []);
    const mutate = useCallback(<K extends Key,>(key: K, update: (value: Values[K]) => Values[K]) => {
        const edit: Edit = values => ({ ...values, [key]: update(values[key]) });
        const next = edit(valuesRef.current);
        display(next);
        if (!readyRef.current) pendingRef.current.push({ key, edit });
        else persist(key, next[key]);
    }, [display, persist]);

    useEffect(() => {
        let disposed = false;
        Store.load("settings.json").then(async store => {
            const [savedDict, savedSnippets, savedVocab, savedBias] = await Promise.all([
                store.get<DictEntry[]>("custom_dictionary"), store.get<SnippetEntry[]>("snippets"),
                store.get<string[]>("custom_vocabulary"), store.get<boolean>("context_bias_enabled"),
            ]);
            if (disposed) return;
            storeRef.current = store;
            let values: Values = {
                custom_dictionary: Array.isArray(savedDict) ? savedDict : [],
                snippets: Array.isArray(savedSnippets) ? savedSnippets : [],
                custom_vocabulary: Array.isArray(savedVocab) ? savedVocab : [],
                context_bias_enabled: savedBias == null ? true : Boolean(savedBias),
            };
            // Replay user actions over the saved state, including edits made
            // while get() was pending. Never replace saved entries with [] + edit.
            const pending = pendingRef.current;
            for (const { edit } of pending) values = edit(values);
            pendingRef.current = [];
            readyRef.current = true;
            display(values);
            for (const key of new Set(pending.map(edit => edit.key))) persist(key, values[key]);
            setLoaded(true);
        }).catch(err => {
            if (!disposed) {
                console.error("Failed to load personalization settings:", err);
                setLoaded(true);
            }
        });
        return () => { disposed = true; };
    }, [display, persist]);

    const addDictEntry = useCallback((entry: Omit<DictEntry, "id">) => {
        const item = { ...entry, id: genId() };
        mutate("custom_dictionary", prev => [...prev, item]);
    }, [mutate]);
    const updateDictEntry = useCallback((id: string, updates: Partial<Omit<DictEntry, "id">>) => {
        mutate("custom_dictionary", prev => prev.map(e => e.id === id ? { ...e, ...updates } : e));
    }, [mutate]);
    const removeDictEntry = useCallback((id: string) => {
        mutate("custom_dictionary", prev => prev.filter(e => e.id !== id));
    }, [mutate]);
    const addSnippet = useCallback((entry: Omit<SnippetEntry, "id">) => {
        const item = { ...entry, id: genId() };
        mutate("snippets", prev => [...prev, item]);
    }, [mutate]);
    const updateSnippet = useCallback((id: string, updates: Partial<Omit<SnippetEntry, "id">>) => {
        mutate("snippets", prev => prev.map(e => e.id === id ? { ...e, ...updates } : e));
    }, [mutate]);
    const removeSnippet = useCallback((id: string) => {
        mutate("snippets", prev => prev.filter(e => e.id !== id));
    }, [mutate]);
    const addVocabWord = useCallback((rawWord: string) => {
        const word = rawWord.trim();
        if (word) mutate("custom_vocabulary", prev => prev.some(w => w.toLowerCase() === word.toLowerCase()) ? prev : [...prev, word]);
    }, [mutate]);
    const removeVocabWord = useCallback((word: string) => {
        mutate("custom_vocabulary", prev => prev.filter(w => w !== word));
    }, [mutate]);
    const addVocabPreset = useCallback((category: keyof typeof VOCAB_PRESETS) => {
        mutate("custom_vocabulary", prev => {
            const existing = new Set(prev.map(w => w.toLowerCase()));
            return [...prev, ...(VOCAB_PRESETS[category] || []).filter(w => !existing.has(w.toLowerCase()))];
        });
    }, [mutate]);
    const clearVocab = useCallback(() => mutate("custom_vocabulary", () => []), [mutate]);
    const setContextBiasEnabled = useCallback((enabled: boolean) => mutate("context_bias_enabled", () => enabled), [mutate]);

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

