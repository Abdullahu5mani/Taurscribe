import { useState, useRef, startTransition } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Store } from "@tauri-apps/plugin-store";
import type { ModelInfo, ParakeetModelInfo, GraniteSpeechModelInfo } from "./useModels";
import type { DownloadProgress } from "../components/settings/types";
import { GRANITE_FP16_MODEL_ID } from "../utils/engineUtils";

export type ASREngine = "whisper" | "parakeet" | "granite_speech";

interface UseEngineSwitchParams {
    models: ModelInfo[];
    parakeetModels: ParakeetModelInfo[];
    graniteModels: GraniteSpeechModelInfo[];
    currentModel: string | null;
    currentParakeetModel: string | null;
    currentGraniteModel: string | null;
    setCurrentModel: (id: string) => void;
    setCurrentParakeetModel: (id: string) => void;
    setCurrentGraniteModel: (id: string) => void;
    setBackendInfo: (info: string) => void;
    storeRef: React.RefObject<Store | null>;
    setHeaderStatus: (msg: string, dur?: number, isProcessing?: boolean) => void;
    setTrayState: (state: "ready" | "recording" | "processing") => Promise<void>;
    asrBackend: "gpu" | "cpu";
    setAsrBackend: (backend: "gpu" | "cpu") => void;
    /** True when FP16 Granite is loaded — ASR CPU/GPU toggle must stay on GPU. */
    graniteGpuOnlyLocked: boolean;
    isRecordingRef: React.RefObject<boolean>;
    downloadProgressRef: React.RefObject<Record<string, DownloadProgress>>;
}

/**
 * Manages the active ASR engine (Whisper / Parakeet / Granite Speech),
 * loading state, and engine-switch handlers.
 */
