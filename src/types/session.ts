export type SessionPhase =
  | "idle"
  | "loading_model"
  | "recording"
  | "paused"
  | "processing"
  | "success"
  | "warning"
  | "error";

export type SessionErrorCode =
  | "already_recording"
  | "not_recording"
  | "mic_permission_denied"
  | "no_input_device"
  | "model_missing"
  | "engine_loading"
  | "paste_blocked_secure_input"
  | "paste_blocked_console"
  | "audio_device_disconnected"
  | "recording_too_short"
  | "nothing_heard"
  | "model_load_failed"
  | "paste_failed"
  | "recording_start_failed"
  | "recording_stop_failed"
  | "unknown";

export interface CommandErrorPayload {
  code: SessionErrorCode | string;
  message: string;
}

export interface CommandResult<T> {
  ok: boolean;
  data: T | null;
  error: CommandErrorPayload | null;
}

/** What `stop_recording` returns (RecordingOutcome in recording.rs). */
export interface RecordingOutcome {
  transcript: string;
  /** Decided when the recording started: two-channel recordings are meetings. */
  kind: "dictation" | "meeting";
  meeting_id: number | null;
  /** Length of the captured audio; excludes the processing after Stop. */
  duration_ms: number | null;
  /** A dictation too short to keep: not saved, and not to be pasted. */
  too_short: boolean;
  /** A dictation is saved to history before its recording is deleted. */
  history:
    | { status: "saved"; id: number }
    | { status: "not_saved"; error: string; recording_path: string | null }
    | { status: "skipped" };
}

export interface EngineSelectionState {
  active_engine: "whisper" | "granite" | "qwen3";
  selected_model_id: string | null;
  loaded_engine: "whisper" | "granite" | "qwen3" | null;
  loaded_model_id: string | null;
  backend: string;
  engine_loading: boolean;
}

export interface SessionNoticeAction {
  id: string;
  label: string;
  onClick: () => void;
}

export interface SessionNotice {
  level: "warning" | "error" | "success";
  code: SessionErrorCode | string;
  title: string;
  message: string;
  sticky?: boolean;
  actions?: SessionNoticeAction[];
}

export interface SessionState {
  phase: SessionPhase;
  notice: SessionNotice | null;
  lastTranscript: string;
  latestLatency: number | null;
}
