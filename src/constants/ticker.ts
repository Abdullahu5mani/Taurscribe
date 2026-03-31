// Ticker phrases — kept short and low-chroma in CSS; fewer lines = calmer header.
// macOS filter in App.tsx still strips CUDA mentions from this list.

export type TickerHighlight = "accent" | "whisper" | "parakeet" | "cohere";

export interface TickerPhrase {
    parts: { text: string; highlight?: TickerHighlight }[];
}

export const TICKER_PHRASES: TickerPhrase[] = [
    { parts: [{ text: "100% " }, { text: "local", highlight: "accent" }, { text: " · nothing leaves your machine" }] },
    { parts: [{ text: "OpenAI " }, { text: "Whisper", highlight: "whisper" }, { text: " · NVIDIA " }, { text: "Parakeet", highlight: "parakeet" }, { text: " · " }, { text: "Cohere", highlight: "cohere" }] },
    { parts: [{ text: "Hit " }, { text: "REC", highlight: "accent" }, { text: " · speech to text in real time" }] },
    { parts: [{ text: "Ctrl+Win", highlight: "accent" }, { text: " from any app to record" }] },
    { parts: [{ text: "No cloud · no API keys · no subscriptions" }] },
    { parts: [{ text: "Offline-first", highlight: "accent" }, { text: " · optional grammar " }, { text: "LLM", highlight: "accent" }] },
    { parts: [{ text: "Pick your engine · batch " }, { text: "Whisper", highlight: "whisper" }, { text: " · streaming " }, { text: "Parakeet", highlight: "parakeet" }] },
    { parts: [{ text: "Download models once · run on your hardware" }] },
    { parts: [{ text: "Your audio never leaves this device" }] },
    { parts: [{ text: "Record · transcribe · copy · done" }] },
];
