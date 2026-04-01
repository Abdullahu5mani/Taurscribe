import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Store } from "@tauri-apps/plugin-store";
import { MODELS } from "../components/settings/types";
import type { DownloadableModel } from "../components/settings/types";
import type { ModelInfo, ParakeetModelInfo, CohereModelInfo } from "./useModels";
import type { ASREngine } from "./useEngineSwitch";
import type { CommandResult, EngineSelectionState } from "../types/session";
import { COHERE_FP16_MODEL_ID } from "../utils/engineUtils";

interface UseInitialLoadParams {
    // Model state setters
    setModels: (models: ModelInfo[]) => void;
    setCurrentModel: (id: string | null) => void;
    setParakeetModels: (models: ParakeetModelInfo[]) => void;
    setCurrentParakeetModel: (id: string | null) => void;
    setCohereModels: (models: CohereModelInfo[]) => void;
    setCurrentCohereModel: (id: string | null) => void;
    setSettingsModels: React.Dispatch<React.SetStateAction<DownloadableModel[]>>;

    // Engine/loading state setters
    setLoadedEngine: (engine: ASREngine | null) => void;
    setActiveEngine: (engine: ASREngine) => void;
    activeEngineRef: React.MutableRefObject<ASREngine>;
    isLoadingRef: React.MutableRefObject<boolean>;
    setIsLoading: (v: boolean) => void;
    setLoadingMessage: (msg: string) => void;

    // App state setters
    setBackendInfo: (info: string) => void;
    setHeaderStatus: (msg: string, dur?: number) => void;
    setShowSetupWizard: (v: boolean) => void;
    setIsInitialLoading: (v: boolean) => void;
    setCloseBehavior: (v: "tray" | "quit") => void;

    /** Sync ASR toggle when FP16 Cohere forces GPU */
    setAsrBackend: (v: "gpu" | "cpu") => void;

    // Store ref — populated by this hook so callers can use it later
    storeRef: React.MutableRefObject<Store | null>;
}

/**
 * Runs the app's startup sequence exactly once on mount:
 *   1. Fetches backend info and all model lists
 *   2. Pre-fetches download status for all known models
 *   3. Loads and restores settings.json (engine, hotkey, device, close-behavior, overlay)
 *   4. Restores the active engine and lazy-loads heavyweight backends as needed
 */
