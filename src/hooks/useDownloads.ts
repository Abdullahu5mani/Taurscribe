import { useState, useEffect, useRef, useCallback } from "react";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { invoke } from "@tauri-apps/api/core";
import { DownloadProgress } from "../components/settings/types";

interface DownloadProgressPayload {
    model_id: string;
    request_id?: string | null;
    error?: string | null;
    total_bytes: number;
    downloaded_bytes: number;
    status: string;
    current_file?: number;
    total_files?: number;
}

const STALL_CHECK_INTERVAL_MS = 5_000;
const STALL_THRESHOLD_MS = 35_000;

const downloadErrorMessage = (raw: string) => raw.includes("internet") || raw.includes("onnect") || raw.includes("stall") || raw.includes("lost")
    ? raw : `Download failed — ${raw}`;

export function useDownloads(
    onModelDownloaded: (id: string) => void,
    onDownloadFailed?: (id: string) => void | Promise<void>,
) {
    const [downloadProgress, setDownloadProgress] = useState<Record<string, DownloadProgress>>({});
    const activeDownloadsRef = useRef<Set<string>>(new Set());
    const stalledRef = useRef<Set<string>>(new Set());
    const lastActivityRef = useRef<Record<string, { bytes: number; time: number; phase: string }>>({});
    const attemptsRef = useRef(new Map<string, string>());
    const settledRef = useRef(new Set<string>());
    const completionsRef = useRef(new Map<string, Promise<void>>());
    const onModelDownloadedRef = useRef(onModelDownloaded);
    onModelDownloadedRef.current = onModelDownloaded;
    const onDownloadFailedRef = useRef(onDownloadFailed);
    onDownloadFailedRef.current = onDownloadFailed;

    const updateProgressState = useCallback(
        (payload: DownloadProgressPayload, statusOverride?: string) => {
            const nextStatus = statusOverride ?? payload.status;
            setDownloadProgress((prev) => {
                const current = prev[payload.model_id];
                const next: DownloadProgress = {
                    bytes: payload.downloaded_bytes,
                    total: payload.total_bytes,
                    status: nextStatus,
                    current_file: payload.current_file,
                    total_files: payload.total_files,
                };

                if (
                    current &&
                    current.bytes === next.bytes &&
                    current.total === next.total &&
                    current.status === next.status &&
                    current.current_file === next.current_file &&
                    current.total_files === next.total_files
                ) {
                    return prev;
                }

                return {
                    ...prev,
                    [payload.model_id]: next,
                };
            });
        },
        [],
    );

    const clearProgress = useCallback((modelId: string) => {
        setDownloadProgress((prev) => {
            const next = { ...prev };
            delete next[modelId];
            return next;
        });
        delete lastActivityRef.current[modelId];
    }, []);

    const markError = useCallback((modelId: string, message: string) => {
        settledRef.current.add(modelId);
        activeDownloadsRef.current.delete(modelId);
        delete lastActivityRef.current[modelId];
        toast.error(message);
        void Promise.resolve().then(() => onDownloadFailedRef.current?.(modelId)).catch((err) => {
            console.warn("onDownloadFailed failed:", err);
        });
        clearProgress(modelId);
    }, [clearProgress]);

    const completeDownload = useCallback((modelId: string) => {
        const previous = completionsRef.current.get(modelId);
        if (previous) return previous;
        settledRef.current.add(modelId);
        const attempt = attemptsRef.current.get(modelId);
        stalledRef.current.delete(modelId);
        setDownloadProgress(prev => ({ ...prev, [modelId]: { ...prev[modelId], bytes: 100, total: 100, status: "finalizing" } }));
        const completion = Promise.resolve().then(() => onModelDownloadedRef.current(modelId))
            .catch(err => console.warn("onModelDownloaded failed:", err))
            .finally(() => {
                if (attemptsRef.current.get(modelId) !== attempt) return;
                activeDownloadsRef.current.delete(modelId);
                clearProgress(modelId);
                toast.success(`Downloaded: ${modelId}`);
            });
        completionsRef.current.set(modelId, completion);
        return completion;
    }, [clearProgress]);

    // Stall detector: fires every STALL_CHECK_INTERVAL_MS, checks each active
    // download for progress. If bytes haven't advanced in STALL_THRESHOLD_MS,
    // auto-cancel and surface an error so the UI never hangs.
    useEffect(() => {
        const timer = setInterval(() => {
            const now = Date.now();
            for (const modelId of activeDownloadsRef.current) {
                if (completionsRef.current.has(modelId)) continue;
                const activity = lastActivityRef.current[modelId];
                if (!activity) continue;
                // Disk work may take arbitrarily long between chunks. The native
                // worker checks cancellation and reports progress itself.
                if (!["starting", "downloading"].includes(activity.phase)) continue;
                if (now - activity.time > STALL_THRESHOLD_MS) {
                    if (stalledRef.current.has(modelId)) continue;
                    stalledRef.current.add(modelId);
                    const attempt = attemptsRef.current.get(modelId);
                    console.warn(`[STALL] No progress for ${modelId} in ${STALL_THRESHOLD_MS}ms — cancelling`);
                    invoke<boolean>("cancel_download", { modelId, requestId: attempt })
                        .then((accepted) => { if (!accepted && attemptsRef.current.get(modelId) === attempt) stalledRef.current.delete(modelId); })
                        .catch(() => {
                            if (attemptsRef.current.get(modelId) !== attempt) return;
                            stalledRef.current.delete(modelId);
                            if (activeDownloadsRef.current.has(modelId)) {
                                markError(modelId, "Download failed — connection stalled. Check your internet and try again.");
                            }
                        });
                }
            }
        }, STALL_CHECK_INTERVAL_MS);
        return () => clearInterval(timer);
    }, [markError]);

    useEffect(() => {
        let unlisten: (() => void) | undefined;
        let disposed = false;

        const setupListener = async () => {
            const unsubscribe = await listen("download-progress", (event) => {
                if (disposed) return;
                const payload = event.payload as DownloadProgressPayload;
                // Native events and command replies can arrive in either order.
                // A previous attempt must never finish or cancel a retry.
                if (!["deleting", "delete-done"].includes(payload.status)) {
                    if (settledRef.current.has(payload.model_id)) return;
                    const attempt = attemptsRef.current.get(payload.model_id);
                    if (attempt && payload.request_id !== attempt) return;
                    if (!attempt && payload.request_id) attemptsRef.current.set(payload.model_id, payload.request_id);
                }

                // Track last-seen activity for stall detection.
                if (["downloading", "copying", "extracting", "verifying"].includes(payload.status)) {
                    if (completionsRef.current.has(payload.model_id)) return;
                    activeDownloadsRef.current.add(payload.model_id);
                    const prev = lastActivityRef.current[payload.model_id];
                    if (!prev || prev.bytes !== payload.downloaded_bytes || prev.phase !== payload.status) {
                        lastActivityRef.current[payload.model_id] = {
                            bytes: payload.downloaded_bytes,
                            time: Date.now(),
                            phase: payload.status,
                        };
                    }
                }

                if (payload.status === "done") {
                    void completeDownload(payload.model_id);
                } else if (payload.status === "error") {
                    if (!activeDownloadsRef.current.has(payload.model_id) || completionsRef.current.has(payload.model_id)) return;
                    const stalled = stalledRef.current.delete(payload.model_id);
                    markError(
                        payload.model_id,
                        stalled
                            ? "Download failed — connection stalled. Check your internet and try again."
                            : payload.error ? downloadErrorMessage(payload.error) : "Download failed — check your internet connection and try again.",
                    );
                } else if (payload.status === "cancelled") {
                    if (!activeDownloadsRef.current.has(payload.model_id) || completionsRef.current.has(payload.model_id)) return;
                    settledRef.current.add(payload.model_id);
                    const stalled = stalledRef.current.delete(payload.model_id);
                    if (stalled) {
                        markError(payload.model_id, "Download failed — connection stalled. Check your internet and try again.");
                        return;
                    }
                    activeDownloadsRef.current.delete(payload.model_id);
                    delete lastActivityRef.current[payload.model_id];
                    toast.info(`Download cancelled: ${payload.model_id}`);
                    clearProgress(payload.model_id);
                    void Promise.resolve().then(() => onDownloadFailedRef.current?.(payload.model_id)).catch(() => {});
                } else if (payload.status === "delete-done") {
                    clearProgress(payload.model_id);
                } else {
                    updateProgressState(payload);
                }
            });
            if (disposed) unsubscribe();
            else unlisten = unsubscribe;
        };

        void setupListener().catch(err => console.warn("Download progress listener failed:", err));

        return () => {
            disposed = true;
            if (unlisten) unlisten();
        };
    }, [completeDownload, clearProgress, markError, updateProgressState]);

    const handleDownload = async (id: string, name: string) => {
        if (activeDownloadsRef.current.has(id)) {
            return;
        }
        activeDownloadsRef.current.add(id);
        const attempt = crypto.randomUUID();
        attemptsRef.current.set(id, attempt);
        settledRef.current.delete(id);
        completionsRef.current.delete(id);
        stalledRef.current.delete(id);
        lastActivityRef.current[id] = { bytes: 0, time: Date.now(), phase: "starting" };
        toast.info(`Starting download: ${name}`);
        setDownloadProgress((prev) => ({
            ...prev,
            [id]: { bytes: 0, total: 100, status: "starting" },
        }));
        try {
            await invoke("download_model", { modelId: id, requestId: attempt });
            if (attemptsRef.current.get(id) === attempt) await completeDownload(id);
        } catch (e) {
            if (attemptsRef.current.get(id) !== attempt || completionsRef.current.has(id)) return;
            settledRef.current.add(id);
            const raw = `${e ?? "Unknown error"}`;
            if (raw.toLowerCase().includes("cancel")) {
                if (activeDownloadsRef.current.has(id)) {
                    if (stalledRef.current.delete(id)) {
                        markError(id, "Download failed — connection stalled. Check your internet and try again.");
                    } else {
                        activeDownloadsRef.current.delete(id);
                        clearProgress(id);
                        toast.info(`Download cancelled: ${id}`);
                        void Promise.resolve().then(() => onDownloadFailedRef.current?.(id)).catch(() => {});
                    }
                }
                return;
            }
            if (!activeDownloadsRef.current.has(id)) return;
            activeDownloadsRef.current.delete(id);
            delete lastActivityRef.current[id];
            toast.error(downloadErrorMessage(raw));
            void Promise.resolve().then(() => onDownloadFailedRef.current?.(id)).catch((err) => {
                console.warn("onDownloadFailed failed:", err);
            });
            clearProgress(id);
        }
    };

    const handleCancelDownload = async (id: string) => {
        const attempt = attemptsRef.current.get(id);
        try {
            const accepted = await invoke<boolean>("cancel_download", { modelId: id, requestId: attempt });
            if (accepted && attemptsRef.current.get(id) === attempt && activeDownloadsRef.current.has(id)) {
                setDownloadProgress(prev => prev[id]
                    ? { ...prev, [id]: { ...prev[id], status: "cancelling" } }
                    : prev);
            }
        } catch (e) {
            console.warn("cancel_download failed:", e);
        }
    };

    return {
        downloadProgress,
        handleDownload,
        handleCancelDownload,
    };
}
