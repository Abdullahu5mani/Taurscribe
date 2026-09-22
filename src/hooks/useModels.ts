import { useCallback, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface ModelInfo {
    id: string;
    display_name: string;
    file_name: string;
    size_mb: number;
    has_coreml: boolean;
}

export interface GraniteModelInfo {
    id: string;
    display_name: string;
    model_type: string;
    size_mb: number;
}

export interface Qwen3ModelInfo {
    id: string;
    display_name: string;
    size_mb: number;
    requires_gpu?: boolean;
}

export interface GraniteStatus {
    loaded: boolean;
    model_id: string | null;
    model_type: string | null;
    backend: string;
}

/**
 * Manages the Whisper, Granite and Qwen3 model lists and provides a refresh function.
 */
export function useModels(setHeaderStatus: (msg: string, dur?: number) => void) {
    const [models, setModels] = useState<ModelInfo[]>([]);
    const [currentModel, setCurrentModel] = useState<string | null>(null);
    const [graniteModels, setGraniteModels] = useState<GraniteModelInfo[]>([]);
    const [currentGraniteModel, setCurrentGraniteModel] = useState<string | null>(null);
    const [qwen3Models, setQwen3Models] = useState<Qwen3ModelInfo[]>([]);
    const [currentQwen3Model, setCurrentQwen3Model] = useState<string | null>(null);

    const refreshModels = useCallback(async (showToast = true) => {
        try {
            console.log("[INFO] Refreshing model lists...");
            const [modelList, pModels, qModels] = await Promise.all([
                invoke<ModelInfo[]>("list_models"),
                invoke<GraniteModelInfo[]>("list_granite_models"),
                invoke<Qwen3ModelInfo[]>("list_qwen3_models"),
            ]);

            setModels(modelList);
            setGraniteModels(pModels);
            setQwen3Models(qModels);

            setCurrentModel(prev => {
                if (modelList.length === 0) return null;
                if (prev && modelList.some(model => model.id === prev)) return prev;
                return modelList[0].id;
            });
            setCurrentGraniteModel(prev => {
                if (pModels.length === 0) return null;
                if (prev && pModels.some(model => model.id === prev)) return prev;
                return pModels[0].id;
            });
            setCurrentQwen3Model(prev => {
                if (qModels.length === 0) return null;
                if (prev && qModels.some(model => model.id === prev)) return prev;
                return qModels[0].id;
            });

            if (showToast) {
                setHeaderStatus("Model list refreshed!");
            }
        } catch (e) {
            console.error("Failed to refresh models:", e);
        }
    }, [setHeaderStatus]);

    return {
        models,
        setModels,
        currentModel,
        setCurrentModel,
        graniteModels,
        setGraniteModels,
        currentGraniteModel,
        setCurrentGraniteModel,
        qwen3Models,
        setQwen3Models,
        currentQwen3Model,
        setCurrentQwen3Model,
        refreshModels,
    };
}
