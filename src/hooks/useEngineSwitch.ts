import { useState, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Store } from "@tauri-apps/plugin-store";
import type { ModelInfo, ParakeetModelInfo } from "./useModels";

export type ASREngine = "whisper" | "parakeet";

interface UseEngineSwitchParams {
    models: ModelInfo[];
    parakeetModels: ParakeetModelInfo[];
    currentModel: string | null;
    currentParakeetModel: string | null;
    setCurrentModel: (id: string) => void;
    setCurrentParakeetModel: (id: string) => void;
    setBackendInfo: (info: string) => void;
    storeRef: React.RefObject<Store | null>;
    setHeaderStatus: (msg: string, dur?: number, isProcessing?: boolean) => void;
    setTrayState: (state: "ready" | "recording" | "processing") => Promise<void>;
}

/**
 * Manages the active ASR engine (Whisper / Parakeet), loading state,
 * and engine-switch handlers.
 */
export function useEngineSwitch({
    models,
    parakeetModels,
    currentModel,
    currentParakeetModel,
    setCurrentModel,
    setCurrentParakeetModel,
    setBackendInfo,
    storeRef,
    setHeaderStatus,
    setTrayState,
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
            await invoke("switch_model", { modelId });
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

    const handleSwitchToParakeet = async () => {
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

        const targetModel = currentParakeetModel || parakeetModels[0].id;

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
            await invoke("init_parakeet", { modelId: targetModel });

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
    };
}
