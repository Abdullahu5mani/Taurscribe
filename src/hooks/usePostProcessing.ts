import { useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Store } from "@tauri-apps/plugin-store";

/**
 * Manages LLM grammar correction and SymSpell spell-check toggle state.
 * All settings are persisted to settings.json and restored on startup.
 *
 * Persisted keys:
 *   enable_grammar_lm   boolean
 *   enable_spell_check  boolean
 *   transcription_style string
 *   llm_backend         "gpu" | "cpu"
 */
export function usePostProcessing(setHeaderStatus: (msg: string, dur?: number, isProcessing?: boolean) => void) {
    const [llmStatus, setLlmStatus] = useState("Not Loaded");
    const [enableGrammarLM, setEnableGrammarLMState] = useState(false);
    const [enableSpellCheck, setEnableSpellCheckState] = useState(false);
    const [spellCheckStatus, setSpellCheckStatus] = useState("Not Loaded");
    const [llmBackend, setLlmBackendState] = useState<"gpu" | "cpu">("gpu");
    const [transcriptionStyle, setTranscriptionStyleState] = useState("Auto");

    // Gate auto-load effects until settings are loaded from store,
    // so the LLM initialises with the correct backend from the start.
    const [settingsLoaded, setSettingsLoaded] = useState(false);

    const enableGrammarLMRef = useRef(enableGrammarLM);
    const enableSpellCheckRef = useRef(enableSpellCheck);
    const transcriptionStyleRef = useRef(transcriptionStyle);
    const storeRef = useRef<Store | null>(null);

    useEffect(() => { enableGrammarLMRef.current = enableGrammarLM; }, [enableGrammarLM]);
    useEffect(() => { enableSpellCheckRef.current = enableSpellCheck; }, [enableSpellCheck]);
    useEffect(() => { transcriptionStyleRef.current = transcriptionStyle; }, [transcriptionStyle]);

    // ── Load persisted settings on mount ──────────────────────────────────
    useEffect(() => {
        Store.load("settings.json")
            .then(async (store) => {
                storeRef.current = store;

                const grammarLM  = await store.get<boolean>("enable_grammar_lm");
                const spellCheck = await store.get<boolean>("enable_spell_check");
                const style      = await store.get<string>("transcription_style");
                const backend    = await store.get<"gpu" | "cpu">("llm_backend");

                if (grammarLM  != null) setEnableGrammarLMState(grammarLM);
                if (spellCheck != null) setEnableSpellCheckState(spellCheck);
                if (style      != null) setTranscriptionStyleState(style);
                if (backend    != null) setLlmBackendState(backend);

                setSettingsLoaded(true);
            })
            .catch((err) => {
                console.error("Failed to load post-processing settings:", err);
                setSettingsLoaded(true); // still allow auto-load to proceed
            });
    }, []);

    // ── Persist helper ────────────────────────────────────────────────────
    const persist = useCallback((key: string, value: unknown) => {
        if (!storeRef.current) return;
        storeRef.current
            .set(key, value)
            .then(() => storeRef.current?.save())
            .catch(console.error);
    }, []);

    // ── Public setters — update state AND persist ─────────────────────────
    const setEnableGrammarLM = useCallback((val: boolean) => {
        setEnableGrammarLMState(val);
        persist("enable_grammar_lm", val);
    }, [persist]);

    const setEnableSpellCheck = useCallback((val: boolean) => {
        setEnableSpellCheckState(val);
        persist("enable_spell_check", val);
    }, [persist]);

    const setTranscriptionStyle = useCallback((val: string) => {
        setTranscriptionStyleState(val);
        persist("transcription_style", val);
    }, [persist]);

    const setLlmBackend = useCallback((val: "gpu" | "cpu") => {
        setLlmBackendState(val);
        persist("llm_backend", val);
    }, [persist]);

    // ── Auto-load / unload LLM ────────────────────────────────────────────
    // Gated on settingsLoaded so the correct backend is always used.
    useEffect(() => {
        if (!settingsLoaded) return;
        if (enableGrammarLM && llmStatus === "Not Loaded") {
            setHeaderStatus("Auto-loading Qwen LLM...", 60_000);
            setLlmStatus("Loading...");
            invoke("init_llm", { useGpu: llmBackend === "gpu" }).then((res) => {
                setLlmStatus("Loaded");
                setHeaderStatus(res as string);
            }).catch((err) => {
                setLlmStatus("Error");
                setHeaderStatus("LLM Load Failed (check logs): " + err, 5000);
            });
        } else if (!enableGrammarLM && llmStatus === "Loaded") {
            setLlmStatus("Loading...");
            invoke("unload_llm").then(() => {
                setLlmStatus("Not Loaded");
                setHeaderStatus("Qwen LLM unloaded");
            }).catch((e) => {
                setLlmStatus("Error");
                setHeaderStatus(`Failed to unload: ${e}`, 5000);
            });
        }
    }, [enableGrammarLM, llmStatus, llmBackend, settingsLoaded]);

    // ── Auto-load / unload SpellCheck ─────────────────────────────────────
    useEffect(() => {
        if (!settingsLoaded) return;
        if (enableSpellCheck && spellCheckStatus === "Not Loaded") {
            setHeaderStatus("Loading SymSpell dictionary...", 60_000);
            setSpellCheckStatus("Loading...");
            invoke("init_spellcheck").then((res) => {
                setSpellCheckStatus("Loaded");
                setHeaderStatus(res as string);
            }).catch((err) => {
                setSpellCheckStatus("Error");
                setHeaderStatus("SymSpell failed: " + err, 5000);
            });
        } else if (!enableSpellCheck && spellCheckStatus === "Loaded") {
            setSpellCheckStatus("Loading...");
            invoke("unload_spellcheck").then(() => {
                setSpellCheckStatus("Not Loaded");
                setHeaderStatus("SymSpell unloaded");
            }).catch((e) => {
                setSpellCheckStatus("Error");
                setHeaderStatus(`Failed to unload: ${e}`, 5000);
            });
        }
    }, [enableSpellCheck, spellCheckStatus, settingsLoaded]);

    return {
        llmStatus,
        enableGrammarLM,
        setEnableGrammarLM,
        enableGrammarLMRef,
        transcriptionStyle,
        setTranscriptionStyle,
        transcriptionStyleRef,
        enableSpellCheck,
        setEnableSpellCheck,
        enableSpellCheckRef,
        spellCheckStatus,
        llmBackend,
        setLlmBackend,
    };
}
