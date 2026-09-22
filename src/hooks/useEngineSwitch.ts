import type { TrayState } from "../App";
import { useState, useRef, startTransition } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Store } from "@tauri-apps/plugin-store";
import type { ModelInfo, GraniteModelInfo, Qwen3ModelInfo } from "./useModels";
import type { DownloadProgress } from "../components/settings/types";
import type { CommandResult, SessionNotice } from "../types/session";

export type ASREngine = "whisper" | "granite" | "qwen3";

interface UseEngineSwitchParams {
    models: ModelInfo[];
    graniteModels: GraniteModelInfo[];
    qwen3Models: Qwen3ModelInfo[];
    currentModel: string | null;
    currentGraniteModel: string | null;
    currentQwen3Model: string | null;
    setCurrentModel: (id: string) => void;
    setCurrentGraniteModel: (id: string) => void;
    setCurrentQwen3Model: (id: string) => void;
    setBackendInfo: (info: string) => void;
    storeRef: React.RefObject<Store | null>;
    setHeaderStatus: (msg: string, dur?: number, isProcessing?: boolean) => void;
    setTrayState: (state: TrayState) => Promise<void>;
    asrBackend: "gpu" | "cpu";
    setAsrBackend: (backend: "gpu" | "cpu") => void;
    isRecordingRef: React.RefObject<boolean>;
    downloadProgressRef: React.RefObject<Record<string, DownloadProgress>>;
    setSessionPhase?: (phase: "idle" | "loading_model" | "recording" | "paused" | "processing" | "success" | "warning" | "error") => void;
    setSessionNotice?: (notice: SessionNotice | null) => void;
}

/**
 * Manages the active ASR engine (Whisper / Granite / Qwen3-ASR),
 * loading state, and engine-switch handlers.
 */
