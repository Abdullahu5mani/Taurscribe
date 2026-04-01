import { invoke } from "@tauri-apps/api/core";
import type { CommandResult } from "../types/session";

export async function invokeCommand<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<CommandResult<T>> {
  return invoke<CommandResult<T>>(command, args);
}

export function unwrapCommandResult<T>(result: CommandResult<T>): T {
  if (!result.ok || result.data == null) {
    const error = result.error ?? { code: "unknown", message: "Unknown command failure" };
    const err = new Error(error.message) as Error & { code?: string };
    err.code = error.code;
    throw err;
  }
  return result.data;
}
