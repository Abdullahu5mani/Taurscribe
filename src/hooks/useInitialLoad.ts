import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Store } from "@tauri-apps/plugin-store";
import { MODELS } from "../components/settings/types";
import type { DownloadableModel } from "../components/settings/types";
import type { ModelInfo, ParakeetModelInfo, GraniteSpeechModelInfo } from "./useModels";
import type { ASREngine } from "./useEngineSwitch";

interface UseInitialLoadParams {
    // Model state setters
    setModels: (models: ModelInfo[]) => void;
    setCurrentModel: (id: string | null) => void;
    setParakeetModels: (models: ParakeetModelInfo[]) => void;
    setCurrentParakeetModel: (id: string) => void;
    setGraniteModels: (models: GraniteSpeechModelInfo[]) => void;
    setCurrentGraniteModel: (id: string) => void;
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
    setOverlayStyle: (v: "minimal" | "full") => void;

    // Store ref — populated by this hook so callers can use it later
    storeRef: React.MutableRefObject<Store | null>;
}

/**
 * Runs the app's startup sequence exactly once on mount:
 *   1. Fetches backend info and all model lists
 *   2. Pre-fetches download status for all known models
 *   3. Loads and restores settings.json (engine, hotkey, device, close-behavior, overlay)
 *   4. Auto-loads Parakeet or Granite Speech if they were active on last run
 */
export function useInitialLoad({
    setModels,
    setCurrentModel,
    setParakeetModels,
    setCurrentParakeetModel,
    setGraniteModels,
    setCurrentGraniteModel,
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
    setOverlayStyle,
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

                const current = (await invoke("get_current_model")) as string | null;
                if (cancelled) return;
                setCurrentModel(current ?? "");
                if (current) setLoadedEngine("whisper");

                const pModels = (await invoke("list_parakeet_models")) as ParakeetModelInfo[];
                if (cancelled) return;
                setParakeetModels(pModels);

                const pStatus = (await invoke("get_parakeet_status")) as {
                    loaded: boolean;
                    model_id: string | null;
                };
                if (cancelled) return;
                setCurrentParakeetModel(pStatus.model_id ?? "");

                const gModels = (await invoke("list_granite_models")) as GraniteSpeechModelInfo[];
                if (cancelled) return;
                setGraniteModels(gModels);
                if (gModels.length > 0) setCurrentGraniteModel(gModels[0].id);

                let savedEngine: ASREngine | null = null;
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

                    // Restore overlay style preference
                    const savedOverlayStyle = await loadedStore.get<"minimal" | "full">("overlay_style");
                    if (savedOverlayStyle && !cancelled) {
                        setOverlayStyle(savedOverlayStyle);
                    }

                    savedEngine =
                        (await loadedStore.get<ASREngine>("active_engine")) || null;
                    if (savedEngine) {
                        setActiveEngine(savedEngine);
                        activeEngineRef.current = savedEngine;
                    }

                    const savedParakeet = await loadedStore.get<string>("parakeet_model");

                    if (savedEngine === "parakeet" && pModels.length > 0) {
                        const targetModel =
                            savedParakeet && pModels.find((m) => m.id === savedParakeet)
                                ? savedParakeet
                                : pModels[0].id;

                        isLoadingRef.current = true;
                        setIsLoading(true);
                        setLoadingMessage(`Loading Parakeet (${targetModel})...`);
                        try {
                            if (cancelled) return;
                            await invoke("init_parakeet", { modelId: targetModel });
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
                    } else if (savedEngine === "granite_speech" && gModels.length > 0) {
                        isLoadingRef.current = true;
                        setIsLoading(true);
                        setLoadingMessage("Loading Granite Speech...");
                        try {
                            if (cancelled) return;
                            await invoke("init_granite_speech", {});
                            if (cancelled) return;
                            setCurrentGraniteModel(gModels[0].id);
                            setLoadedEngine("granite_speech");
                            setHeaderStatus("Granite Speech model loaded");
                        } catch (e) {
                            if (cancelled) return;
                            setHeaderStatus(`Failed to auto-load Granite Speech: ${e}`, 5000);
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

                if (!cancelled && pStatus.loaded && !current && !savedEngine) {
                    setActiveEngine("parakeet");
                    activeEngineRef.current = "parakeet";
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
