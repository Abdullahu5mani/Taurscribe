/**
 * Formats a model size in MB to a human-readable string.
 */
export function formatSize(sizeMb: number): string {
  if (sizeMb >= 1024) return `${(sizeMb / 1024).toFixed(1)} GB`;
  return `${Math.round(sizeMb)} MB`;
}

/**
 * Converts a raw model filename/display_name into a human-readable label.
 *
 * Examples:
 *   ggml-small.en.bin → "Small"
 *   ggml-medium-q8_0.bin → "Medium (Fast)"
 */
export function beautifyModelName(rawName: string): string {
  let name = rawName
    .replace("ggml-", "")
    .replace(".bin", "")
    .replace("distil-", "Distil ")
    .replace("medium.en", "Medium")
    .replace("small.en", "Small")
    .replace("tiny.en", "Tiny")
    .replace("base.en", "Base")
    .replace("-q8_0", " (Fast)")
    .replace("-q5_1", " (Balanced)")
    .replace("nemotron", "Nemotron")
    .replace("parakeet", "")
    .replace("ctc-", "CTC ")
    .replace("tdt-", "TDT ")
    .replace("streaming", "Streaming")
    .replace("-", " ")
    .replace("_", " ")
    .trim();
  return name.replace(/\b\w/g, l => l.toUpperCase());
}

/**
 * Converts a raw model ID into a human-readable display label.
 *
 * Examples:
 *   whisper-small-en-q5_1       → "Small EN"
 *   whisper-medium-en-q5_0      → "Medium EN"
 *   whisper-large-v3-turbo-q5_0 → "Large V3 Turbo"
 *   whisper-large-v3            → "Large V3"
 *   whisper-base-q5_1           → "Base"
 *   parakeet-tdt-0.6b-v2        → "TDT 0.6b V2"
 *   granite-speech-3b-a800m     → "3b A800m"
 */
export function formatModelDisplay(modelId: string | null | undefined): string | null {
    if (!modelId) return null;

    const m = modelId
        .replace(/^whisper-/, '')
        .replace(/^parakeet-/, '')
        .replace(/^granite-speech-/, '')
        .replace(/-q\d[\w]*$/i, '')  // strip quantization: -q5_1, -q5_0, -q4_k_m
        .replace(/-coreml$/, '');    // strip CoreML encoder suffix

    if (!m) return null;

    return m
        .split('-')
        .map(s => {
            const lower = s.toLowerCase();
            if (lower === 'en') return 'EN';
            if (lower === 'v3') return 'V3';
            if (lower === 'v2') return 'V2';
            if (lower === 'v1') return 'V1';
            if (lower === 'tdt') return 'TDT';
            return s.charAt(0).toUpperCase() + s.slice(1);
        })
        .join(' ');
}
