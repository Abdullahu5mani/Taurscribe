// Ticker phrases defined outside the component so the array is never recreated on render.
// Extracted from App.tsx to reduce its top-of-file noise.

export type TickerHighlight = "accent" | "whisper" | "parakeet" | "granite";

export interface TickerPhrase {
    parts: { text: string; highlight?: TickerHighlight }[];
}

export const TICKER_PHRASES: TickerPhrase[] = [
    { parts: [{ text: "100% " }, { text: "local", highlight: "accent" }, { text: " · nothing leaves your machine" }] },
    { parts: [{ text: "OpenAI " }, { text: "Whisper", highlight: "whisper" }, { text: " & NVIDIA " }, { text: "Parakeet", highlight: "parakeet" }, { text: " · GPU-accelerated" }] },
    { parts: [{ text: "Hit " }, { text: "REC", highlight: "accent" }, { text: " · speech to text in real time" }] },
    { parts: [{ text: "No cloud", highlight: "accent" }, { text: " · no API keys · no subscriptions" }] },
    { parts: [{ text: "Switch between " }, { text: "Whisper", highlight: "whisper" }, { text: " and " }, { text: "Parakeet", highlight: "parakeet" }, { text: " anytime" }] },
    { parts: [{ text: "IBM " }, { text: "Granite Speech", highlight: "granite" }, { text: " · 1B · ONNX · runs anywhere" }] },
    { parts: [{ text: "Ctrl+Win", highlight: "accent" }, { text: " from anywhere to record" }] },
    { parts: [{ text: "Grammar correction · optional " }, { text: "LLM", highlight: "accent" }, { text: "" }] },
    { parts: [{ text: "Offline-first", highlight: "accent" }, { text: " · your data stays yours" }] },
    { parts: [{ text: "Pick your engine · " }, { text: "Whisper", highlight: "whisper" }, { text: " · " }, { text: "Parakeet", highlight: "parakeet" }, { text: " · " }, { text: "Granite Speech", highlight: "granite" }] },
    { parts: [{ text: "Studio-grade", highlight: "accent" }, { text: " · runs on your hardware" }] },
    { parts: [{ text: "Real-time transcription with " }, { text: "Whisper", highlight: "whisper" }, { text: " or " }, { text: "Parakeet", highlight: "parakeet" }] },
    { parts: [{ text: "Your audio never leaves this device" }] },
    { parts: [{ text: "CUDA · CPU · Metal · flexible backends" }] },
    { parts: [{ text: "Download models once · use forever" }] },
    { parts: [{ text: "Built for privacy · built for speed" }] },
    { parts: [{ text: "Three engines · " }, { text: "Whisper", highlight: "whisper" }, { text: " · " }, { text: "Parakeet", highlight: "parakeet" }, { text: " · " }, { text: "Granite Speech", highlight: "granite" }] },
    { parts: [{ text: "Press REC and speak · that's it" }] },
    { parts: [{ text: "No account", highlight: "accent" }, { text: " · no sign-up · no tracking" }] },
    { parts: [{ text: "Low latency · high accuracy" }] },
    { parts: [{ text: "Use " }, { text: "Whisper", highlight: "whisper" }, { text: " for batch · " }, { text: "Parakeet", highlight: "parakeet" }, { text: " for streaming" }] },
    { parts: [{ text: "Desktop-first · always ready" }] },
    { parts: [{ text: "Your words · your machine · your rules" }] },
    { parts: [{ text: "Multilingual " }, { text: "Whisper", highlight: "whisper" }, { text: " · real-time " }, { text: "Parakeet", highlight: "parakeet" }] },
    { parts: [{ text: "Transcribe meetings · notes · ideas" }] },
    { parts: [{ text: "One click to record", highlight: "accent" }, { text: " · one click to copy" }] },
    { parts: [{ text: "GPU-accelerated when you have it" }] },
    { parts: [{ text: "Open source models · open future" }] },
    { parts: [{ text: "IBM " }, { text: "Granite Speech", highlight: "granite" }, { text: " · no GPU required" }] },
    { parts: [{ text: "Privacy by design", highlight: "accent" }, { text: " · not as an afterthought" }] },
    { parts: [{ text: "Capture every word · edit later" }] },
    { parts: [{ text: "No internet? No problem." }] },
    { parts: [{ text: "Tiny to large · pick your " }, { text: "Whisper", highlight: "whisper" }, { text: " size" }] },
    { parts: [{ text: "Streaming with " }, { text: "Parakeet", highlight: "parakeet" }, { text: " · see text as you speak" }] },
    { parts: [{ text: "Hotkey ready", highlight: "accent" }, { text: " · Ctrl+Win from any app" }] },
    { parts: [{ text: "Local AI · no data in the cloud" }] },
    { parts: [{ text: "Built for creators · built for you" }] },
    { parts: [{ text: "Switch engines mid-workflow" }] },
    { parts: [{ text: "Grammar correction · optional" }] },
    { parts: [{ text: "Whisper", highlight: "whisper" }, { text: " for accuracy · " }, { text: "Parakeet", highlight: "parakeet" }, { text: " for speed · " }, { text: "Granite Speech", highlight: "granite" }, { text: " for reliability" }] },
    { parts: [{ text: "Your microphone · your transcript" }] },
    { parts: [{ text: "Download once · run anywhere" }] },
    { parts: [{ text: "No subscriptions", highlight: "accent" }, { text: " · pay with your hardware" }] },
    { parts: [{ text: "Transcription that respects you" }] },
    { parts: [{ text: "Fast " }, { text: "Whisper", highlight: "whisper" }, { text: " · faster " }, { text: "Parakeet", highlight: "parakeet" }] },
    { parts: [{ text: "Record · transcribe · copy · done" }] },
    { parts: [{ text: "OpenAI · NVIDIA · IBM · three giants · one app" }] },
    { parts: [{ text: "One app · three engines · zero compromise" }] },
];
