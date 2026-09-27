import { describe, expect, test } from "bun:test";
import { beautifyModelName, formatModelDisplay, formatSize } from "../../src/utils/modelDisplay";
import { getEngineForModelId } from "../../src/utils/engineUtils";
import { levelToPercent } from "../../src/utils/audioLevel";
import { formatRemaining, formatTimeoutLabel } from "../../src/hooks/useAutoUnload";

describe("beautifyModelName", () => {
  test("turns ggml filenames into short labels", () => {
    expect(beautifyModelName("ggml-small.en.bin")).toBe("Small");
    expect(beautifyModelName("ggml-medium-q8_0.bin")).toBe("Medium (Fast)");
    expect(beautifyModelName("ggml-tiny.en-q5_1.bin")).toBe("Tiny (Balanced)");
  });

  test("replaces every dash and underscore, not just the first", () => {
    expect(beautifyModelName("ggml-large-v3-turbo-q5_0.bin")).toBe("Large V3 Turbo Q5 0");
  });

  test("keeps backend display names readable", () => {
    expect(beautifyModelName("Large V3 Turbo Multilingual")).toBe("Large V3 Turbo Multilingual");
    expect(beautifyModelName("Granite Speech 5 (470M)")).toBe("Granite Speech 5 (470M)");
  });
});

describe("formatModelDisplay", () => {
  test.each([
    ["whisper-small-en-q5_1", "Small EN"],
    ["whisper-medium-en-q5_0", "Medium EN"],
    ["whisper-large-v3-turbo-q5_0", "Large V3 Turbo"],
    ["whisper-large-v3", "Large V3"],
    ["whisper-base-q5_1", "Base"],
    ["whisper-tiny-en-coreml", "Tiny EN"],
    ["granite-tdt-0.6b-v2", "TDT 0.6b V2"],
  ])("%s -> %s", (id, label) => {
    expect(formatModelDisplay(id)).toBe(label);
  });

  test("returns null for missing ids", () => {
    expect(formatModelDisplay(null)).toBeNull();
    expect(formatModelDisplay(undefined)).toBeNull();
    expect(formatModelDisplay("")).toBeNull();
    expect(formatModelDisplay("whisper-")).toBeNull();
  });
});

test("formatSize switches to GB at 1024 MB", () => {
  expect(formatSize(0)).toBe("0 MB");
  expect(formatSize(487.4)).toBe("487 MB");
  expect(formatSize(1024)).toBe("1.0 GB");
  expect(formatSize(1536)).toBe("1.5 GB");
});

test("getEngineForModelId maps id prefixes to engines", () => {
  expect(getEngineForModelId("whisper-base-en")).toBe("whisper");
  expect(getEngineForModelId("granite-speech-5-nc")).toBe("granite");
  expect(getEngineForModelId("qwen3-asr-0.6b")).toBe("qwen3");
  expect(getEngineForModelId("flowscribe-qwen3.5-0.8b-v3")).toBeNull();
  expect(getEngineForModelId("")).toBeNull();
});

describe("levelToPercent", () => {
  test("maps -60..0 dBFS onto 0..100", () => {
    expect(levelToPercent(1)).toBe(100);
    expect(levelToPercent(0.001)).toBe(0);
    expect(levelToPercent(0.01)).toBe(33);
    expect(levelToPercent(0.1)).toBe(67);
  });

  test("clamps out-of-range and invalid input", () => {
    expect(levelToPercent(0)).toBe(0);
    expect(levelToPercent(-0.5)).toBe(0);
    expect(levelToPercent(Number.NaN)).toBe(0);
    expect(levelToPercent(1e-9)).toBe(0);
    expect(levelToPercent(5)).toBe(100);
  });
});

describe("auto-unload labels", () => {
  test("formatTimeoutLabel names presets and rounds other values", () => {
    expect(formatTimeoutLabel(0)).toBe("Never");
    expect(formatTimeoutLabel(1)).toBe("Instant");
    expect(formatTimeoutLabel(1800)).toBe("30m");
    expect(formatTimeoutLabel(45)).toBe("45s");
    expect(formatTimeoutLabel(120)).toBe("2m");
    expect(formatTimeoutLabel(7200)).toBe("2h");
  });

  test("formatRemaining pads seconds after minutes", () => {
    expect(formatRemaining(-3)).toBe("0s");
    expect(formatRemaining(0)).toBe("0s");
    expect(formatRemaining(9)).toBe("9s");
    expect(formatRemaining(61)).toBe("1m 01s");
    expect(formatRemaining(600)).toBe("10m 00s");
  });
});
