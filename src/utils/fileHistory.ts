import { invoke } from "@tauri-apps/api/core";
import type { FileImportMethod } from "../types/fileHistory";

interface CompletedFile {
    id?: string;
    path: string;
    transcript: string;
    engine: string;
    modelId?: string | null;
    audioDurationMs?: number;
    processingTimeMs?: number;
    importMethod: FileImportMethod;
    completedAt?: string;
}

/** Only text and metadata go to history; the source audio is never copied. */
export async function saveFileHistory(item: CompletedFile): Promise<void> {
    if (!item.transcript.trim()) return;
    await invoke("save_transcript_history", {
        transcript: item.transcript,
        engine: item.engine,
        durationMs: item.audioDurationMs ?? null,
        grammarLlmUsed: false,
        processingTimeMs: item.processingTimeMs ?? null,
        modelId: item.modelId ?? null,
        audioSource: item.path.split(/[\\/]/).pop() ?? item.path,
        sourcePath: item.path,
        importMethod: item.importMethod,
        completedAt: item.completedAt ?? null,
        ...(item.id ? { jobId: item.id } : {}),
    });
}

export function importMethodLabel(method: string | null): string {
    if (method === "drop") return "Dragged and dropped";
    if (method === "picker") return "Selected with file picker";
    return "Unknown (older entry)";
}
