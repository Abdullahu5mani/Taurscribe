import type { ASREngine } from "../hooks/useEngineSwitch";

/** Legacy constant name kept for compatibility with existing engine-slot code. */
export const GRANITE_MODEL_ID = "granite-speech-4.1-2b-nar";
export const COHERE_FP16_MODEL_ID = GRANITE_MODEL_ID;

/**
 * Maps a model ID to the engine that owns it, by prefix convention.
 * Single source of truth — replaces the inline if-chain that appeared
 * at multiple sites in App.tsx and settings components.
 */
export function getEngineForModelId(id: string): ASREngine | null {
    if (id.startsWith("parakeet")) return "parakeet";
    if (id.startsWith("cohere")) return "granite";
    if (id.startsWith("granite")) return "granite";
    if (id.startsWith("whisper")) return "whisper";
    return null;
}
