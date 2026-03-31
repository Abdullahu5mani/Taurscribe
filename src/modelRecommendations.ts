export interface SystemInfo {
  cpu_name: string;
  cpu_cores: number;
  ram_total_gb: number;
  gpu_name: string;
  cuda_available: boolean;
  vram_gb: number | null;
  backend_hint: string;
}

export type WhisperTier = "Tiny" | "Base" | "Small" | "Medium" | "Large";
export type RecommendationEngine = "whisper" | "parakeet" | "cohere";
export type OnboardingUseCase = "quick_notes" | "coding" | "meetings" | "multilingual";

export interface UseCaseOption {
  id: OnboardingUseCase;
  label: string;
  shortLabel: string;
  description: string;
  audience: string;
}

export interface ModelRecommendation {
  useCase: OnboardingUseCase;
  useCaseLabel: string;
  title: string;
  summary: string;
  primaryEngine: RecommendationEngine;
  primaryEngineLabel: string;
  primaryModelId: string;
  primaryLabel: string;
  primaryReasoning: string[];
  backupEngine: RecommendationEngine | null;
  backupEngineLabel: string | null;
  backupModelId: string | null;
  backupLabel: string | null;
  hardwareLine: string;
  whisperTier: WhisperTier | null;
}

export const ONBOARDING_USE_CASES: UseCaseOption[] = [
  {
    id: "quick_notes",
    label: "Quick Dictation",
    shortLabel: "Dictation",
    description: "Fast notes, messages, and short bursts where latency matters most.",
    audience: "Low friction",
  },
  {
    id: "coding",
    label: "Coding",
    shortLabel: "Coding",
    description: "English-heavy dictation where proper terms, casing, and accuracy matter.",
    audience: "Technical",
  },
  {
    id: "meetings",
    label: "Meetings",
    shortLabel: "Meetings",
    description: "Longer captures where recall and overall accuracy matter more than speed.",
    audience: "Long form",
  },
  {
    id: "multilingual",
    label: "Multilingual",
    shortLabel: "Languages",
    description: "You switch languages often and need the safer multilingual Whisper path.",
    audience: "Flexible",
  },
];

const ENGINE_LABELS: Record<RecommendationEngine, string> = {
  whisper: "Whisper",
  parakeet: "Parakeet",
  cohere: "Cohere Speech",
};

function hasDiscreteGpu(sysInfo: SystemInfo | null): boolean {
  if (!sysInfo) return false;
  return Boolean(sysInfo.gpu_name && sysInfo.gpu_name !== "Unknown");
}

function hardwareProfile(sysInfo: SystemInfo | null, isAppleSilicon: boolean) {
  const ram = sysInfo?.ram_total_gb ?? 0;
  const vram = sysInfo?.vram_gb ?? 0;
  const gpu = hasDiscreteGpu(sysInfo);
  const accelerated = Boolean(sysInfo?.cuda_available || isAppleSilicon || (gpu && ram >= 16));
  const highHeadroom = ram >= 24 || vram >= 10 || (isAppleSilicon && ram >= 16);
  const mediumHeadroom = ram >= 16 || vram >= 6 || isAppleSilicon;
  const entryHeadroom = ram >= 8;

  return {
    ram,
    accelerated,
    highHeadroom,
    mediumHeadroom,
    entryHeadroom,
    veryLowMemory: ram > 0 && ram < 8,
  };
}

function whisperModelId(
  tier: WhisperTier,
  multilingual: boolean,
  isAppleSilicon: boolean,
): string {
  const table: Record<WhisperTier, { english: string; multilingual: string }> = isAppleSilicon
    ? {
        Tiny: { english: "whisper-tiny-en", multilingual: "whisper-tiny" },
        Base: { english: "whisper-base-en", multilingual: "whisper-base" },
        Small: { english: "whisper-small-en", multilingual: "whisper-small" },
        Medium: { english: "whisper-medium-en", multilingual: "whisper-medium" },
        Large: { english: "whisper-large-v3-turbo", multilingual: "whisper-large-v3-turbo" },
      }
    : {
        Tiny: { english: "whisper-tiny-en-q5_1", multilingual: "whisper-tiny-q5_1" },
        Base: { english: "whisper-base-en-q5_1", multilingual: "whisper-base-q5_1" },
        Small: { english: "whisper-small-en-q5_1", multilingual: "whisper-small-q5_1" },
        Medium: { english: "whisper-medium-en-q5_0", multilingual: "whisper-medium-q5_0" },
        Large: { english: "whisper-large-v3-turbo-q5_0", multilingual: "whisper-large-v3-turbo-q5_0" },
      };

  return multilingual ? table[tier].multilingual : table[tier].english;
}

