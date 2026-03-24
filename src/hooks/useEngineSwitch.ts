import { useState, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Store } from "@tauri-apps/plugin-store";
import type { ModelInfo, ParakeetModelInfo, GraniteSpeechModelInfo } from "./useModels";
import type { DownloadProgress } from "../components/settings/types";

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
    downloadProgressRef: React.RefObject<Record<string, DownloadProgress>>;
}

/**
 * Manages the active ASR engine (Whisper / Parakeet), loading state,
 * and engine-switch handlers.
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

    const handleModelChange = async (modelId: string) => {
        if (modelId === currentModel && activeEngine === "whisper") return;
        if (isLoading || isLoadingRef.current) {
            console.log("[LOADING] Skipping handleModelChange — already loading");
            return;
        }

        isLoadingRef.current = true;
        setIsLoading(true);
        setLoadedEngine(null);
        setLoadingTargetEngine("whisper");
        const displayName = models.find(m => m.id === modelId)?.display_name || modelId;
        const msg = `Loading ${displayName}...`;
        setLoadingMessage(msg);
        setHeaderStatus(msg, 60_000);
        console.log("[LOADING] Loading Whisper model " + modelId);

        try {
            await setTrayState("processing");
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
        } catch (e) {
            setHeaderStatus(`Error switching model: ${e}`, 5000);
        } finally {
            console.log("[LOADING] Set loading FALSE — handleModelChange");
            isLoadingRef.current = false;
            setIsLoading(false);
            setLoadingMessage("");
            setLoadingTargetEngine(null);
            setTransferLineFadingOut(true);
            await setTrayState("ready");
        }
    };

    const handleSwitchToWhisper = async () => {
        if (activeEngine === "whisper") return;
        if (!currentModel && models.length > 0) {
            await handleModelChange(models[0].id);
        } else if (currentModel) {
            await handleModelChange(currentModel);
        } else {
            setActiveEngine("whisper");
            activeEngineRef.current = "whisper";
        }
    };

    const handleSwitchToParakeet = async (targetModelOverride?: string) => {
        const progress = downloadProgressRef.current ?? {};
        const parakeetDownloading = parakeetModels.some(m => progress[m.id]) ||
            Object.keys(progress).some(k => k.startsWith('parakeet'));
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

        isLoadingRef.current = true;
        setIsLoading(true);
        setLoadedEngine(null);
        setLoadingTargetEngine("parakeet");
        const msg = `Loading Parakeet (${targetModel})...`;
        setLoadingMessage(msg);
        setHeaderStatus(msg, 60_000);
        console.log("[LOADING] Loading Parakeet (" + targetModel + ")");

        try {
            await setTrayState("processing");
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
        } catch (e) {
            setHeaderStatus(`Error switching to Parakeet: ${e}`, 5000);
        } finally {
            console.log("[LOADING] Set loading FALSE — handleSwitchToParakeet");
            isLoadingRef.current = false;
            setIsLoading(false);
            setLoadingMessage("");
            setLoadingTargetEngine(null);
            setTransferLineFadingOut(true);
            await setTrayState("ready");
        }
    };

    const handleSwitchToGranite = async (targetModelOverride?: string) => {
        const progress = downloadProgressRef.current ?? {};
        const graniteDownloading = graniteModels.some(m => progress[m.id]) ||
            Object.keys(progress).some(k => k.startsWith('granite'));
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

        isLoadingRef.current = true;
        setIsLoading(true);
        setLoadedEngine(null);
        setLoadingTargetEngine("granite_speech");
        const msg = `Loading Granite Speech...`;
        setLoadingMessage(msg);
        setHeaderStatus(msg, 60_000);
        console.log("[LOADING] Loading Granite Speech (" + targetModel + ")");

        try {
            await setTrayState("processing");
            await invoke("init_granite_speech", { forceCpu: asrBackend === "cpu" });

            setCurrentGraniteModel(targetModel);
            setActiveEngine("granite_speech");
            activeEngineRef.current = "granite_speech";
            setLoadedEngine("granite_speech");

            if (storeRef.current) {
                await storeRef.current.set("granite_model", targetModel);
                await storeRef.current.set("active_engine", "granite_speech");
                await storeRef.current.save();
            }

            setHeaderStatus("Switched to Granite Speech");

            const backend = await invoke("get_backend_info");
            setBackendInfo(backend as string);
        } catch (e) {
            setHeaderStatus(`Error switching to Granite Speech: ${e}`, 5000);
        } finally {
            console.log("[LOADING] Set loading FALSE — handleSwitchToGranite");
            isLoadingRef.current = false;
            setIsLoading(false);
            setLoadingMessage("");
            setLoadingTargetEngine(null);
            setTransferLineFadingOut(true);
            await setTrayState("ready");
        }
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
    };
}
