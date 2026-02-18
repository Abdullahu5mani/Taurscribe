/** Shared types for the Settings modal and its sub-components. */

export interface DownloadableModel {
    id: string;
    name: string;
    type: 'Whisper' | 'Parakeet' | 'LLM' | 'Utility';
    size: string;
    description: string;
    downloaded: boolean;
    verified?: boolean;
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

    // --- LLM ---
    { id: 'qwen2.5-0.5b-safetensors', name: 'Qwen 2.5 0.5B (GPU)', type: 'LLM', size: '~1 GB', description: 'Safetensors model for CUDA/CPU. Best for grammar correction.', downloaded: false },
    { id: 'qwen2.5-0.5b-instruct', name: 'Qwen 2.5 0.5B (Instruct, GGUF)', type: 'LLM', size: '429 MB', description: 'Quantized Q4_K_M. Use if you prefer smaller size.', downloaded: false },
    { id: 'qwen2.5-0.5b-instruct-tokenizer', name: 'Qwen 2.5 Tokenizer Files', type: 'LLM', size: '11.5 MB', description: 'Required for GGUF Instruct model only.', downloaded: false },

    // --- Utility ---
    { id: 'symspell-en-82k', name: 'English Dictionary (SymSpell)', type: 'Utility', size: '1 MB', description: 'Fast spelling correction (82k words). English only.', downloaded: false },
];