function whisperLabel(tier: WhisperTier, multilingual: boolean): string {
  return multilingual ? `Whisper ${tier} (Multilingual)` : `Whisper ${tier} (English)`;
}

function modelTierForUseCase(
  useCase: OnboardingUseCase,
  profile: ReturnType<typeof hardwareProfile>,
): WhisperTier {
  if (useCase === "quick_notes") {
    if (profile.mediumHeadroom) return "Small";
    if (profile.entryHeadroom) return "Base";
    return "Tiny";
  }
  if (useCase === "coding") {
    if (profile.highHeadroom) return "Medium";
    if (profile.mediumHeadroom) return "Small";
    if (profile.entryHeadroom) return "Base";
    return "Tiny";
  }
  if (useCase === "meetings") {
    if (profile.highHeadroom) return "Medium";
    if (profile.mediumHeadroom) return "Small";
    return "Base";
  }
  if (profile.highHeadroom) return "Medium";
  if (profile.mediumHeadroom) return "Small";
  return "Base";
}

function hardwareLine(sysInfo: SystemInfo | null, profile: ReturnType<typeof hardwareProfile>): string {
  if (!sysInfo) {
    return "Recommendation based on the default balanced profile.";
  }
  if (profile.accelerated && profile.highHeadroom) {
    return `Detected ${sysInfo.backend_hint} acceleration with ${sysInfo.ram_total_gb.toFixed(0)} GB RAM, so this machine can handle larger models comfortably.`;
  }
  if (profile.accelerated) {
    return `Detected ${sysInfo.backend_hint} acceleration, so low-latency and mid-size models should feel responsive on this machine.`;
  }
  if (profile.veryLowMemory) {
    return `Detected ${sysInfo.ram_total_gb.toFixed(0)} GB RAM with no acceleration, so smaller models are the safer starting point.`;
  }
  return `No strong accelerator was detected, so this recommendation prioritizes stable CPU-friendly models first.`;
}

export function getEngineLabel(engine: RecommendationEngine): string {
  return ENGINE_LABELS[engine];
}

export function getWhisperTierFromModelId(modelId: string | null | undefined): WhisperTier | null {
  if (!modelId) return null;
  if (modelId.includes("tiny")) return "Tiny";
  if (modelId.includes("base")) return "Base";
  if (modelId.includes("small")) return "Small";
  if (modelId.includes("medium")) return "Medium";
  if (modelId.includes("large")) return "Large";
  return null;
}

