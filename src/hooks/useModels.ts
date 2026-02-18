import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface ModelInfo {
    id: string;
    display_name: string;
    file_name: string;
    size_mb: number;
}

export interface ParakeetModelInfo {
    id: string;
    display_name: string;
    model_type: string;
    size_mb: number;
}

export interface ParakeetStatus {
    loaded: boolean;
    model_id: string | null;
    model_type: string | null;
    backend: string;
}

/**
 * Manages Whisper and Parakeet model lists and provides a refresh function.
 */
export function useModels(setHeaderStatus: (msg: string, dur?: number) => void) {
    const [models, setModels] = useState<ModelInfo[]>([]);
    const [currentModel, setCurrentModel] = useState<string | null>(null);
    const [parakeetModels, setParakeetModels] = useState<ParakeetModelInfo[]>([]);
    const [currentParakeetModel, setCurrentParakeetModel] = useState<string | null>(null);

    const refreshModels = async (showToast = true) => {
        try {
            console.log("[INFO] Refreshing model lists...");
            const modelList = await invoke("list_models") as ModelInfo[];
            setModels(modelList);

            const pModels = await invoke("list_parakeet_models") as ParakeetModelInfo[];
            setParakeetModels(pModels);

            if (modelList.length > 0) {
                setCurrentModel(prev => prev ?? modelList[0].id);
            }
            if (pModels.length > 0) {
                setCurrentParakeetModel(prev => prev ?? pModels[0].id);
            }

            if (showToast) {
                setHeaderStatus("Model list refreshed!");
            }
        } catch (e) {
            console.error("Failed to refresh models:", e);
        }
    };

    return {
        models,
        setModels,
        currentModel,
        setCurrentModel,
        parakeetModels,
        setParakeetModels,
        currentParakeetModel,
        setCurrentParakeetModel,
        refreshModels,
    };
}
