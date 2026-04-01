import { useState, useRef, startTransition } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Store } from "@tauri-apps/plugin-store";
import type { ModelInfo, ParakeetModelInfo, CohereModelInfo } from "./useModels";
import type { DownloadProgress } from "../components/settings/types";
import type { CommandResult, SessionNotice } from "../types/session";
import { COHERE_FP16_MODEL_ID } from "../utils/engineUtils";

export type ASREngine = "whisper" | "parakeet" | "cohere";

interface UseEngineSwitchParams {
    models: ModelInfo[];
    parakeetModels: ParakeetModelInfo[];
    cohereModels: CohereModelInfo[];
    currentModel: string | null;
    currentParakeetModel: string | null;
    currentCohereModel: string | null;
    setCurrentModel: (id: string) => void;
    setCurrentParakeetModel: (id: string) => void;
    setCurrentCohereModel: (id: string) => void;
    setBackendInfo: (info: string) => void;
    storeRef: React.RefObject<Store | null>;
    setHeaderStatus: (msg: string, dur?: number, isProcessing?: boolean) => void;
    setTrayState: (state: "ready" | "recording" | "processing") => Promise<void>;
    asrBackend: "gpu" | "cpu";
    setAsrBackend: (backend: "gpu" | "cpu") => void;
    /** True when FP16 Cohere is loaded — ASR CPU/GPU toggle must stay on GPU. */
    cohereGpuOnlyLocked: boolean;
    isRecordingRef: React.RefObject<boolean>;
    downloadProgressRef: React.RefObject<Record<string, DownloadProgress>>;
    setSessionPhase?: (phase: "idle" | "loading_model" | "recording" | "paused" | "processing" | "success" | "warning" | "error") => void;
    setSessionNotice?: (notice: SessionNotice | null) => void;
}

/**
 * Manages the active ASR engine (Whisper / Parakeet / Cohere Speech),
 * loading state, and engine-switch handlers.
 */
