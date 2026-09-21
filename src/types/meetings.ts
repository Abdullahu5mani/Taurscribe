export interface ActionItem {
    id: string;
    task: string;
    assignee: string;
    status: "todo" | "done" | "in_progress" | string;
}

export interface DiarizedTurn {
    speaker_id: string;
    speaker_name: string;
    start_ms: number;
    end_ms: number;
    channel: number; // 0 = You, 1 = Remote Callers
    text: string;
    snippet_path?: string | null;
    candidate_snippets?: string[];
    current_snippet_idx?: number;
}

export interface MeetingSummaryRecord {
    id: number;
    session_id: string;
    title: string;
    platform: string;
    app_name: string;
    url: string;
    created_at: string;
    duration_ms: number;
    category: string;
    speaker_count: number;
    snippet_preview: string;
    action_item_count: number;
    has_audio: boolean;
    /** Had a recording whose file is now gone; the transcript is still there. */
    audio_missing?: boolean;
}

export interface MeetingDetailRecord {
    id: number;
    session_id: string;
    title: string;
    platform: string;
    app_name: string;
    url: string;
    created_at: string;
    duration_ms: number;
    audio_path?: string | null;
    transcript_raw: string;
    summary: string[];
    action_items: ActionItem[];
    category: string;
    speaker_count: number;
    turns: DiarizedTurn[];
    /** The recording file is gone (deleted or moved); the transcript is complete. */
    audio_missing?: boolean;
    /** Speaker clips whose files are gone (left out of `turns`). */
    clips_missing?: number;
}

export interface SpeakerVaultRecord {
    id: string;
    name: string;
    created_at: string;
    /** Voice samples averaged into this person's voiceprint. */
    sample_count: number;
    /** Meetings this person appeared in. */
    meeting_count: number;
    last_seen: string;
    snippet_path?: string | null;
    candidate_snippets?: string[];
    /** The voice clip file is gone (recognition still works). */
    clip_missing?: boolean;
}

export interface PlatformCount {
    platform: string;
    count: number;
}

