import type { ASREngine } from "../hooks/useEngineSwitch";

/** Legacy Cohere FP16 id kept for backward-compatible settings migration. */
export const COHERE_FP16_MODEL_ID = "granite-speech-1b-fp16";

/**
 * Maps a model ID to the engine that owns it, by prefix convention.
 * Single source of truth — replaces the inline if-chain that appeared
 * at multiple sites in App.tsx and settings components.
 */
export function getEngineForModelId(id: string): ASREngine | null {
    if (id.startsWith("parakeet")) return "parakeet";
    if (id.startsWith("granite")) return "cohere";
    if (id.startsWith("whisper")) return "whisper";
    return null;
}
