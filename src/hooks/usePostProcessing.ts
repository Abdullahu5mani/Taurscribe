import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";

/**
 * Manages LLM grammar correction and SymSpell spell-check toggle state,
 * including auto-loading/unloading when the feature is toggled.
 */
export function usePostProcessing(setHeaderStatus: (msg: string, dur?: number, isProcessing?: boolean) => void) {
    const [llmStatus, setLlmStatus] = useState("Not Loaded");
    const [enableGrammarLM, setEnableGrammarLM] = useState(false);

    const [enableSpellCheck, setEnableSpellCheck] = useState(false);
    const [spellCheckStatus, setSpellCheckStatus] = useState("Not Loaded");

    // Refs so recording handlers always read the latest values without re-subscribing
    const enableGrammarLMRef = useRef(enableGrammarLM);
    const enableSpellCheckRef = useRef(enableSpellCheck);

    useEffect(() => { enableGrammarLMRef.current = enableGrammarLM; }, [enableGrammarLM]);
    useEffect(() => { enableSpellCheckRef.current = enableSpellCheck; }, [enableSpellCheck]);

    // Auto-init/Unload LLM when enabled/disabled
    useEffect(() => {
        if (enableGrammarLM && llmStatus === "Not Loaded") {
            setHeaderStatus("Auto-loading Qwen LLM...", 60_000);
            setLlmStatus("Loading...");
            invoke("init_llm").then((res) => {
                setLlmStatus("Loaded");
                setHeaderStatus(res as string);
            }).catch(() => setLlmStatus("Error"));
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
    }, [enableGrammarLM, llmStatus]);

    // Auto-init/Unload SpellCheck when enabled/disabled
    useEffect(() => {
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
    }, [enableSpellCheck, spellCheckStatus]);

    return {
        llmStatus,
        enableGrammarLM,
        setEnableGrammarLM,
        enableGrammarLMRef,
        enableSpellCheck,
        setEnableSpellCheck,
        enableSpellCheckRef,
        spellCheckStatus,
    };
}