export function useEngineSwitch({
    models,
    graniteModels,
    qwen3Models,
    currentModel,
    currentGraniteModel,
    currentQwen3Model,
    setCurrentModel,
    setCurrentGraniteModel,
    setCurrentQwen3Model,
    setBackendInfo,
    storeRef,
    setHeaderStatus,
    setTrayState,
    asrBackend,
    setAsrBackend,
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
            await setTrayState("loading_model");
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
        // After unload, active tab is still Whisper — must reload, not return (Granite/Qwen3 already check `loaded`).
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

    // ── Granite ──────────────────────────────────────────────────────────
    const handleSwitchToGranite = async (targetModelOverride?: string) => {
        const progress = downloadProgressRef.current ?? {};
        const graniteDownloading = graniteModels.some(m => progress[m.id]) ||
            Object.keys(progress).some(k => k.startsWith("granite"));
        if (graniteDownloading) {
            setHeaderStatus("Granite is still downloading — please wait", 3000);
            return;
        }
        if (graniteModels.length === 0) {
            setActiveEngine("granite");
            activeEngineRef.current = "granite";
            return;
        }
        if (isLoading || isLoadingRef.current) {
            console.log("[LOADING] Skipping handleSwitchToGranite — already loading");
            return;
        }

        const targetModel = targetModelOverride || currentGraniteModel || graniteModels[0].id;
        const displayName = graniteModels.find(m => m.id === targetModel)?.display_name || targetModel;

        if (activeEngine === "granite") {
            try {
                const pStatus = await invoke("get_granite_status") as { loaded: boolean; model_id?: string | null };
                if (pStatus.loaded && pStatus.model_id === targetModel) return;
            } catch {
                // proceed with loading attempt
            }
        }

        await withEngineLoad("granite", `Loading ${displayName}...`, async () => {
            const result = await invoke<CommandResult<string>>("init_granite", { modelId: targetModel, useGpu: asrBackend === "gpu" });
            if (!result.ok) throw new Error(result.error?.message ?? "Failed to load Granite");

            setCurrentGraniteModel(targetModel);
            setActiveEngine("granite");
            activeEngineRef.current = "granite";
            setLoadedEngine("granite");
            setSessionNotice?.(null);

            if (storeRef.current) {
                await storeRef.current.set("granite_model", targetModel);
                await storeRef.current.set("active_engine", "granite");
                await storeRef.current.save();
            }

            setHeaderStatus(`Switched to ${displayName}`);
            const backend = await invoke("get_backend_info");
            setBackendInfo(backend as string);
        }).catch(e => {
            setHeaderStatus(`Error switching to Granite: ${e}`, 5000);
            setSessionPhase?.("error");
            setSessionNotice?.({
                level: "error",
                code: "model_load_failed",
                title: "Granite failed to load",
                message: String(e),
                sticky: true,
            });
        });
    };

    const handleSwitchToQwen3 = async (targetModelOverride?: string) => {
        const progress = downloadProgressRef.current ?? {};
        if (Object.keys(progress).some(key => key.startsWith("qwen3"))) {
            setHeaderStatus("Qwen3-ASR is still downloading — please wait", 3000);
            return;
        }
        if (qwen3Models.length === 0) {
            setActiveEngine("qwen3");
            activeEngineRef.current = "qwen3";
            return;
        }
        if (isLoading || isLoadingRef.current) return;
        const targetModel = targetModelOverride || currentQwen3Model || qwen3Models[0].id;
        if (activeEngine === "qwen3") {
            const status = await invoke<{ loaded: boolean; model_id?: string }>("get_qwen3_status").catch(() => null);
            if (status?.loaded && status.model_id === targetModel) return;
        }
        await withEngineLoad("qwen3", "Loading Qwen3-ASR...", async () => {
            const result = await invoke<CommandResult<string>>("init_qwen3", {
                modelId: targetModel,
                useGpu: asrBackend === "gpu",
            });
            if (!result.ok) throw new Error(result.error?.message ?? "Failed to load Qwen3-ASR");
            setCurrentQwen3Model(targetModel);
            setActiveEngine("qwen3");
            activeEngineRef.current = "qwen3";
            setLoadedEngine("qwen3");
            setSessionNotice?.(null);
            if (storeRef.current) {
                await storeRef.current.set("qwen3_model", targetModel);
                await storeRef.current.set("active_engine", "qwen3");
                await storeRef.current.save();
            }
            setBackendInfo(await invoke<string>("get_backend_info"));
            setHeaderStatus("Switched to Qwen3-ASR");
        }).catch(error => {
            setHeaderStatus(`Error switching to Qwen3-ASR: ${error}`, 5000);
            setSessionPhase?.("error");
        });
    };

    // ── CPU / GPU hot-swap ────────────────────────────────────────────────
    const handleToggleAsrBackend = async (newBackend: "gpu" | "cpu") => {
        if (newBackend === asrBackend) return;
        if (isLoading || isLoadingRef.current) return;
        if (isRecordingRef.current) return;

        setAsrBackend(newBackend);

        const useGpu = newBackend === "gpu";
        const label = useGpu ? "GPU" : "CPU";
        const engine = activeEngineRef.current;

        // Fast-path: no model loaded — just update preference
        const hasModel =
            (engine === "whisper" && !!currentModel) ||
            (engine === "granite" && !!(currentGraniteModel || graniteModels.length > 0)) ||
            (engine === "qwen3" && qwen3Models.length > 0);

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
            } else if (engine === "granite") {
                const targetModel = currentGraniteModel || graniteModels[0]?.id;
                const result = await invoke<CommandResult<string>>("init_granite", { modelId: targetModel, useGpu });
                if (!result.ok) throw new Error(result.error?.message ?? `Failed to switch Granite to ${label}`);
                setLoadedEngine("granite");
                const info = await invoke("get_backend_info");
                setBackendInfo(info as string);
                setHeaderStatus(`Granite running on ${label}`);
                setSessionNotice?.(null);
            } else if (engine === "qwen3") {
                const qid = currentQwen3Model || qwen3Models[0]?.id;
                const result = await invoke<CommandResult<string>>("init_qwen3", { modelId: qid, useGpu });
                if (!result.ok) throw new Error(result.error?.message ?? `Failed to switch Qwen3-ASR to ${label}`);
                setLoadedEngine("qwen3");
                setBackendInfo(await invoke<string>("get_backend_info"));
                setHeaderStatus(`Qwen3-ASR running on ${label}`);
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
        handleSwitchToGranite,
        handleSwitchToQwen3,
        handleToggleAsrBackend,
    };
}