export function computeModelRecommendation({
  sysInfo,
  isAppleSilicon,
  useCase,
}: {
  sysInfo: SystemInfo | null;
  isAppleSilicon: boolean;
  useCase: OnboardingUseCase;
}): ModelRecommendation {
  const profile = hardwareProfile(sysInfo, isAppleSilicon);
  const tier = modelTierForUseCase(useCase, profile);
  const useCaseLabel = ONBOARDING_USE_CASES.find((item) => item.id === useCase)?.label ?? "Quick Dictation";
  const baseHardwareLine = hardwareLine(sysInfo, profile);

  if (useCase === "quick_notes" && profile.accelerated && profile.entryHeadroom) {
    const backupTier = profile.mediumHeadroom ? "Small" : "Base";
    return {
      useCase,
      useCaseLabel,
      title: "Fast live dictation first",
      summary: "Parakeet gives you the fastest English-only live text, with Whisper as the safer fallback when you want a second opinion.",
      primaryEngine: "parakeet",
      primaryEngineLabel: ENGINE_LABELS.parakeet,
      primaryModelId: "parakeet-nemotron",
      primaryLabel: "Parakeet Nemotron Streaming",
      primaryReasoning: [
        "Best fit for short English dictation where sub-second feedback matters.",
        "Your hardware profile can support the larger streaming model comfortably.",
        "You can still switch to Whisper later for a slower but more conservative pass.",
      ],
      backupEngine: "whisper",
      backupEngineLabel: ENGINE_LABELS.whisper,
      backupModelId: whisperModelId(backupTier, false, isAppleSilicon),
      backupLabel: whisperLabel(backupTier, false),
      hardwareLine: baseHardwareLine,
      whisperTier: backupTier,
    };
  }

  if (useCase === "coding") {
    return {
      useCase,
      useCaseLabel,
      title: "Accuracy-first technical dictation",
      summary: "Whisper stays the safest starting point for code-adjacent vocabulary, acronyms, and technical phrasing.",
      primaryEngine: "whisper",
      primaryEngineLabel: ENGINE_LABELS.whisper,
      primaryModelId: whisperModelId(tier, false, isAppleSilicon),
      primaryLabel: whisperLabel(tier, false),
      primaryReasoning: [
        "English-only Whisper variants are the cleanest starting point for coding and docs.",
        profile.highHeadroom
          ? "This machine can afford a heavier model for fewer corrections."
          : "This tier keeps correction load low without making the app feel heavy.",
        "Parakeet is still available later if you decide raw speed matters more than precision.",
      ],
      backupEngine: profile.accelerated ? "parakeet" : null,
      backupEngineLabel: profile.accelerated ? ENGINE_LABELS.parakeet : null,
      backupModelId: profile.accelerated ? "parakeet-nemotron" : null,
      backupLabel: profile.accelerated ? "Parakeet Nemotron Streaming" : null,
      hardwareLine: baseHardwareLine,
      whisperTier: tier,
    };
  }

  if (useCase === "meetings") {
    const backupTier = profile.entryHeadroom ? "Base" : "Tiny";
    return {
      useCase,
      useCaseLabel,
      title: "Balanced for longer captures",
      summary: "Whisper gives you the best starting point for long-form transcription where overall accuracy matters more than the absolute lowest latency.",
      primaryEngine: "whisper",
      primaryEngineLabel: ENGINE_LABELS.whisper,
      primaryModelId: whisperModelId(tier, true, isAppleSilicon),
      primaryLabel: whisperLabel(tier, true),
      primaryReasoning: [
        "Multilingual Whisper is the safest choice when meetings can drift across names, accents, or languages.",
        profile.highHeadroom
          ? "A larger tier makes sense here because longer sessions benefit from stronger recall."
          : "This tier avoids overloading the machine during extended sessions.",
        "If you only need live English captions, Parakeet remains a fast optional switch later.",
      ],
      backupEngine: "whisper",
      backupEngineLabel: ENGINE_LABELS.whisper,
      backupModelId: whisperModelId(backupTier, true, isAppleSilicon),
      backupLabel: whisperLabel(backupTier, true),
      hardwareLine: baseHardwareLine,
      whisperTier: tier,
    };
  }

  const multilingualTier = tier === "Tiny" ? "Base" : tier;
  const fallbackTier = multilingualTier === "Medium" ? "Base" : multilingualTier;
  return {
    useCase,
    useCaseLabel,
    title: "Safer multilingual baseline",
    summary: "A multilingual Whisper model keeps the app flexible when you switch languages or work with mixed-language audio.",
    primaryEngine: "whisper",
    primaryEngineLabel: ENGINE_LABELS.whisper,
    primaryModelId: whisperModelId(multilingualTier, true, isAppleSilicon),
    primaryLabel: whisperLabel(multilingualTier, true),
    primaryReasoning: [
      "Multilingual Whisper is the most reliable path when you do not want to guess language upfront.",
      profile.mediumHeadroom
        ? "This tier keeps a good balance between language coverage and responsiveness."
        : "This smaller tier keeps the first-run download and CPU cost under control.",
      "Parakeet is intentionally not the primary pick here because it is English-only.",
    ],
    backupEngine: "whisper",
    backupEngineLabel: ENGINE_LABELS.whisper,
    backupModelId: whisperModelId(fallbackTier, true, isAppleSilicon),
    backupLabel: whisperLabel(fallbackTier, true),
    hardwareLine: baseHardwareLine,
    whisperTier: multilingualTier,
  };
}