export function useEngineSwitch({
    models,
    parakeetModels,
    graniteModels,
    currentModel,
    currentParakeetModel,
    currentGraniteModel,
    setCurrentModel,
    setCurrentParakeetModel,
    setCurrentGraniteModel,
    setBackendInfo,
    storeRef,
    setHeaderStatus,
    setTrayState,
    asrBackend,
    setAsrBackend,
    graniteGpuOnlyLocked,
    isRecordingRef,
    downloadProgressRef,
}: UseEngineSwitchParams) {
    const [activeEngine, setActiveEngine] = useState<ASREngine>("whisper");
    const [loadedEngine, setLoadedEngine] = useState<ASREngine | null>(null);
    const [isLoading, setIsLoading] = useState(false);
    const [loadingMessage, setLoadingMessage] = useState("");
    const [loadingTargetEngine, setLoadingTargetEngine] = useState<ASREngine | null>(null);
    const [transferLineFadingOut, setTransferLineFadingOut] = useState(false);

    // Ref prevents double-switch when state updates are async
    const isLoadingRef = useRef(false);
    const activeEngineRef = useRef(activeEngine);

    // ── Loading lifecycle wrapper ─────────────────────────────────────────
    // Handles the identical boilerplate that surrounded every engine-load:
    //   set loading flags → set tray → run fn → clear flags → set tray ready
    // Each handler only provides the unique message + async work (fn).
    const withEngineLoad = async (
        engine: ASREngine,
        message: string,
        fn: () => Promise<void>
    ): Promise<void> => {
        isLoadingRef.current = true;
        // Non-urgent UI: keep clicks/scroll responsive while the heavy invoke runs.
        startTransition(() => {
            setIsLoading(true);
            setLoadingTargetEngine(engine);
            setLoadingMessage(message);
        });
        // Do not clear loadedEngine here — avoids flashing every engine to "unloaded"
        // during a switch; handlers update loadedEngine on success.
        setHeaderStatus(message, 60_000);

        try {
            await setTrayState("processing");
            await fn();
        } finally {
            isLoadingRef.current = false;
            startTransition(() => {
                setIsLoading(false);
                setLoadingMessage("");
                setLoadingTargetEngine(null);
            });
            setTransferLineFadingOut(true);
            await setTrayState("ready");
        }
    };

    // ── Whisper ───────────────────────────────────────────────────────────
    const handleModelChange = async (modelId: string) => {
        // Same UI selection can be "unloaded" in VRAM — only skip if Whisper already holds this id.
        if (modelId === currentModel && activeEngine === "whisper") {
            try {
                const loadedId = (await invoke("get_current_model")) as string | null;
                if (loadedId === modelId) return;
            } catch {
                /* proceed to load */
            }
        }
        if (isLoading || isLoadingRef.current) {
            console.log("[LOADING] Skipping handleModelChange — already loading");
            return;
        }

        const displayName = models.find(m => m.id === modelId)?.display_name || modelId;

        await withEngineLoad("whisper", `Loading ${displayName}...`, async () => {
            await invoke("switch_model", { modelId, useGpu: asrBackend === "gpu" });

            if (activeEngine !== "whisper") {
                setActiveEngine("whisper");
                activeEngineRef.current = "whisper";
                setHeaderStatus(`Switched to Whisper (${modelId})`);
            } else {
                setHeaderStatus(`Switched model to ${modelId}`);
            }

            setCurrentModel(modelId);
            setLoadedEngine("whisper");

            if (storeRef.current) {
                await storeRef.current.set("whisper_model", modelId);
                await storeRef.current.set("active_engine", "whisper");
                await storeRef.current.save();
            }

            const backend = await invoke("get_backend_info");
            setBackendInfo(backend as string);
        }).catch(e => {
            setHeaderStatus(`Error switching model: ${e}`, 5000);
        });
    };

    const handleSwitchToWhisper = async () => {
        if (isLoading || isLoadingRef.current) {
            console.log("[LOADING] Skipping handleSwitchToWhisper — already loading");
            return;
        }
        // After unload, active tab is still Whisper — must reload, not return (Parakeet/Granite already check `loaded`).
        if (activeEngine === "whisper") {
            try {
                const loadedId = (await invoke("get_current_model")) as string | null;
                if (loadedId != null && loadedId !== "") return;
            } catch {
                /* proceed with loading attempt */
            }
        }

        if (!currentModel && models.length > 0) {
            await handleModelChange(models[0].id);
        } else if (currentModel) {
            await handleModelChange(currentModel);
        } else {
            setActiveEngine("whisper");
            activeEngineRef.current = "whisper";
        }
    };

    // ── Parakeet ──────────────────────────────────────────────────────────
    const handleSwitchToParakeet = async (targetModelOverride?: string) => {
        const progress = downloadProgressRef.current ?? {};
        const parakeetDownloading = parakeetModels.some(m => progress[m.id]) ||
            Object.keys(progress).some(k => k.startsWith("parakeet"));
        if (parakeetDownloading) {
            setHeaderStatus("Parakeet is still downloading — please wait", 3000);
            return;
        }
        if (parakeetModels.length === 0) {
            setActiveEngine("parakeet");
            activeEngineRef.current = "parakeet";
            return;
        }
        if (isLoading || isLoadingRef.current) {
            console.log("[LOADING] Skipping handleSwitchToParakeet — already loading");
            return;
        }

        if (activeEngine === "parakeet") {
            try {
                const pStatus = await invoke("get_parakeet_status") as { loaded: boolean };
                if (pStatus.loaded) return;
            } catch {
                // proceed with loading attempt
            }
        }

        const targetModel = targetModelOverride || currentParakeetModel || parakeetModels[0].id;

        await withEngineLoad("parakeet", `Loading Parakeet (${targetModel})...`, async () => {
            await invoke("init_parakeet", { modelId: targetModel, useGpu: asrBackend === "gpu" });

            setCurrentParakeetModel(targetModel);
            setActiveEngine("parakeet");
            activeEngineRef.current = "parakeet";
            setLoadedEngine("parakeet");

            if (storeRef.current) {
                await storeRef.current.set("parakeet_model", targetModel);
                await storeRef.current.set("active_engine", "parakeet");
                await storeRef.current.save();
            }

            setHeaderStatus("Switched to Parakeet");
            const backend = await invoke("get_backend_info");
            setBackendInfo(backend as string);
        }).catch(e => {
            setHeaderStatus(`Error switching to Parakeet: ${e}`, 5000);
        });
    };

    // ── Granite Speech ────────────────────────────────────────────────────
    const handleSwitchToGranite = async (targetModelOverride?: string) => {
        const progress = downloadProgressRef.current ?? {};
        const graniteDownloading = graniteModels.some(m => progress[m.id]) ||
            Object.keys(progress).some(k => k.startsWith("granite"));
        if (graniteDownloading) {
            setHeaderStatus("Granite Speech is still downloading — please wait", 3000);
            return;
        }
        if (graniteModels.length === 0) {
            setActiveEngine("granite_speech");
            activeEngineRef.current = "granite_speech";
            return;
        }
        if (isLoading || isLoadingRef.current) {
            console.log("[LOADING] Skipping handleSwitchToGranite — already loading");
            return;
        }

        if (activeEngine === "granite_speech") {
            try {
                const gStatus = await invoke("get_granite_speech_status") as { loaded: boolean };
                if (gStatus.loaded) return;
            } catch {
                // proceed with loading attempt
            }
        }

        const targetModel = targetModelOverride || currentGraniteModel || graniteModels[0].id;

        await withEngineLoad("granite_speech", "Loading Granite Speech...", async () => {
            const fp16 = targetModel === GRANITE_FP16_MODEL_ID;
            await invoke("init_granite_speech", {
                modelId: targetModel,
                forceCpu: asrBackend === "cpu" && !fp16,
            });

            setCurrentGraniteModel(targetModel);
            setActiveEngine("granite_speech");
            activeEngineRef.current = "granite_speech";
            setLoadedEngine("granite_speech");

            if (fp16) {
                setAsrBackend("gpu");
            }

            if (storeRef.current) {
                await storeRef.current.set("granite_model", targetModel);
                await storeRef.current.set("active_engine", "granite_speech");
                if (fp16) {
                    await storeRef.current.set("asr_backend", "gpu");
                }
                await storeRef.current.save();
            }

            setHeaderStatus("Switched to Granite Speech");
            const backend = await invoke("get_backend_info");
            setBackendInfo(backend as string);
        }).catch(e => {
            setHeaderStatus(`Error switching to Granite Speech: ${e}`, 5000);
        });
    };

    // ── CPU / GPU hot-swap ────────────────────────────────────────────────
    const handleToggleAsrBackend = async (newBackend: "gpu" | "cpu") => {
        if (newBackend === asrBackend) return;
        if (graniteGpuOnlyLocked) return;
        if (isLoading || isLoadingRef.current) return;
        if (isRecordingRef.current) return;

        setAsrBackend(newBackend);

        const useGpu = newBackend === "gpu";
        const label = useGpu ? "GPU" : "CPU";
        const engine = activeEngineRef.current;

        // Fast-path: no model loaded — just update preference
        const hasModel =
            (engine === "whisper" && !!currentModel) ||
            (engine === "parakeet" && !!(currentParakeetModel || parakeetModels.length > 0)) ||
            (engine === "granite_speech" && graniteModels.length > 0);

        if (!hasModel) {
            setHeaderStatus(`ASR backend set to ${label}`);
            return;
        }

        // Heavy-path: reload active model on the new backend via withEngineLoad
        await withEngineLoad(engine, `Reloading on ${label}...`, async () => {
            if (engine === "whisper") {
                const displayName = models.find(m => m.id === currentModel)?.display_name || currentModel;
                setLoadingMessage(`Reloading ${displayName} on ${label}...`);
                await invoke("switch_model", { modelId: currentModel, useGpu });
                setLoadedEngine("whisper");
                const info = await invoke("get_backend_info");
                setBackendInfo(info as string);
                setHeaderStatus(`Whisper running on ${label}`);
            } else if (engine === "parakeet") {
                const targetModel = currentParakeetModel || parakeetModels[0]?.id;
                await invoke("init_parakeet", { modelId: targetModel, useGpu });
                setLoadedEngine("parakeet");
                const info = await invoke("get_backend_info");
                setBackendInfo(info as string);
                setHeaderStatus(`Parakeet running on ${label}`);
            } else if (engine === "granite_speech") {
                const gid = currentGraniteModel || graniteModels[0]?.id;
                await invoke("init_granite_speech", {
                    modelId: gid,
                    forceCpu: !useGpu,
                });
                setLoadedEngine("granite_speech");
                const info = await invoke("get_backend_info");
                setBackendInfo(info as string);
                setHeaderStatus(`Granite Speech running on ${label}`);
            }
        }).catch(e => {
            setHeaderStatus(`Failed to switch to ${label}: ${e}`, 5000);
        });
    };

    return {
        activeEngine,
        setActiveEngine,
        activeEngineRef,
        loadedEngine,
        setLoadedEngine,
        isLoading,
        setIsLoading,
        isLoadingRef,
        loadingMessage,
        loadingTargetEngine,
        transferLineFadingOut,
        setTransferLineFadingOut,
        handleModelChange,
        handleSwitchToWhisper,
        handleSwitchToParakeet,
        handleSwitchToGranite,
        handleToggleAsrBackend,
    };
}
