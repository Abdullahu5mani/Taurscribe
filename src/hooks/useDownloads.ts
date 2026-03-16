import { useState, useEffect, useRef } from "react";
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

export function useDownloads(onModelDownloaded: (id: string) => void) {
    const [downloadProgress, setDownloadProgress] = useState<Record<string, DownloadProgress>>({});
    const activeDownloadsRef = useRef<Set<string>>(new Set());
    const cancelledRef = useRef<Set<string>>(new Set());

    const clearProgress = (modelId: string) => {
        setDownloadProgress((prev) => {
            const next = { ...prev };
            delete next[modelId];
            return next;
        });
    };

    useEffect(() => {
        let unlisten: (() => void) | undefined;

        const setupListener = async () => {
            unlisten = await listen("download-progress", (event) => {
                const payload = event.payload as DownloadProgressPayload;

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
                    activeDownloadsRef.current.delete(payload.model_id);
                    setDownloadProgress((prev) => ({
                        ...prev,
                        [payload.model_id]: {
                            bytes: payload.downloaded_bytes,
                            total: payload.total_bytes,
                            status: "error",
                            current_file: payload.current_file,
                            total_files: payload.total_files,
                            error: "Download failed — file may be corrupted. Try again.",
                        },
                    }));
                    toast.error(`Download failed — file may be corrupted. Try again.`);
                } else if (payload.status === "cancelled") {
                    activeDownloadsRef.current.delete(payload.model_id);
                    cancelledRef.current.add(payload.model_id);
                    toast.info(`Download cancelled: ${payload.model_id}`);
                    clearProgress(payload.model_id);
                } else if (payload.status === "delete-done") {
                    // Clean up progress entry after deletion completes — no callback needed.
                    clearProgress(payload.model_id);
                }
            });
        };

        setupListener();

        return () => {
            if (unlisten) unlisten();
        };
    }, [onModelDownloaded]);

    const handleDownload = async (id: string, name: string) => {
        const current = downloadProgress[id];
        if (activeDownloadsRef.current.has(id) || (current && current.status !== "error")) {
            return;
        }
        activeDownloadsRef.current.add(id);
        toast.info(`Starting download: ${name}`);
        setDownloadProgress((prev) => ({
            ...prev,
            [id]: { bytes: 0, total: 100, status: "starting" },
        }));
        try {
            await invoke("download_model", { modelId: id });
        } catch (e) {
            activeDownloadsRef.current.delete(id);
            // If a cancellation was requested, the "cancelled" event already
            // cleaned up the progress state — don't overwrite it with an error.
            if (cancelledRef.current.has(id)) {
                cancelledRef.current.delete(id);
                return;
            }
            const message = `${e ?? "Unknown error"}`;
            toast.error(`Download failed: ${e}`);
            setDownloadProgress((prev) => ({
                ...prev,
                [id]: {
                    ...(prev[id] ?? { bytes: 0, total: 100 }),
                    status: "error",
                    error: message,
                },
            }));
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
