import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { FileHistoryRecord } from "../types/fileHistory";

export const FILE_HISTORY_PAGE_SIZE = 25;

export function useFileHistory(refreshKey: number) {
    const [records, setRecords] = useState<FileHistoryRecord[]>([]);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);
    const [hasMore, setHasMore] = useState(false);
    const offset = useRef(0);
    const generation = useRef(0);
    const busy = useRef(false);

    const fetchPage = useCallback(async (reset: boolean) => {
        if (!reset && busy.current) return;
        const request = ++generation.current;
        const start = reset ? 0 : offset.current;
        busy.current = true;
        setLoading(true);
        setError(null);
        try {
            const page = await invoke<FileHistoryRecord[]>("list_transcript_history", {
                kind: "file", limit: FILE_HISTORY_PAGE_SIZE, offset: start,
            });
            if (request !== generation.current) return;
            offset.current = start + page.length;
            setRecords(previous => {
                if (reset) return page;
                // A new save can shift offset pages; never show duplicate rows.
                const ids = new Set(previous.map(row => row.id));
                return [...previous, ...page.filter(row => !ids.has(row.id))];
            });
            setHasMore(page.length === FILE_HISTORY_PAGE_SIZE);
        } catch (cause) {
            if (request === generation.current) setError(String(cause));
        } finally {
            if (request === generation.current) {
                busy.current = false;
                setLoading(false);
            }
        }
    }, []);

    const refresh = useCallback(() => fetchPage(true), [fetchPage]);
    const loadMore = useCallback(() => fetchPage(false), [fetchPage]);
    useEffect(() => {
        void refresh();
        return () => { ++generation.current; busy.current = false; };
    }, [refresh, refreshKey]);

    return { records, loading, error, hasMore, refresh, loadMore };
}
