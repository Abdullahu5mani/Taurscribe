import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { MODELS } from "../components/settings/types";
import type { DownloadableModel, DownloadProgress } from "../components/settings/types";

interface UseModelsWatcherParams {
    refreshModels: (showToast?: boolean) => Promise<void>;
    downloadProgressRef: React.RefObject<Record<string, DownloadProgress>>;
    setSettingsModels: React.Dispatch<React.SetStateAction<DownloadableModel[]>>;
}

interface DownloadStatus {
    id: string;
    downloaded: boolean;
    verified: boolean;
}

/**
 * Listens for the `models-changed` event emitted by the Rust file watcher
 * and refreshes both the backend model lists and the AppMall download-status
 * flags (downloaded / verified). Active download/verify/delete operations are
 * skipped to avoid clobbering in-flight state with partial on-disk reads.
 */
export function useModelsWatcher({
    refreshModels,
    downloadProgressRef,
    setSettingsModels,
}: UseModelsWatcherParams) {
    useEffect(() => {
        let active = true;
        let unlisten: (() => void) | undefined;
        let debounceTimer: ReturnType<typeof setTimeout> | null = null;
        let refreshInFlight = false;
        let refreshQueued = false;

        const runRefresh = async () => {
            if (refreshInFlight) {
                refreshQueued = true;
                return;
            }

            refreshInFlight = true;
            try {
                // Refresh backend model lists (Whisper + Parakeet + Granite)
                await refreshModels(false);

                // Refresh AppMall status (downloaded / verified flags) so the UI
                // reflects SHA-256 verification results as soon as they complete.
                const statuses = await invoke<DownloadStatus[]>("get_download_status", {
                    modelIds: MODELS.map((m) => m.id),
                });
                if (!active) return;

                const statusById = new Map(statuses.map((status) => [status.id, status]));
                const activeOps = downloadProgressRef.current ?? {};
                setSettingsModels((prev) =>
                    prev.map((m) => {
                        // Don't overwrite state for models with an active operation
                        // (download, verify, delete) — the FS watcher sees partial
                        // files on disk and would prematurely report them as downloaded.
                        const op = activeOps[m.id];
                        if (
                            op &&
                            [
                                "starting",
                                "downloading",
                                "extracting",
                                "verifying",
                                "finalizing",
                                "deleting",
                            ].includes(op.status)
                        ) {
                            return m;
                        }
                        const s = statusById.get(m.id);
                        return s ? { ...m, downloaded: s.downloaded, verified: s.verified } : m;
                    })
                );
            } catch (e) {
                console.error("Failed to refresh models after models-changed:", e);
            } finally {
                refreshInFlight = false;
                if (refreshQueued && active) {
                    refreshQueued = false;
                    void runRefresh();
                }
            }
        };

        const handleModelsChanged = () => {
            if (debounceTimer !== null) {
                clearTimeout(debounceTimer);
            }
            debounceTimer = setTimeout(() => {
                debounceTimer = null;
                void runRefresh();
            }, 120);
        };

        const setup = async () => {
            const unsub = await listen("models-changed", handleModelsChanged);
            if (active) unlisten = unsub;
            else unsub();
        };

        setup();
        return () => {
            active = false;
            if (debounceTimer !== null) {
                clearTimeout(debounceTimer);
            }
            if (unlisten) unlisten();
        };
    }, [refreshModels, downloadProgressRef, setSettingsModels]);
}
