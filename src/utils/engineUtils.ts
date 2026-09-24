import type { ASREngine } from "../hooks/useEngineSwitch";

/**
 * Maps a model ID to the engine that owns it, by prefix convention.
 * Single source of truth — replaces the inline if-chain that appeared
 * at multiple sites in App.tsx and settings components.
 */
export function getEngineForModelId(id: string): ASREngine | null {
    if (id.startsWith("granite")) return "granite";
    if (id.startsWith("whisper")) return "whisper";
    if (id.startsWith("qwen3")) return "qwen3";
    return null;
}
