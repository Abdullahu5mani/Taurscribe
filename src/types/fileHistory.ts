export type FileImportMethod = "drop" | "picker";

export interface FileHistoryRecord {
    id: number;
    created_at: string;
    transcript: string;
    engine: string;
    model_id: string | null;
    audio_source: string | null;
    source_path: string | null;
    import_method: string | null;
    duration_ms: number | null;
    processing_time_ms: number | null;
    grammar_llm_used: boolean;
    kind: string;
}
