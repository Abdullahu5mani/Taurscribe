import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { MODELS } from "../components/settings/types";
import type { DownloadableModel, DownloadProgress } from "../components/settings/types";

interface UseModelsWatcherParams {
    refreshModels: (force: boolean) => void;
    downloadProgressRef: React.RefObject<Record<string, DownloadProgress>>;
    setSettingsModels: React.Dispatch<React.SetStateAction<DownloadableModel[]>>;
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

        const handleModelsChanged = async () => {
            // Refresh backend model lists (Whisper + Parakeet + Granite)
            refreshModels(false);

            // Refresh AppMall status (downloaded / verified flags) so the UI
            // reflects SHA-256 verification results as soon as they complete.
            try {
                const statuses = await invoke<any[]>("get_download_status", {
                    modelIds: MODELS.map((m) => m.id),
                });
                if (!active) return;
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
                        const s = statuses.find((x) => x.id === m.id);
                        return s ? { ...m, downloaded: s.downloaded, verified: s.verified } : m;
                    })
                );
            } catch (e) {
                console.error("Failed to refresh model statuses after models-changed:", e);
            }
        };

        const setup = async () => {
            const unsub = await listen("models-changed", handleModelsChanged);
            if (active) unlisten = unsub;
            else unsub();
        };

        setup();
        return () => {
            active = false;
            if (unlisten) unlisten();
        };
    }, []); // eslint-disable-line react-hooks/exhaustive-deps
}