export function useInitialLoad({
    setModels,
    setCurrentModel,
    setParakeetModels,
    setCurrentParakeetModel,
    setCohereModels,
    setCurrentCohereModel,
    setSettingsModels,
    setLoadedEngine,
    setActiveEngine,
    activeEngineRef,
    isLoadingRef,
    setIsLoading,
    setLoadingMessage,
    setBackendInfo,
    setHeaderStatus,
    setShowSetupWizard,
    setIsInitialLoading,
    setCloseBehavior,
    setAsrBackend,
    storeRef,
}: UseInitialLoadParams) {
    useEffect(() => {
        let cancelled = false;

        async function loadInitialData() {
            try {
                const backend = await invoke("get_backend_info");
                if (cancelled) return;
                setBackendInfo(backend as string);

                // Pre-fetch download status of all known models
                try {
                    const statuses = await invoke<any[]>("get_download_status", {
                        modelIds: MODELS.map((m) => m.id),
                    });
                    if (!cancelled) {
                        setSettingsModels((prev) =>
                            prev.map((m) => {
                                const s = statuses.find((x) => x.id === m.id);
                                return s ? { ...m, downloaded: s.downloaded, verified: s.verified } : m;
                            })
                        );
                    }
                } catch (e) {
                    console.error("Failed to fetch initial model statuses:", e);
                }

                const modelList = (await invoke("list_models")) as ModelInfo[];
                if (cancelled) return;
                setModels(modelList);

                const pModels = (await invoke("list_parakeet_models")) as ParakeetModelInfo[];
                if (cancelled) return;
                setParakeetModels(pModels);

                const gModels = (await invoke("list_cohere_models")) as CohereModelInfo[];
                if (cancelled) return;
                setCohereModels(gModels);

                const engineState = await invoke<EngineSelectionState>("get_engine_selection_state");
                if (cancelled) return;

                const validWhisperModel = engineState.active_engine === "whisper" && engineState.selected_model_id && modelList.some((m) => m.id === engineState.selected_model_id)
                    ? engineState.selected_model_id
                    : modelList[0]?.id ?? null;
                const validParakeetModel = engineState.active_engine === "parakeet" && engineState.selected_model_id && pModels.some((m) => m.id === engineState.selected_model_id)
                    ? engineState.selected_model_id
                    : pModels[0]?.id ?? null;
                const validCohereModel = engineState.active_engine === "cohere" && engineState.selected_model_id && gModels.some((m) => m.id === engineState.selected_model_id)
                    ? engineState.selected_model_id
                    : gModels[0]?.id ?? null;

                setCurrentModel(validWhisperModel);
                setCurrentParakeetModel(validParakeetModel);
                setCurrentCohereModel(validCohereModel);
                if (engineState.loaded_engine) {
                    setLoadedEngine(engineState.loaded_engine);
                }

                let savedEngine: ASREngine | null = null;
                let savedCohereModel: string | null = null;
                try {
                    const loadedStore = await Store.load("settings.json");
                    if (cancelled) return;
                    storeRef.current = loadedStore;
                    await loadedStore.save(); // ensure the file exists on disk on first launch

                    const setupComplete = await loadedStore.get<boolean>("setup_complete");
                    if (!cancelled) setShowSetupWizard(setupComplete !== true);

                    // Restore saved hotkey binding
                    const savedHotkey = await loadedStore.get<{ keys: string[] }>("hotkey_binding");
                    if (savedHotkey?.keys?.length && !cancelled) {
                        invoke("set_hotkey", { binding: savedHotkey }).catch(() => {});
                    }

                    // Restore saved input device preference
                    const savedDevice = await loadedStore.get<string>("input_device");
                    if (savedDevice && !cancelled) {
                        invoke("set_input_device", { name: savedDevice }).catch(() => {});
                    }

                    // Restore close-button behavior preference
                    const savedCloseBehavior = await loadedStore.get<"tray" | "quit">("close_behavior");
                    if (savedCloseBehavior && !cancelled) {
                        setCloseBehavior(savedCloseBehavior);
                        invoke("set_close_behavior", { behaviour: savedCloseBehavior }).catch(() => {});
                    }

                    savedEngine =
                        (await loadedStore.get<ASREngine>("active_engine")) || null;
                    if (savedEngine) {
                        setActiveEngine(savedEngine);
                        activeEngineRef.current = savedEngine;
                    }

                    const savedParakeet = await loadedStore.get<string>("parakeet_model");
                    savedCohereModel = (await loadedStore.get<string>("granite_model")) ?? null;

                    const granitePick =
                        gModels.length > 0
                            ? savedCohereModel && gModels.some((m) => m.id === savedCohereModel)
                                ? savedCohereModel
                                : gModels[0].id
                            : "";
                    if (granitePick) setCurrentCohereModel(granitePick);

                    const savedAsrBackend = await loadedStore.get<"gpu" | "cpu">("asr_backend");
                    const useGpuPref = savedAsrBackend !== "cpu";

                    if (savedEngine === "whisper" && modelList.length > 0) {
                        const savedWhisper = await loadedStore.get<string>("whisper_model");
                        const targetWhisper =
                            savedWhisper && modelList.some((m) => m.id === savedWhisper)
                                ? savedWhisper
                                : validWhisperModel && modelList.some((m) => m.id === validWhisperModel)
                                  ? validWhisperModel
                                  : modelList[0].id;
                        const alreadyOk = engineState.loaded_engine === "whisper" && engineState.loaded_model_id === targetWhisper;
                        setCurrentModel(targetWhisper);
                        if (alreadyOk) {
                            setLoadedEngine("whisper");
                        }
                    } else if (savedEngine === "parakeet" && pModels.length > 0) {
                        const targetModel =
                            savedParakeet && pModels.find((m) => m.id === savedParakeet)
                                ? savedParakeet
                                : pModels[0].id;

                        isLoadingRef.current = true;
                        setIsLoading(true);
                        setLoadingMessage(`Loading Parakeet (${targetModel})...`);
                        try {
                            if (cancelled) return;
                            const result = await invoke<CommandResult<string>>("init_parakeet", {
                                modelId: targetModel,
                                useGpu: useGpuPref,
                            });
                            if (!result.ok) {
                                throw new Error(result.error?.message ?? "Failed to load Parakeet");
                            }
                            if (cancelled) return;
                            setCurrentParakeetModel(targetModel);
                            setLoadedEngine("parakeet");
                            setHeaderStatus("Parakeet model loaded");
                        } catch (e) {
                            if (cancelled) return;
                            setHeaderStatus(`Failed to auto-load Parakeet: ${e}`, 5000);
                        } finally {
                            if (!cancelled) {
                                isLoadingRef.current = false;
                                setIsLoading(false);
                                setLoadingMessage("");
                            }
                        }
                    } else if (savedEngine === "cohere" && granitePick) {
                        isLoadingRef.current = true;
                        setIsLoading(true);
                        setLoadingMessage("Loading Cohere Speech...");
                        try {
                            if (cancelled) return;
                            const result = await invoke<CommandResult<string>>("init_cohere", {
                                modelId: granitePick,
                                forceCpu:
                                    savedAsrBackend === "cpu" &&
                                    granitePick !== COHERE_FP16_MODEL_ID,
                            });
                            if (!result.ok) {
                                throw new Error(result.error?.message ?? "Failed to load Cohere Speech");
                            }
                            if (cancelled) return;
                            setLoadedEngine("cohere");
                            if (granitePick === COHERE_FP16_MODEL_ID) {
                                setAsrBackend("gpu");
                                await loadedStore.set("asr_backend", "gpu");
                                await loadedStore.save();
                            }
                            setHeaderStatus("Cohere Speech model loaded");
                        } catch (e) {
                            if (cancelled) return;
                            setHeaderStatus(`Failed to auto-load Cohere Speech: ${e}`, 5000);
                        } finally {
                            if (!cancelled) {
                                isLoadingRef.current = false;
                                setIsLoading(false);
                                setLoadingMessage("");
                            }
                        }
                    }
                } catch (storeErr) {
                    console.warn("Store load failed:", storeErr);
                    if (!cancelled) setShowSetupWizard(true);
                }

                if (!cancelled && engineState.loaded_engine && !savedEngine) {
                    setActiveEngine(engineState.loaded_engine);
                    activeEngineRef.current = engineState.loaded_engine;
                }
            } catch (e) {
                if (cancelled) return;
                console.error("Failed to load initial data:", e);
                setBackendInfo("Unknown");
                setHeaderStatus(`Error loading models: ${e}`, 5000);
                setShowSetupWizard(false);
            } finally {
                if (!cancelled) {
                    setIsInitialLoading(false);
                    invoke("show_main_window").catch(() => {});
                }
            }
        }

        loadInitialData();
        return () => {
            cancelled = true;
        };
    }, []); // eslint-disable-line react-hooks/exhaustive-deps
}
