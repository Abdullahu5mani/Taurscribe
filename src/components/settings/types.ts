/** Shared types for the Settings modal and its sub-components. */

export interface DownloadableModel {
    id: string;
    name: string;
    type: 'Whisper' | 'Granite' | 'LLM' | 'CoreML' | 'Qwen3' | 'Speaker';
    size: string;
    description: string;
    downloaded: boolean;
    verified?: boolean;
    macosOnly?: boolean;
    /** Hide unless running on Windows (e.g. NVIDIA CUDA-only download). */
    windowsOnly?: boolean;
    /**
     * Full-precision Whisper model whose download bundles the CoreML ANE
     * encoder (ggml-{stem}-encoder.mlmodelc) on Apple Silicon. The UI shows
     * an "⚡ ANE Accelerated" badge for these when running on Apple Silicon.
     */
    aneCapable?: boolean;
    /** Shown with a "Beta" badge: works, but still being evaluated. */
    beta?: boolean;
}

export interface DownloadProgress {
    bytes: number;
    total: number;
    status: string;
    current_file?: number;
    total_files?: number;
    error?: string;
}

/** Speaker recognition model (voiceprints / Speaker Vault). Must match model_registry.rs. */
export const SPEAKER_MODEL_ID = 'speaker-campplus-en';

/** Speaker diarization model (who spoke when on the call). Must match model_registry.rs. */
export const DIARIZATION_MODEL_ID = 'diarization-nemotron3';

