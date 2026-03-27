import type { ASREngine } from "../hooks/useEngineSwitch";

/** Full FP16 ONNX bundle (`granite-speech-1b-fp16`) — backend has no CPU path. */
export const GRANITE_FP16_MODEL_ID = "granite-speech-1b-fp16";

/**
 * Maps a model ID to the engine that owns it, by prefix convention.
 * Single source of truth — replaces the inline if-chain that appeared
 * at multiple sites in App.tsx and settings components.
 */
export function getEngineForModelId(id: string): ASREngine | null {
    if (id.startsWith("parakeet")) return "parakeet";
    if (id.startsWith("granite")) return "granite_speech";
    if (id.startsWith("whisper")) return "whisper";
    return null;
}
