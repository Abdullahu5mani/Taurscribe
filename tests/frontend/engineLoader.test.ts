import { beforeEach, describe, expect, mock, test } from "bun:test";

type Call = { cmd: string; args?: Record<string, unknown> };
let calls: Call[] = [];
let responses: Record<string, unknown> = {};

mock.module("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args });
    const r = responses[cmd];
    return typeof r === "function" ? (r as (a?: unknown) => unknown)(args) : r;
  },
}));

const { ensureEngineModel, isEngineModelLoaded, loadEngineModel } = await import("../../src/utils/engineLoader");

beforeEach(() => {
  calls = [];
  responses = {};
});

describe("isEngineModelLoaded", () => {
  test("whisper compares the loaded model id", async () => {
    responses.get_current_model = " small.en ";
    expect(await isEngineModelLoaded("whisper", "small.en")).toBe(true);
    expect(await isEngineModelLoaded("whisper", "base.en")).toBe(false);
    responses.get_current_model = null;
    expect(await isEngineModelLoaded("whisper", "small.en")).toBe(false);
  });

  test("granite and qwen3 read their status commands", async () => {
    responses.get_granite_status = { loaded: true, model_id: "granite-speech-5-nc" };
    responses.get_qwen3_status = { loaded: false, model_id: null };
    expect(await isEngineModelLoaded("granite", "granite-speech-5-nc")).toBe(true);
    expect(await isEngineModelLoaded("qwen3", "qwen3-asr-0.6b")).toBe(false);
    expect(calls.map((c) => c.cmd)).toEqual(["get_granite_status", "get_qwen3_status"]);
  });
});

describe("loadEngineModel", () => {
  test("calls the engine's load command with model and backend", async () => {
    responses.switch_model = { ok: true, data: "Backend: Metal", error: null };
    responses.init_granite = { ok: true, data: "Granite loaded", error: null };
    responses.init_qwen3 = { ok: true, data: "Qwen3 loaded", error: null };
    expect(await loadEngineModel("whisper", "small.en", true)).toBe("Backend: Metal");
    await loadEngineModel("granite", "granite-speech-5-nc", false);
    await loadEngineModel("qwen3", "qwen3-asr-0.6b", true);
    expect(calls).toEqual([
      { cmd: "switch_model", args: { modelId: "small.en", useGpu: true } },
      { cmd: "init_granite", args: { modelId: "granite-speech-5-nc", useGpu: false } },
      { cmd: "init_qwen3", args: { modelId: "qwen3-asr-0.6b", useGpu: true } },
    ]);
  });

  test("throws the backend's error code", async () => {
    responses.init_granite = { ok: false, data: null, error: { code: "model_missing", message: "not downloaded" } };
    const err = await loadEngineModel("granite", "x", true).catch((e) => e);
    expect(err).toBeInstanceOf(Error);
    expect(err).toMatchObject({ code: "model_missing", message: "not downloaded" });
    expect(`${err}`).toBe("EngineLoadError: not downloaded");
    responses.init_qwen3 = { ok: false, data: null, error: null };
    await expect(loadEngineModel("qwen3", "x", true)).rejects.toMatchObject({ code: "model_load_failed" });
  });
});

describe("ensureEngineModel", () => {
  test("does nothing when the model is already loaded", async () => {
    responses.get_qwen3_status = { loaded: true, model_id: "qwen3-asr-0.6b" };
    let loading = false;
    expect(await ensureEngineModel("qwen3", "qwen3-asr-0.6b", true, () => (loading = true))).toBe("already");
    expect(loading).toBe(false);
    expect(calls.map((c) => c.cmd)).toEqual(["get_qwen3_status"]);
  });

  test("loads when a different model or none is loaded", async () => {
    responses.get_current_model = "base.en";
    responses.switch_model = { ok: true, data: "ok", error: null };
    let loading = 0;
    expect(await ensureEngineModel("whisper", "small.en", false, () => loading++)).toBe("loaded");
    expect(loading).toBe(1);
    expect(calls.at(-1)).toEqual({ cmd: "switch_model", args: { modelId: "small.en", useGpu: false } });
  });

  test("propagates load failures", async () => {
    responses.get_granite_status = { loaded: false, model_id: null };
    responses.init_granite = { ok: false, data: null, error: { code: "engine_loading", message: "busy" } };
    await expect(ensureEngineModel("granite", "g", true)).rejects.toMatchObject({ code: "engine_loading" });
  });
});
