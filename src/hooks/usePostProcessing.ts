import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Store } from "@tauri-apps/plugin-store";
import { useSyncedRef } from "../utils/useSyncedRef";

/**
 * Manages LLM grammar correction toggle state.
 * All settings are persisted via the storeRef passed in from App.tsx
 * (single Store instance shared with the rest of the app — no second load).
 *
 * Persisted keys:
 *   enable_grammar_lm   boolean
 *   transcription_style string
 *   llm_backend         "gpu" | "cpu"
 *   asr_backend         "gpu" | "cpu"
 *   enable_denoise      boolean
 *   enable_overlay      boolean
 *   mute_background_audio boolean
 */
export function usePostProcessing(
    setHeaderStatus: (msg: string, dur?: number, isProcessing?: boolean) => void,
    onOpenSettings?: () => void,
    storeRef?: React.RefObject<Store | null>
) {
    const [llmStatus, setLlmStatus] = useState("Not Loaded");
    const [enableGrammarLM, setEnableGrammarLMState] = useState(false);
    const [enableDenoise, setEnableDenoiseState] = useState(false);
    const [enableOverlay, setEnableOverlayState] = useState(true);
    const [muteBackgroundAudio, setMuteBackgroundAudioState] = useState(false);
    const [llmBackend, setLlmBackendState] = useState<"gpu" | "cpu">("gpu");
    const [asrBackend, setAsrBackendState] = useState<"gpu" | "cpu">("gpu");
    const [transcriptionStyle, setTranscriptionStyleState] = useState("Casual");

    // Gate auto-load effects until settings are loaded from store,
    // so the LLM initialises with the correct backend from the start.
    const [settingsLoaded, setSettingsLoaded] = useState(false);

    // useSyncedRef replaces the 6 useRef + useEffect sync pairs
    const enableGrammarLMRef = useSyncedRef(enableGrammarLM);
    const enableDenoiseRef = useSyncedRef(enableDenoise);
    const enableOverlayRef = useSyncedRef(enableOverlay);
    const muteBackgroundAudioRef = useSyncedRef(muteBackgroundAudio);
    const transcriptionStyleRef = useSyncedRef(transcriptionStyle);
    const llmBackendRef = useSyncedRef(llmBackend);
    const llmStatusRef = useSyncedRef(llmStatus);

    // ── Load persisted settings on mount ──────────────────────────────────
    // Uses the shared storeRef if provided; falls back to a local load so the
    // hook remains self-contained when used in isolation (e.g. tests).
    useEffect(() => {
        const load = async (store: Store) => {
            const grammarLM = await store.get<boolean>("enable_grammar_lm");
            const denoise = await store.get<boolean>("enable_denoise");
            const overlay = await store.get<boolean>("enable_overlay");
            const muteBg = await store.get<boolean>("mute_background_audio");
            const style = await store.get<string>("transcription_style");
            const backend = await store.get<"gpu" | "cpu">("llm_backend");
            const asrBe = await store.get<"gpu" | "cpu">("asr_backend");

            if (grammarLM != null) setEnableGrammarLMState(grammarLM);
            if (denoise != null) setEnableDenoiseState(denoise);
            if (overlay != null) setEnableOverlayState(overlay);
            if (muteBg != null) setMuteBackgroundAudioState(muteBg);
            if (style != null) setTranscriptionStyleState(style);
            if (backend != null) setLlmBackendState(backend);
            if (asrBe != null) setAsrBackendState(asrBe);

            setSettingsLoaded(true);
        };

        const init = async () => {
            try {
                // Poll for the shared storeRef (populated async by useInitialLoad)
                if (storeRef) {
                    let attempts = 0;
                    while (!storeRef.current && attempts < 50) {
                        await new Promise(r => setTimeout(r, 100));
                        attempts++;
                    }
                    if (storeRef.current) {
                        await load(storeRef.current);
                        return;
                    }
                }
                // Fallback: load independently (no storeRef provided or timed out)
                const { Store } = await import("@tauri-apps/plugin-store");
                const store = await Store.load("settings.json");
                await load(store);
            } catch (err) {
                console.error("Failed to load post-processing settings:", err);
                setSettingsLoaded(true); // still allow auto-load to proceed
            }
        };

        init();
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    // ── Persist helper ────────────────────────────────────────────────────
    const persist = useCallback((key: string, value: unknown) => {
        // Use shared store if available; the fallback path is a no-op until
        // the store is ready (settings will be re-written on next change).
        const store = storeRef?.current;
        if (!store) return;
        store.set(key, value)
            .then(() => store.save())
            .catch(console.error);
    }, [storeRef]);

    // ── Public setters — update state AND persist ─────────────────────────
    const setEnableGrammarLM = useCallback((val: boolean) => {
        setEnableGrammarLMState(val);
        persist("enable_grammar_lm", val);
    }, [persist]);

    const setEnableDenoise = useCallback((val: boolean) => {
        setEnableDenoiseState(val);
        persist("enable_denoise", val);
    }, [persist]);

    const setEnableOverlay = useCallback((val: boolean) => {
        setEnableOverlayState(val);
        persist("enable_overlay", val);
    }, [persist]);

    const setMuteBackgroundAudio = useCallback((val: boolean) => {
        setMuteBackgroundAudioState(val);
        persist("mute_background_audio", val);
    }, [persist]);

    const setTranscriptionStyle = useCallback((val: string) => {
        setTranscriptionStyleState(val);
        persist("transcription_style", val);
    }, [persist]);

    const setLlmBackend = useCallback((val: "gpu" | "cpu") => {
        setLlmBackendState(val);
        persist("llm_backend", val);
    }, [persist]);

    const setAsrBackend = useCallback((val: "gpu" | "cpu") => {
        setAsrBackendState(val);
        persist("asr_backend", val);
    }, [persist]);

    // ── Auto-load / unload LLM ────────────────────────────────────────────
    // Gated on settingsLoaded so the correct backend is always used.
    useEffect(() => {
        if (!settingsLoaded) return;
        if (enableGrammarLM && llmStatus === "Not Loaded") {
            invoke("check_grammar_llm_available").then((available) => {
                if (!available) {
                    setLlmStatus("Not Downloaded");
                    setEnableGrammarLM(false);
                    setHeaderStatus("Grammar LLM not downloaded. Open Settings > Models to download FlowScribe Qwen.", 8000);
                    onOpenSettings?.();
                    return;
                }
                setHeaderStatus("Auto-loading FlowScribe LLM...", 60_000);
                setLlmStatus("Loading...");
                invoke("init_llm", { useGpu: llmBackend === "gpu" }).then((res) => {
                    setLlmStatus("Loaded");
                    setHeaderStatus(res as string);
                }).catch((err) => {
                    setLlmStatus("Error");
                    setHeaderStatus("LLM Load Failed (check logs): " + err, 5000);
                });
            }).catch(() => {
                setLlmStatus("Not Downloaded");
                setHeaderStatus("Grammar LLM not downloaded. Open Settings > Models to download FlowScribe Qwen.", 8000);
                onOpenSettings?.();
            });
        } else if (!enableGrammarLM) {
            if (llmStatus === "Loaded") {
                setLlmStatus("Loading...");
                invoke("unload_llm").then(() => {
                    setLlmStatus("Not Loaded");
                    setHeaderStatus("FlowScribe LLM unloaded");
                }).catch((e) => {
                    setLlmStatus("Error");
                    setHeaderStatus(`Failed to unload: ${e}`, 5000);
                });
            }
        }
    }, [enableGrammarLM, llmStatus, llmBackend, settingsLoaded, setEnableGrammarLM]);

    // ── Hot-reload LLM when backend (GPU ↔ CPU) changes while loaded ──────
    useEffect(() => {
        // Skip on first render — llmBackendRef starts as the initial value
        if (llmBackendRef.current === llmBackend) return;

        // Only hot-reload if the LLM is currently active
        if (llmStatusRef.current !== "Loaded") return;

        const label = llmBackend === "gpu" ? "GPU" : "CPU";
        setHeaderStatus(`Switching LLM to ${label}…`, 60_000);
        setLlmStatus("Loading...");

        invoke("unload_llm")
            .then(() => invoke("init_llm", { useGpu: llmBackend === "gpu" }))
            .then((res) => {
                setLlmStatus("Loaded");
                setHeaderStatus(res as string);
            })
            .catch((err) => {
                setLlmStatus("Error");
                setHeaderStatus(`LLM backend switch failed: ${err}`, 5000);
            });
    }, [llmBackend, setHeaderStatus]);

    // ── Re-check LLM availability when models change (e.g. after download) ──
    useEffect(() => {
        if (llmStatus !== "Not Downloaded") return;
        let unlisten: (() => void) | undefined;
        listen("models-changed", () => {
            invoke("check_grammar_llm_available").then((available) => {
                if (available) setLlmStatus("Not Loaded");
            });
        }).then((fn) => { unlisten = fn; });
        return () => { unlisten?.(); };
    }, [llmStatus]);

    return {
        llmStatus,
        enableGrammarLM,
        setEnableGrammarLM,
        enableGrammarLMRef,
        transcriptionStyle,
        setTranscriptionStyle,
        transcriptionStyleRef,
        enableDenoise,
        setEnableDenoise,
        enableDenoiseRef,
        enableOverlay,
        setEnableOverlay,
        enableOverlayRef,
        muteBackgroundAudio,
        setMuteBackgroundAudio,
        muteBackgroundAudioRef,
        llmBackend,
        setLlmBackend,
        asrBackend,
        setAsrBackend,
    };
}
