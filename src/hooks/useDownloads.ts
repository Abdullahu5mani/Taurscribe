import { useState, useEffect, useRef, useCallback } from "react";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { invoke } from "@tauri-apps/api/core";
import { DownloadProgress } from "../components/settings/types";

interface DownloadProgressPayload {
    model_id: string;
    total_bytes: number;
    downloaded_bytes: number;
    status: string;
    current_file?: number;
    total_files?: number;
}

const STALL_CHECK_INTERVAL_MS = 5_000;
const STALL_THRESHOLD_MS = 35_000;

export function useDownloads(
    onModelDownloaded: (id: string) => void,
    onDownloadFailed?: (id: string) => void | Promise<void>,
) {
    const [downloadProgress, setDownloadProgress] = useState<Record<string, DownloadProgress>>({});
    const activeDownloadsRef = useRef<Set<string>>(new Set());
    const cancelledRef = useRef<Set<string>>(new Set());
    const lastActivityRef = useRef<Record<string, { bytes: number; time: number }>>({});
    const onDownloadFailedRef = useRef(onDownloadFailed);
    onDownloadFailedRef.current = onDownloadFailed;

    const clearProgress = useCallback((modelId: string) => {
        setDownloadProgress((prev) => {
            const next = { ...prev };
            delete next[modelId];
            return next;
        });
        delete lastActivityRef.current[modelId];
    }, []);

    const markError = useCallback((modelId: string, message: string) => {
        activeDownloadsRef.current.delete(modelId);
        delete lastActivityRef.current[modelId];
        toast.error(message);
        void Promise.resolve(onDownloadFailedRef.current?.(modelId)).catch((err) => {
            console.warn("onDownloadFailed failed:", err);
        });
        clearProgress(modelId);
    }, [clearProgress]);

    // Stall detector: fires every STALL_CHECK_INTERVAL_MS, checks each active
    // download for progress. If bytes haven't advanced in STALL_THRESHOLD_MS,
    // auto-cancel and surface an error so the UI never hangs.
    useEffect(() => {
        const timer = setInterval(() => {
            const now = Date.now();
            for (const modelId of activeDownloadsRef.current) {
                const activity = lastActivityRef.current[modelId];
                if (!activity) continue;
                if (now - activity.time > STALL_THRESHOLD_MS) {
                    console.warn(`[STALL] No progress for ${modelId} in ${STALL_THRESHOLD_MS}ms — cancelling`);
                    invoke("cancel_download", { modelId }).catch(() => {});
                    markError(modelId, "Download failed — connection stalled. Check your internet and try again.");
                }
            }
        }, STALL_CHECK_INTERVAL_MS);
        return () => clearInterval(timer);
    }, [markError]);

    useEffect(() => {
        let unlisten: (() => void) | undefined;

        const setupListener = async () => {
            unlisten = await listen("download-progress", (event) => {
                const payload = event.payload as DownloadProgressPayload;

                // Track last-seen activity for stall detection.
                if (payload.status === "downloading" || payload.status === "extracting" || payload.status === "verifying") {
                    const prev = lastActivityRef.current[payload.model_id];
                    if (!prev || prev.bytes !== payload.downloaded_bytes) {
                        lastActivityRef.current[payload.model_id] = {
                            bytes: payload.downloaded_bytes,
                            time: Date.now(),
                        };
                    }
                }

                setDownloadProgress((prev) => ({
                    ...prev,
                    [payload.model_id]: {
                        bytes: payload.downloaded_bytes,
                        total: payload.total_bytes,
                        status: payload.status,
                        current_file: payload.current_file,
                        total_files: payload.total_files,
                    },
                }));

                if (payload.status === "done") {
                    setDownloadProgress((prev) => ({
                        ...prev,
                        [payload.model_id]: {
                            bytes: payload.downloaded_bytes,
                            total: payload.total_bytes,
                            status: "finalizing",
                            current_file: payload.current_file,
                            total_files: payload.total_files,
                        },
                    }));
                    Promise.resolve(onModelDownloaded(payload.model_id))
                        .catch((err) => {
                            console.warn("onModelDownloaded failed:", err);
                        })
                        .finally(() => {
                            activeDownloadsRef.current.delete(payload.model_id);
                            clearProgress(payload.model_id);
                            toast.success(`Downloaded: ${payload.model_id}`);
                        });
                } else if (payload.status === "error") {
                    markError(
                        payload.model_id,
                        "Download failed — check your internet connection and try again.",
                    );
                } else if (payload.status === "cancelled") {
                    activeDownloadsRef.current.delete(payload.model_id);
                    cancelledRef.current.add(payload.model_id);
                    delete lastActivityRef.current[payload.model_id];
                    toast.info(`Download cancelled: ${payload.model_id}`);
                    clearProgress(payload.model_id);
                } else if (payload.status === "delete-done") {
                    clearProgress(payload.model_id);
                }
            });
        };

        setupListener();

        return () => {
            if (unlisten) unlisten();
        };
    }, [onModelDownloaded, clearProgress, markError]);

    const handleDownload = async (id: string, name: string) => {
        if (activeDownloadsRef.current.has(id)) {
            return;
        }
        activeDownloadsRef.current.add(id);
        lastActivityRef.current[id] = { bytes: 0, time: Date.now() };
        toast.info(`Starting download: ${name}`);
        setDownloadProgress((prev) => ({
            ...prev,
            [id]: { bytes: 0, total: 100, status: "starting" },
        }));
        try {
            await invoke("download_model", { modelId: id });
        } catch (e) {
            activeDownloadsRef.current.delete(id);
            delete lastActivityRef.current[id];
            if (cancelledRef.current.has(id)) {
                cancelledRef.current.delete(id);
                return;
            }
            const raw = `${e ?? "Unknown error"}`;
            const message = raw.includes("internet") || raw.includes("onnect") || raw.includes("stall") || raw.includes("lost")
                ? raw
                : `Download failed — ${raw}`;
            toast.error(message);
            void Promise.resolve(onDownloadFailedRef.current?.(id)).catch((err) => {
                console.warn("onDownloadFailed failed:", err);
            });
            clearProgress(id);
        }
    };

    const handleCancelDownload = async (id: string) => {
        try {
            await invoke("cancel_download", { modelId: id });
        } catch (e) {
            console.warn("cancel_download failed:", e);
        }
    };

    const handleVerify = async () => {
        // Stub: verification is now automatic on download.
    };

    return {
        downloadProgress,
        handleDownload,
        handleCancelDownload,
        handleVerify,
    };
}
