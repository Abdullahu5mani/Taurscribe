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

export interface EngineSelectionState {
  active_engine: "whisper" | "parakeet" | "cohere";
  selected_model_id: string | null;
  loaded_engine: "whisper" | "parakeet" | "cohere" | null;
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
