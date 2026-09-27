import { invoke } from "@tauri-apps/api/core";
import type { CommandResult } from "../types/session";

/** The ASR engines (mirrors `ASREngine` in src-tauri/src/types.rs). */
export type EngineId = "whisper" | "granite" | "qwen3";

/** Thrown by `loadEngineModel` / `ensureEngineModel`: the backend's `CommandResult` error. */
export class EngineLoadError extends Error {
    code: string;
    constructor(code: string, message: string) {
        super(message);
        this.name = "EngineLoadError";
        this.code = code;
    }
}

/** The Tauri command that loads each engine's model. */
const LOAD_COMMAND: Record<EngineId, string> = {
    whisper: "switch_model",
    granite: "init_granite",
    qwen3: "init_qwen3",
};

/** Whether `modelId` is the model currently loaded for `engine`. */
export async function isEngineModelLoaded(engine: EngineId, modelId: string): Promise<boolean> {
    if (engine === "whisper") {
        const loaded = ((await invoke<string | null>("get_current_model")) ?? "").trim();
        return loaded !== "" && loaded === modelId;
    }
    const status = await invoke<{ loaded: boolean; model_id?: string | null }>(
        engine === "granite" ? "get_granite_status" : "get_qwen3_status",
    );
    return status.loaded && status.model_id === modelId;
}

/**
 * Loads `modelId` for `engine` (unloading the other engines; see
 * commands/models.rs). Throws an `EngineLoadError` when the backend reports a
 * failure, so callers can map `code` to a notice.
 */
export async function loadEngineModel(engine: EngineId, modelId: string, useGpu: boolean): Promise<string> {
    const result = await invoke<CommandResult<string>>(LOAD_COMMAND[engine], { modelId, useGpu });
    if (!result.ok) {
        throw new EngineLoadError(
            result.error?.code ?? "model_load_failed",
            result.error?.message ?? `Failed to load ${engine} model`,
        );
    }
    return result.data ?? "";
}

/**
 * Makes sure `modelId` is loaded for `engine`, loading it only when needed.
 * Returns "already" when nothing had to be done and "loaded" after a load.
 * `onLoading` runs just before a load starts (for status messages).
 */
export async function ensureEngineModel(
    engine: EngineId,
    modelId: string,
    useGpu: boolean,
    onLoading?: () => void,
): Promise<"already" | "loaded"> {
    if (await isEngineModelLoaded(engine, modelId)) return "already";
    onLoading?.();
    await loadEngineModel(engine, modelId, useGpu);
    return "loaded";
}
