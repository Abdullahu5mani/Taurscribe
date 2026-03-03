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
 * Runs early in the pipeline to fix proper nouns before spell check.
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

// ── Hook ─────────────────────────────────────────────────────────────────────

/**
 * Manages custom dictionary entries and text snippets.
 * Both are persisted to settings.json and restored on startup.
 *
 * Persisted keys:
 *   custom_dictionary  DictEntry[]
 *   snippets           SnippetEntry[]
 */
export function usePersonalization() {
    const [dictionary, setDictionaryState] = useState<DictEntry[]>([]);
    const [snippets, setSnippetsState] = useState<SnippetEntry[]>([]);
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

                if (savedDict && Array.isArray(savedDict)) setDictionaryState(savedDict);
                if (savedSnippets && Array.isArray(savedSnippets)) setSnippetsState(savedSnippets);

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
    const setDictionary = useCallback((entries: DictEntry[]) => {
        setDictionaryState(entries);
        persist("custom_dictionary", entries);
    }, [persist]);

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
    const setSnippets = useCallback((entries: SnippetEntry[]) => {
        setSnippetsState(entries);
        persist("snippets", entries);
    }, [persist]);

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

    return {
        // Dictionary
        dictionary,
        dictionaryRef,
        setDictionary,
        addDictEntry,
        updateDictEntry,
        removeDictEntry,

        // Snippets
        snippets,
        snippetsRef,
        setSnippets,
        addSnippet,
        updateSnippet,
        removeSnippet,

        loaded,
    };
}
