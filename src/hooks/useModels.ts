import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface ModelInfo {
    id: string;
    display_name: string;
    file_name: string;
    size_mb: number;
    has_coreml: boolean;
}

export interface ParakeetModelInfo {
    id: string;
    display_name: string;
    model_type: string;
    size_mb: number;
}

export interface CohereModelInfo {
    id: string;
    display_name: string;
    size_mb: number;
    /** True for the FP16 package — requires GPU; download INT4 for CPU-only machines. */
    requires_gpu?: boolean;
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
    const [cohereModels, setCohereModels] = useState<CohereModelInfo[]>([]);
    const [currentCohereModel, setCurrentCohereModel] = useState<string | null>(null);

    const refreshModels = async (showToast = true) => {
        try {
            console.log("[INFO] Refreshing model lists...");
            const modelList = await invoke("list_models") as ModelInfo[];
            setModels(modelList);

            const pModels = await invoke("list_parakeet_models") as ParakeetModelInfo[];
            setParakeetModels(pModels);

            const gModels = await invoke("list_cohere_models") as CohereModelInfo[];
            setCohereModels(gModels);

            setCurrentModel(prev => {
                if (modelList.length === 0) return null;
                if (prev && modelList.some(model => model.id === prev)) return prev;
                return modelList[0].id;
            });
            setCurrentParakeetModel(prev => {
                if (pModels.length === 0) return null;
                if (prev && pModels.some(model => model.id === prev)) return prev;
                return pModels[0].id;
            });
            setCurrentCohereModel(prev => {
                if (gModels.length === 0) return null;
                if (prev && gModels.some(model => model.id === prev)) return prev;
                return gModels[0].id;
            });

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
        cohereModels,
        setCohereModels,
        currentCohereModel,
        setCurrentCohereModel,
        refreshModels,
    };
}