export const MODELS: DownloadableModel[] = [
    // --- Speaker recognition (required for the Speaker Vault) ---
    { id: SPEAKER_MODEL_ID, name: 'Speaker Recognition (CAM++)', type: 'Speaker', size: '28 MB', description: 'Recognises meeting participants by voice across meetings. Required for the Speaker Vault.', downloaded: false },

    { id: DIARIZATION_MODEL_ID, name: 'Speaker Separation (Nemotron 3)', type: 'Speaker', size: '199 MB', description: 'Tells the people on a call apart (up to 8). Much more accurate than the built-in voice grouping. OpenMDW licence.', downloaded: false },

    // --- Tiny ---
    // aneCapable = full-precision models whose download auto-bundles the ANE encoder on Apple Silicon.
    { id: 'whisper-tiny', name: 'Tiny (Multilingual)', type: 'Whisper', size: '75 MB', description: 'Fastest, lowest accuracy. 99+ languages.', downloaded: false, aneCapable: true },
    { id: 'whisper-tiny-q5_1', name: 'Tiny (Multi, Q5_1)', type: 'Whisper', size: '31 MB', description: 'Quantized Tiny. 99+ languages.', downloaded: false },
    { id: 'whisper-tiny-en', name: 'Tiny (English)', type: 'Whisper', size: '75 MB', description: 'Fastest model. English only.', downloaded: false, aneCapable: true },
    { id: 'whisper-tiny-en-q5_1', name: 'Tiny (English, Q5_1)', type: 'Whisper', size: '31 MB', description: 'Quantized, ultra-fast. English only.', downloaded: false },

    // --- Base ---
    { id: 'whisper-base', name: 'Base (Multilingual)', type: 'Whisper', size: '142 MB', description: 'Balanced entry model. 99+ languages.', downloaded: false, aneCapable: true },
    { id: 'whisper-base-en', name: 'Base (English)', type: 'Whisper', size: '142 MB', description: 'Standard balanced model. English only.', downloaded: false, aneCapable: true },
    { id: 'whisper-base-q5_1', name: 'Base (Multi, Q5_1)', type: 'Whisper', size: '57 MB', description: 'Quantized Base. 99+ languages.', downloaded: false },
    { id: 'whisper-base-en-q5_1', name: 'Base (English, Q5_1)', type: 'Whisper', size: '57 MB', description: 'Quantized Base. English only.', downloaded: false },

    // --- Small ---
    { id: 'whisper-small', name: 'Small (Multilingual)', type: 'Whisper', size: '466 MB', description: 'Good accuracy for general use. 99+ languages.', downloaded: false, aneCapable: true },
    { id: 'whisper-small-en', name: 'Small (English)', type: 'Whisper', size: '466 MB', description: 'Good accuracy model. English only.', downloaded: false, aneCapable: true },
    { id: 'whisper-small-q5_1', name: 'Small (Multi, Q5_1)', type: 'Whisper', size: '181 MB', description: 'Quantized Small. 99+ languages.', downloaded: false },
    { id: 'whisper-small-en-q5_1', name: 'Small (English, Q5_1)', type: 'Whisper', size: '181 MB', description: 'Quantized Small. English only.', downloaded: false },

    // --- Medium ---
    { id: 'whisper-medium', name: 'Medium (Multilingual)', type: 'Whisper', size: '1.5 GB', description: 'High accuracy, slower. 99+ languages.', downloaded: false, aneCapable: true },
    { id: 'whisper-medium-en', name: 'Medium (English)', type: 'Whisper', size: '1.5 GB', description: 'High accuracy. English only.', downloaded: false, aneCapable: true },
    { id: 'whisper-medium-q5_0', name: 'Medium (Multi, Q5_0)', type: 'Whisper', size: '514 MB', description: 'Quantized Medium. 99+ languages.', downloaded: false },
    { id: 'whisper-medium-en-q5_0', name: 'Medium (English, Q5_0)', type: 'Whisper', size: '514 MB', description: 'Quantized Medium. English only.', downloaded: false },

    // --- Large ---
    { id: 'whisper-large-v3', name: 'Large V3 (Multilingual)', type: 'Whisper', size: '2.9 GB', description: 'State of the art accuracy. 99+ languages.', downloaded: false, aneCapable: true },
    { id: 'whisper-large-v3-q5_0', name: 'Large V3 (Multi, Q5_0)', type: 'Whisper', size: '1.1 GB', description: 'Quantized Large V3. 99+ languages.', downloaded: false },
    { id: 'whisper-large-v3-turbo', name: 'Large V3 Turbo', type: 'Whisper', size: '1.5 GB', description: 'Optimized Large V3. 99+ languages.', downloaded: false, aneCapable: true },
    { id: 'whisper-large-v3-turbo-q5_0', name: 'Large V3 Turbo (Q5_0)', type: 'Whisper', size: '547 MB', description: 'Quantized Turbo. 99+ languages.', downloaded: false },

    // --- Unquantized GGUF models (same choices on every supported OS) ---
    { id: 'granite-speech-5-nc', name: 'Granite Speech 5 (470M, F16)', type: 'Granite', size: '948 MB', description: 'Fast English dictation. Non-commercial CC-BY-NC-SA-4.0 weights; runs through transcribe.cpp.', downloaded: false },
    { id: 'qwen3-asr-1.7b', name: 'Qwen3-ASR 1.7B (F16)', type: 'Qwen3', size: '4.1 GB', description: 'Full-size multilingual Qwen3-ASR. Unquantized F16 GGUF; runs through transcribe.cpp.', downloaded: false },
    { id: 'qwen3-asr-0.6b', name: 'Qwen3-ASR 0.6B (F16)', type: 'Qwen3', size: '1.6 GB', description: 'Smaller multilingual Qwen3-ASR architecture for lower-memory machines. Unquantized F16 GGUF.', downloaded: false },

    // --- LLM ---
    { id: 'flowscribe-qwen3.5-0.8b-v3', name: 'FlowScribe V3 (Qwen3.5 0.8B)', type: 'LLM', size: '1.52 GB', description: 'Cleans dictation into what you meant: fillers and corrections removed, numbers and emails written properly, your dictionary and the app you type into respected. Full precision (F16).', downloaded: false, beta: true },


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

/** settings.json keys shared with useInitialLoad and MeetingBanner. */
export const MEETING_KEYS = {
    sourceMode: 'audio_source_mode',
    detection: 'meeting_detection_enabled',
    autoRecord: 'auto_record_meetings',
    autoRecordDelay: 'meeting_autorecord_delay',
    showBanner: 'meeting_show_banner',
    matchThreshold: 'speaker_match_threshold',
    continueMinutes: 'meeting_continue_minutes',
} as const;

export const DEFAULT_AUTORECORD_DELAY = 3;
export const DEFAULT_MATCH_THRESHOLD = 0.6;
export const DEFAULT_CONTINUE_MINUTES = 10;
