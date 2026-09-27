import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  computeModelRecommendation,
  getWhisperTierFromModelId,
  ONBOARDING_USE_CASES,
  type SystemInfo,
} from "../../src/modelRecommendations";

const root = join(import.meta.dir, "..", "..");
const registry = readFileSync(join(root, "src-tauri/src/commands/model_registry.rs"), "utf8");
const registryIds = new Set([...registry.matchAll(/^\s*"([a-z0-9._-]+)" => Some\(/gm)].map((m) => m[1]));

function sys(ram: number, extra: Partial<SystemInfo> = {}): SystemInfo {
  return {
    cpu_name: "CPU",
    cpu_cores: 8,
    ram_total_gb: ram,
    gpu_name: "Unknown",
    cuda_available: false,
    vram_gb: null,
    backend_hint: "CPU",
    ...extra,
  };
}

const machines: Array<[string, SystemInfo | null, boolean]> = [
  ["unknown", null, false],
  ["4 GB laptop", sys(4), false],
  ["8 GB CPU", sys(8), false],
  ["16 GB CPU", sys(16), false],
  ["32 GB CUDA", sys(32, { gpu_name: "RTX 4090", cuda_available: true, vram_gb: 24, backend_hint: "CUDA" }), false],
  ["8 GB Apple Silicon", sys(8, { backend_hint: "Metal" }), true],
  ["24 GB Apple Silicon", sys(24, { backend_hint: "Metal" }), true],
];

test("the registry parse found the model ids", () => {
  expect(registryIds.has("whisper-base-en")).toBe(true);
  expect(registryIds.size).toBeGreaterThan(20);
});

describe("computeModelRecommendation", () => {
  for (const [name, info, apple] of machines) {
    for (const { id: useCase } of ONBOARDING_USE_CASES) {
      test(`${useCase} on ${name} recommends downloadable models`, () => {
        const rec = computeModelRecommendation({ sysInfo: info, isAppleSilicon: apple, useCase });
        expect(rec.useCase).toBe(useCase);
        expect(registryIds.has(rec.primaryModelId)).toBe(true);
        if (rec.backupModelId !== null) {
          expect(registryIds.has(rec.backupModelId)).toBe(true);
          expect(rec.backupEngine).not.toBeNull();
        }
        if (rec.primaryEngine === "whisper") {
          expect(getWhisperTierFromModelId(rec.primaryModelId)).toBe(rec.whisperTier);
        }
        // Apple Silicon uses full-precision weights (CoreML encoder); others use quantized ones.
        const ids = [rec.primaryModelId, rec.backupModelId].filter((id): id is string => !!id && id.startsWith("whisper"));
        for (const id of ids) expect(/-q\d/.test(id)).toBe(!apple);
      });
    }
  }

  test("multilingual never recommends English-only Whisper", () => {
    for (const [, info, apple] of machines) {
      const rec = computeModelRecommendation({ sysInfo: info, isAppleSilicon: apple, useCase: "multilingual" });
      expect(rec.primaryEngine).toBe("whisper");
      expect(rec.primaryModelId).not.toContain("-en");
      expect(rec.backupModelId ?? "").not.toContain("-en");
    }
  });

  test("more memory never picks a smaller coding model", () => {
    const order = ["Tiny", "Base", "Small", "Medium", "Large"];
    let prev = -1;
    for (const ram of [4, 8, 16, 32]) {
      const rec = computeModelRecommendation({ sysInfo: sys(ram), isAppleSilicon: false, useCase: "coding" });
      const tier = order.indexOf(rec.whisperTier ?? "");
      expect(tier).toBeGreaterThanOrEqual(prev);
      prev = tier;
    }
  });

  test("fast dictation prefers Granite only with acceleration", () => {
    const cpu = computeModelRecommendation({ sysInfo: sys(8), isAppleSilicon: false, useCase: "quick_notes" });
    expect(cpu.primaryEngine).toBe("whisper");
    const mac = computeModelRecommendation({ sysInfo: sys(16), isAppleSilicon: true, useCase: "quick_notes" });
    expect(mac.primaryEngine).toBe("granite");
    expect(mac.backupEngine).toBe("whisper");
  });
});

test("getWhisperTierFromModelId", () => {
  expect(getWhisperTierFromModelId("whisper-large-v3-turbo-q5_0")).toBe("Large");
  expect(getWhisperTierFromModelId("whisper-small-en")).toBe("Small");
  expect(getWhisperTierFromModelId("granite-speech-5-nc")).toBeNull();
  expect(getWhisperTierFromModelId(null)).toBeNull();
});

test("every Whisper id the Models tab offers is in the download registry", () => {
  const tab = readFileSync(join(root, "src/components/settings/ModelsTab.tsx"), "utf8");
  const ids = [...new Set([...tab.matchAll(/'(whisper-[a-z0-9._-]+)'/g)].map((m) => m[1]))];
  expect(ids.length).toBeGreaterThan(10);
  expect(ids.filter((id) => !registryIds.has(id))).toEqual([]);
});
