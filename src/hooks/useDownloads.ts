import { useState, useEffect } from "react";
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
                    toast.success(`Downloaded: ${payload.model_id}`);
                    setDownloadProgress((prev) => {
                        const next = { ...prev };
                        delete next[payload.model_id];
                        return next;
                    });
                    onModelDownloaded(payload.model_id);
                } else if (payload.status === "error") {
                    setDownloadProgress((prev) => {
                        const next = { ...prev };
                        delete next[payload.model_id];
                        return next;
                    });
                } else if (payload.status === "delete-done") {
                    // Clean up progress entry after deletion completes — no callback needed.
                    setDownloadProgress((prev) => {
                        const next = { ...prev };
                        delete next[payload.model_id];
                        return next;
                    });
                }
            });
        };

        setupListener();

        return () => {
            if (unlisten) unlisten();
        };
    }, [onModelDownloaded]);

    const handleDownload = async (id: string, name: string) => {
        toast.info(`Starting download: ${name}`);
        setDownloadProgress((prev) => ({
            ...prev,
            [id]: { bytes: 0, total: 100, status: "starting" },
        }));
        try {
            await invoke("download_model", { modelId: id });
        } catch (e) {
            toast.error(`Download failed: ${e}`);
            setDownloadProgress((prev) => {
                const n = { ...prev };
                delete n[id];
                return n;
            });
        }
    };

    const handleVerify = async () => {
        // Stub: verification is now automatic on download.
    };

    return {
        downloadProgress,
        handleDownload,
        handleVerify,
    };
}
