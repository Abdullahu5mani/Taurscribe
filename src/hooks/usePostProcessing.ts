import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Store } from "@tauri-apps/plugin-store";
import { useSyncedRef } from "../utils/useSyncedRef";
import { serialTasks } from "../utils/serialTasks";

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
 *   compute_device      "auto" or a GPU id (gpu.rs); used when asr_backend is "gpu"
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
    const [enableDenoise, setEnableDenoiseState] = useState(true);
    const [enableOverlay, setEnableOverlayState] = useState(true);
    const [muteBackgroundAudio, setMuteBackgroundAudioState] = useState(false);
    const [llmBackend, setLlmBackendState] = useState<"gpu" | "cpu">("gpu");
    const [asrBackend, setAsrBackendState] = useState<"gpu" | "cpu">("gpu");
    const [computeDevice, setComputeDeviceState] = useState("auto");
    const [transcriptionStyle, setTranscriptionStyleState] = useState("Clean");

    // Gate auto-load effects until settings are loaded from store,
    // so the LLM initialises with the correct backend from the start.
    const [settingsLoaded, setSettingsLoaded] = useState(false);
    const settingsStoreRef = useRef<Store | null>(null);
    const changedSettingsRef = useRef(new Set<string>());
    const pendingSettingsRef = useRef(new Map<string, unknown>());
    const persistQueueRef = useRef(serialTasks());
    const deviceQueueRef = useRef(serialTasks());

    // useSyncedRef replaces the 6 useRef + useEffect sync pairs
    const enableGrammarLMRef = useSyncedRef(enableGrammarLM);
    const enableDenoiseRef = useSyncedRef(enableDenoise);
    const enableOverlayRef = useSyncedRef(enableOverlay);
    const muteBackgroundAudioRef = useSyncedRef(muteBackgroundAudio);
    const transcriptionStyleRef = useSyncedRef(transcriptionStyle);
    // The backend the loaded FlowScribe engine was started with (null: none).
    // Compared with the setting to decide when a reload is due.
    const appliedLlmBackendRef = useRef<{ backend: "gpu" | "cpu"; device: string } | null>(null);
    const llmOperationRef = useRef(false);
    const llmRevisionRef = useRef(0);
    const llmErrorRevisionRef = useRef<number | null>(null);
    const mountedRef = useRef(true);
    const llmIntentRef = useRef({ enabled: enableGrammarLM, backend: llmBackend, device: computeDevice });
    llmIntentRef.current = { enabled: enableGrammarLM, backend: llmBackend, device: computeDevice };
    useEffect(() => {
        mountedRef.current = true;
        return () => { mountedRef.current = false; };
    }, []);

    // ── Load persisted settings on mount ──────────────────────────────────
    // Uses the shared storeRef if provided; falls back to a local load so the
    // hook remains self-contained when used in isolation (e.g. tests).
    useEffect(() => {
        const load = async (store: Store) => {
            settingsStoreRef.current = store;
            // Enqueue pre-store choices before exposing the store to new writes.
            // Otherwise hydration could replay an old choice after a newer one.
            for (const [key, value] of pendingSettingsRef.current) {
                void persistQueueRef.current(async () => {
                    await store.set(key, value);
                    await store.save();
                }).catch(console.error);
            }
            pendingSettingsRef.current.clear();
            const grammarLM = await store.get<boolean>("enable_grammar_lm");
            const denoise = await store.get<boolean>("enable_denoise");
            const overlay = await store.get<boolean>("enable_overlay");
            const muteBg = await store.get<boolean>("mute_background_audio");
            const style = await store.get<string>("transcription_style");
            const backend = await store.get<"gpu" | "cpu">("llm_backend");
            const asrBe = await store.get<"gpu" | "cpu">("asr_backend");
            const device = await store.get<string>("compute_device");

            if (grammarLM != null && !changedSettingsRef.current.has("enable_grammar_lm")) setEnableGrammarLMState(grammarLM);
            if (denoise != null && !changedSettingsRef.current.has("enable_denoise")) setEnableDenoiseState(denoise);
            if (overlay != null && !changedSettingsRef.current.has("enable_overlay")) setEnableOverlayState(overlay);
            if (muteBg != null && !changedSettingsRef.current.has("mute_background_audio")) setMuteBackgroundAudioState(muteBg);
            // Older tone styles (Casual, Professional, ...) became the "Clean" level.
            if (style != null && !changedSettingsRef.current.has("transcription_style")) setTranscriptionStyleState(["Verbatim", "Clean", "Formatted"].includes(style) ? style : "Clean");
            if (backend != null && !changedSettingsRef.current.has("llm_backend")) setLlmBackendState(backend);
            if (asrBe != null && !changedSettingsRef.current.has("asr_backend")) setAsrBackendState(asrBe);
            if (device && !changedSettingsRef.current.has("compute_device")) setComputeDeviceState(device);

            if (mountedRef.current) setSettingsLoaded(true);
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
        changedSettingsRef.current.add(key);
        const store = storeRef?.current ?? settingsStoreRef.current;
        if (!store) {
            pendingSettingsRef.current.set(key, value);
            return;
        }
        void persistQueueRef.current(async () => {
            await store.set(key, value);
            await store.save();
        }).catch(console.error);
    }, [storeRef]);

    // ── Public setters — update state AND persist ─────────────────────────
    const setEnableGrammarLM = useCallback((val: boolean) => {
        llmRevisionRef.current += 1;
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
        llmRevisionRef.current += 1;
        setLlmBackendState(val);
        persist("llm_backend", val);
    }, [persist]);

    const setAsrBackend = useCallback((val: "gpu" | "cpu") => {
        setAsrBackendState(val);
        persist("asr_backend", val);
    }, [persist]);

    /** The GPU to run on; the backend applies it from the next model load. */
    const setComputeDevice = useCallback(async (id: string) => {
        // Publish the setting only after the backend can use it for a reload.
        await deviceQueueRef.current(async () => {
            await invoke("set_compute_device", { id });
            llmRevisionRef.current += 1;
            setComputeDeviceState(id);
            persist("compute_device", id);
        });
    }, [persist]);

    // One lifecycle operation at a time. Preferences made during an operation
    // are read again before loading, and reconciled once that operation settles.
    useEffect(() => {
        if (!settingsLoaded || llmOperationRef.current || llmStatus === "Not Downloaded") return;
        const applied = appliedLlmBackendRef.current;
        if (llmStatus === "Error" && llmErrorRevisionRef.current === llmRevisionRef.current) return;
        if (enableGrammarLM && llmStatus === "Loaded" && applied?.backend === llmBackend
            && (llmBackend === "cpu" || applied.device === computeDevice)) return;
        if (!enableGrammarLM && !applied) {
            if (llmStatus !== "Not Loaded") setLlmStatus("Not Loaded");
            return;
        }
        llmOperationRef.current = true;
        setLlmStatus("Loading...");
        const run = async () => {
            let revision = llmRevisionRef.current;
            try {
                if (applied) {
                    await invoke("unload_llm");
                    appliedLlmBackendRef.current = null;
                }
                if (!mountedRef.current) return;
                if (!llmIntentRef.current.enabled) {
                    setLlmStatus("Not Loaded");
                    return;
                }
                revision = llmRevisionRef.current;
                const available = await invoke<boolean>("check_grammar_llm_available");
                if (!mountedRef.current) return;
                if (revision !== llmRevisionRef.current || !llmIntentRef.current.enabled) {
                    setLlmStatus("Not Loaded");
                    return;
                }
                if (!available) {
                    setLlmStatus("Not Downloaded");
                    setEnableGrammarLM(false);
                    setHeaderStatus("FlowScribe is not downloaded. Open Settings > Models to download FlowScribe V3.", 8000);
                    onOpenSettings?.();
                    return;
                }
                const intent = llmIntentRef.current;
                setHeaderStatus("Loading FlowScribe LLM...", 60_000);
                const res = await invoke("init_llm", { useGpu: intent.backend === "gpu" });
                appliedLlmBackendRef.current = { backend: intent.backend, device: intent.device };
                if (mountedRef.current) {
                    setLlmStatus("Loaded");
                    setHeaderStatus(res as string);
                }
            } catch (err) {
                llmErrorRevisionRef.current = revision;
                if (mountedRef.current) {
                    setLlmStatus("Error");
                    setHeaderStatus(`LLM Load Failed (check logs): ${err}`, 5000);
                }
            } finally {
                llmOperationRef.current = false;
            }
        };
        void run();
    }, [enableGrammarLM, llmStatus, llmBackend, computeDevice, settingsLoaded, setEnableGrammarLM, setHeaderStatus, onOpenSettings]);

    // Storage moves release mapped weights. Reload only once the backend has
    // finished the move (including rollback after a failed copy).
    useEffect(() => {
        let disposed = false;
        let unlisten: (() => void) | undefined;
        listen("grammar-llm-unloaded", () => {
            if (disposed) return;
            llmRevisionRef.current += 1;
            appliedLlmBackendRef.current = null;
            setLlmStatus("Not Loaded");
        }).then((fn) => { if (disposed) fn(); else unlisten = fn; });
        return () => { disposed = true; unlisten?.(); };
    }, []);

    // ── Re-check LLM availability when models change (e.g. after download) ──
    useEffect(() => {
        if (llmStatus !== "Not Downloaded") return;
        let unlisten: (() => void) | undefined;
        let active = true;
        listen("models-changed", () => {
            if (!active) return;
            invoke("check_grammar_llm_available").then((available) => {
                if (active && available) setLlmStatus("Not Loaded");
            });
        }).then((fn) => { if (active) unlisten = fn; else fn(); }).catch(console.error);
        return () => { active = false; unlisten?.(); };
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
        computeDevice,
        setComputeDevice,
    };
}