export function useEngineSwitch({
    models,
    parakeetModels,
    cohereModels,
    currentModel,
    currentParakeetModel,
    currentCohereModel,
    setCurrentModel,
    setCurrentParakeetModel,
    setCurrentCohereModel,
    setBackendInfo,
    storeRef,
    setHeaderStatus,
    setTrayState,
    asrBackend,
    setAsrBackend,
    cohereGpuOnlyLocked,
    isRecordingRef,
    downloadProgressRef,
    setSessionPhase,
    setSessionNotice,
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
        setSessionPhase?.("loading_model");
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
            setSessionPhase?.("idle");
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
            const result = await invoke<CommandResult<string>>("switch_model", { modelId, useGpu: asrBackend === "gpu" });
            if (!result.ok) throw new Error(result.error?.message ?? "Failed to load Whisper");

            if (activeEngine !== "whisper") {
                setActiveEngine("whisper");
                activeEngineRef.current = "whisper";
                setHeaderStatus(`Switched to Whisper (${modelId})`);
            } else {
                setHeaderStatus(`Switched model to ${modelId}`);
            }
            setSessionNotice?.(null);

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
            setSessionPhase?.("error");
            setSessionNotice?.({
                level: "error",
                code: "model_load_failed",
                title: "Whisper failed to load",
                message: String(e),
                sticky: true,
            });
        });
    };

    const handleSwitchToWhisper = async () => {
        if (isLoading || isLoadingRef.current) {
            console.log("[LOADING] Skipping handleSwitchToWhisper — already loading");
            return;
        }
        // After unload, active tab is still Whisper — must reload, not return (Parakeet/Cohere already check `loaded`).
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
            const result = await invoke<CommandResult<string>>("init_parakeet", { modelId: targetModel, useGpu: asrBackend === "gpu" });
            if (!result.ok) throw new Error(result.error?.message ?? "Failed to load Parakeet");

            setCurrentParakeetModel(targetModel);
            setActiveEngine("parakeet");
            activeEngineRef.current = "parakeet";
            setLoadedEngine("parakeet");
            setSessionNotice?.(null);

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
            setSessionPhase?.("error");
            setSessionNotice?.({
                level: "error",
                code: "model_load_failed",
                title: "Parakeet failed to load",
                message: String(e),
                sticky: true,
            });
        });
    };

    // ── Cohere Speech ────────────────────────────────────────────────────
    const handleSwitchToCohere = async (targetModelOverride?: string) => {
        const progress = downloadProgressRef.current ?? {};
        const graniteDownloading = cohereModels.some(m => progress[m.id]) ||
            Object.keys(progress).some(k => k.startsWith("granite"));
        if (graniteDownloading) {
            setHeaderStatus("Cohere Speech is still downloading — please wait", 3000);
            return;
        }
        if (cohereModels.length === 0) {
            setActiveEngine("cohere");
            activeEngineRef.current = "cohere";
            return;
        }
        if (isLoading || isLoadingRef.current) {
            console.log("[LOADING] Skipping handleSwitchToCohere — already loading");
            return;
        }

        if (activeEngine === "cohere") {
            try {
                const gStatus = await invoke("get_cohere_status") as { loaded: boolean };
                if (gStatus.loaded) return;
            } catch {
                // proceed with loading attempt
            }
        }

        const targetModel = targetModelOverride || currentCohereModel || cohereModels[0].id;

        await withEngineLoad("cohere", "Loading Cohere Speech...", async () => {
            const fp16 = targetModel === COHERE_FP16_MODEL_ID;
            const result = await invoke<CommandResult<string>>("init_cohere", {
                modelId: targetModel,
                forceCpu: asrBackend === "cpu" && !fp16,
            });
            if (!result.ok) throw new Error(result.error?.message ?? "Failed to load Cohere Speech");

            setCurrentCohereModel(targetModel);
            setActiveEngine("cohere");
            activeEngineRef.current = "cohere";
            setLoadedEngine("cohere");
            setSessionNotice?.(null);

            if (fp16) {
                setAsrBackend("gpu");
            }

            if (storeRef.current) {
                await storeRef.current.set("granite_model", targetModel);
                await storeRef.current.set("active_engine", "cohere");
                if (fp16) {
                    await storeRef.current.set("asr_backend", "gpu");
                }
                await storeRef.current.save();
            }

            setHeaderStatus("Switched to Cohere Speech");
            const backend = await invoke("get_backend_info");
            setBackendInfo(backend as string);
        }).catch(e => {
            setHeaderStatus(`Error switching to Cohere Speech: ${e}`, 5000);
            setSessionPhase?.("error");
            setSessionNotice?.({
                level: "error",
                code: "model_load_failed",
                title: "Cohere Speech failed to load",
                message: String(e),
                sticky: true,
            });
        });
    };

    // ── CPU / GPU hot-swap ────────────────────────────────────────────────
    const handleToggleAsrBackend = async (newBackend: "gpu" | "cpu") => {
        if (newBackend === asrBackend) return;
        if (cohereGpuOnlyLocked) return;
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
            (engine === "cohere" && cohereModels.length > 0);

        if (!hasModel) {
            setHeaderStatus(`ASR backend set to ${label}`);
            return;
        }

        // Heavy-path: reload active model on the new backend via withEngineLoad
        await withEngineLoad(engine, `Reloading on ${label}...`, async () => {
            if (engine === "whisper") {
                const displayName = models.find(m => m.id === currentModel)?.display_name || currentModel;
                setLoadingMessage(`Reloading ${displayName} on ${label}...`);
                const result = await invoke<CommandResult<string>>("switch_model", { modelId: currentModel, useGpu });
                if (!result.ok) throw new Error(result.error?.message ?? `Failed to switch Whisper to ${label}`);
                setLoadedEngine("whisper");
                const info = await invoke("get_backend_info");
                setBackendInfo(info as string);
                setHeaderStatus(`Whisper running on ${label}`);
                setSessionNotice?.(null);
            } else if (engine === "parakeet") {
                const targetModel = currentParakeetModel || parakeetModels[0]?.id;
                const result = await invoke<CommandResult<string>>("init_parakeet", { modelId: targetModel, useGpu });
                if (!result.ok) throw new Error(result.error?.message ?? `Failed to switch Parakeet to ${label}`);
                setLoadedEngine("parakeet");
                const info = await invoke("get_backend_info");
                setBackendInfo(info as string);
                setHeaderStatus(`Parakeet running on ${label}`);
                setSessionNotice?.(null);
            } else if (engine === "cohere") {
                const gid = currentCohereModel || cohereModels[0]?.id;
                const result = await invoke<CommandResult<string>>("init_cohere", {
                    modelId: gid,
                    forceCpu: !useGpu,
                });
                if (!result.ok) throw new Error(result.error?.message ?? `Failed to switch Cohere Speech to ${label}`);
                setLoadedEngine("cohere");
                const info = await invoke("get_backend_info");
                setBackendInfo(info as string);
                setHeaderStatus(`Cohere Speech running on ${label}`);
                setSessionNotice?.(null);
            }
        }).catch(e => {
            setHeaderStatus(`Failed to switch to ${label}: ${e}`, 5000);
            setSessionPhase?.("error");
            setSessionNotice?.({
                level: "error",
                code: "model_load_failed",
                title: `Failed to switch to ${label}`,
                message: String(e),
                sticky: true,
            });
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
        handleSwitchToCohere,
        handleToggleAsrBackend,
    };
}
