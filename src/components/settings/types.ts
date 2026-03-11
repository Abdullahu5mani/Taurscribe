/** Shared types for the Settings modal and its sub-components. */

export interface DownloadableModel {
    id: string;
    name: string;
    type: 'Whisper' | 'Parakeet' | 'LLM' | 'CoreML' | 'GraniteSpeech';
    size: string;
    description: string;
    downloaded: boolean;
    verified?: boolean;
    macosOnly?: boolean;
}

export interface DownloadProgress {
    bytes: number;
    total: number;
    status: string;
    current_file?: number;
    total_files?: number;
}

export const MODELS: DownloadableModel[] = [
    // --- Tiny ---
    { id: 'whisper-tiny', name: 'Tiny (Multilingual)', type: 'Whisper', size: '75 MB', description: 'Fastest, lowest accuracy. 99+ languages.', downloaded: false },
    { id: 'whisper-tiny-q5_1', name: 'Tiny (Multi, Q5_1)', type: 'Whisper', size: '31 MB', description: 'Quantized Tiny. 99+ languages.', downloaded: false },
    { id: 'whisper-tiny-en', name: 'Tiny (English)', type: 'Whisper', size: '75 MB', description: 'Fastest model. English only.', downloaded: false },
    { id: 'whisper-tiny-en-q5_1', name: 'Tiny (English, Q5_1)', type: 'Whisper', size: '31 MB', description: 'Quantized, ultra-fast. English only.', downloaded: false },

    // --- Base ---
    { id: 'whisper-base', name: 'Base (Multilingual)', type: 'Whisper', size: '142 MB', description: 'Balanced entry model. 99+ languages.', downloaded: false },
    { id: 'whisper-base-en', name: 'Base (English)', type: 'Whisper', size: '142 MB', description: 'Standard balanced model. English only.', downloaded: false },
    { id: 'whisper-base-q5_1', name: 'Base (Multi, Q5_1)', type: 'Whisper', size: '57 MB', description: 'Quantized Base. 99+ languages.', downloaded: false },
    { id: 'whisper-base-en-q5_1', name: 'Base (English, Q5_1)', type: 'Whisper', size: '57 MB', description: 'Quantized Base. English only.', downloaded: false },

    // --- Small ---
    { id: 'whisper-small', name: 'Small (Multilingual)', type: 'Whisper', size: '466 MB', description: 'Good accuracy for general use. 99+ languages.', downloaded: false },
    { id: 'whisper-small-en', name: 'Small (English)', type: 'Whisper', size: '466 MB', description: 'Good accuracy model. English only.', downloaded: false },
    { id: 'whisper-small-q5_1', name: 'Small (Multi, Q5_1)', type: 'Whisper', size: '181 MB', description: 'Quantized Small. 99+ languages.', downloaded: false },
    { id: 'whisper-small-en-q5_1', name: 'Small (English, Q5_1)', type: 'Whisper', size: '181 MB', description: 'Quantized Small. English only.', downloaded: false },

    // --- Medium ---
    { id: 'whisper-medium', name: 'Medium (Multilingual)', type: 'Whisper', size: '1.5 GB', description: 'High accuracy, slower. 99+ languages.', downloaded: false },
    { id: 'whisper-medium-en', name: 'Medium (English)', type: 'Whisper', size: '1.5 GB', description: 'High accuracy. English only.', downloaded: false },
    { id: 'whisper-medium-q5_0', name: 'Medium (Multi, Q5_0)', type: 'Whisper', size: '514 MB', description: 'Quantized Medium. 99+ languages.', downloaded: false },
    { id: 'whisper-medium-en-q5_0', name: 'Medium (English, Q5_0)', type: 'Whisper', size: '514 MB', description: 'Quantized Medium. English only.', downloaded: false },

    // --- Large ---
    { id: 'whisper-large-v3', name: 'Large V3 (Multilingual)', type: 'Whisper', size: '2.9 GB', description: 'State of the art accuracy. 99+ languages.', downloaded: false },
    { id: 'whisper-large-v3-q5_0', name: 'Large V3 (Multi, Q5_0)', type: 'Whisper', size: '1.1 GB', description: 'Quantized Large V3. 99+ languages.', downloaded: false },
    { id: 'whisper-large-v3-turbo', name: 'Large V3 Turbo', type: 'Whisper', size: '1.5 GB', description: 'Optimized Large V3. 99+ languages.', downloaded: false },
    { id: 'whisper-large-v3-turbo-q5_0', name: 'Large V3 Turbo (Q5_0)', type: 'Whisper', size: '547 MB', description: 'Quantized Turbo. 99+ languages.', downloaded: false },

    // --- Parakeet ---
    { id: 'parakeet-nemotron', name: 'Nemotron Streaming', type: 'Parakeet', size: '1.2 GB', description: 'Ultra-low latency streaming. English only.', downloaded: true },

    // --- Granite Speech ---
    { id: 'granite-speech-1b-cpu', name: 'Granite 4.0 1B Speech', type: 'GraniteSpeech', size: '~1.8 GB', description: 'IBM Granite 4.0 1B · ONNX · English · runs on any hardware.', downloaded: false },

    // --- LLM ---
    { id: 'flowscribe-qwen2.5-0.5b', name: 'FlowScribe Qwen 2.5 0.5B', type: 'LLM', size: '398 MB', description: 'Fine-tuned Q4_K_M GGUF for speech-to-text grammar correction.', downloaded: false },


    // --- CoreML Encoders (macOS Apple Silicon only) ---
    { id: 'whisper-tiny-coreml', name: 'Tiny CoreML Encoder', type: 'CoreML', size: '15 MB', description: 'Apple Neural Engine encoder for Tiny (multilingual). Pair with ggml-tiny.bin.', downloaded: false, macosOnly: true },
    { id: 'whisper-tiny-en-coreml', name: 'Tiny (English) CoreML Encoder', type: 'CoreML', size: '15 MB', description: 'Apple Neural Engine encoder for Tiny (English). Pair with ggml-tiny.en.bin.', downloaded: false, macosOnly: true },
    { id: 'whisper-base-coreml', name: 'Base CoreML Encoder', type: 'CoreML', size: '38 MB', description: 'Apple Neural Engine encoder for Base (multilingual). Pair with ggml-base.bin.', downloaded: false, macosOnly: true },
    { id: 'whisper-base-en-coreml', name: 'Base (English) CoreML Encoder', type: 'CoreML', size: '38 MB', description: 'Apple Neural Engine encoder for Base (English). Pair with ggml-base.en.bin.', downloaded: false, macosOnly: true },
    { id: 'whisper-small-coreml', name: 'Small CoreML Encoder', type: 'CoreML', size: '163 MB', description: 'Apple Neural Engine encoder for Small (multilingual). Pair with ggml-small.bin.', downloaded: false, macosOnly: true },
    { id: 'whisper-small-en-coreml', name: 'Small (English) CoreML Encoder', type: 'CoreML', size: '163 MB', description: 'Apple Neural Engine encoder for Small (English). Pair with ggml-small.en.bin.', downloaded: false, macosOnly: true },
    { id: 'whisper-medium-coreml', name: 'Medium CoreML Encoder', type: 'CoreML', size: '568 MB', description: 'Apple Neural Engine encoder for Medium (multilingual). Pair with ggml-medium.bin.', downloaded: false, macosOnly: true },
    { id: 'whisper-medium-en-coreml', name: 'Medium (English) CoreML Encoder', type: 'CoreML', size: '567 MB', description: 'Apple Neural Engine encoder for Medium (English). Pair with ggml-medium.en.bin.', downloaded: false, macosOnly: true },
    { id: 'whisper-large-v3-coreml', name: 'Large V3 CoreML Encoder', type: 'CoreML', size: '1.18 GB', description: 'Apple Neural Engine encoder for Large V3. Pair with ggml-large-v3.bin.', downloaded: false, macosOnly: true },
    { id: 'whisper-large-v3-turbo-coreml', name: 'Large V3 Turbo CoreML Encoder', type: 'CoreML', size: '1.17 GB', description: 'Apple Neural Engine encoder for Large V3 Turbo. Pair with ggml-large-v3-turbo.bin.', downloaded: false, macosOnly: true },
];
