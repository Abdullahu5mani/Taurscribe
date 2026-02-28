/// Model file descriptor — a single file that belongs to a model.
pub struct ModelFile {
    pub filename: &'static str,    // Local filename (e.g. "ggml-tiny.bin")
    pub remote_path: &'static str, // Remote path relative to repo root
    pub sha1: &'static str,
}

/// Full configuration for a downloadable model.
pub struct ModelConfig {
    pub repo: &'static str,
    pub branch: &'static str,
    pub files: Vec<ModelFile>,
    pub subdirectory: Option<&'static str>, // Local subdirectory to store files in
}

// Hugging Face defaults
const DEFAULT_HF_REPO: &str = "ggerganov/whisper.cpp";
const DEFAULT_HF_BRANCH: &str = "main";

/// Build a single-file Whisper model config using the default HF repo.
fn single_file_whisper(filename: &'static str, sha1: &'static str) -> ModelConfig {
    ModelConfig {
        repo: DEFAULT_HF_REPO,
        branch: DEFAULT_HF_BRANCH,
        files: vec![ModelFile {
            filename,
            remote_path: filename,
            sha1,
        }],
        subdirectory: None,
    }
}

/// Look up the download configuration for a model by its ID.
/// Returns `None` if the model ID is not recognised.
pub fn get_model_config(model_id: &str) -> Option<ModelConfig> {
    match model_id {
        // ── Whisper Tiny ──────────────────────────────────────────────────────
        "whisper-tiny" => Some(single_file_whisper(
            "ggml-tiny.bin",
            "bd577a113a864445d4c299885e0cb97d4ba92b5f",
        )),
        "whisper-tiny-q5_1" => Some(single_file_whisper(
            "ggml-tiny-q5_1.bin",
            "2827a03e495b1ed3048ef28a6a4620537db4ee51",
        )),
        "whisper-tiny-q8_0" => Some(single_file_whisper(
            "ggml-tiny-q8_0.bin",
            "19e8118f6652a650569f5a949d962154e01571d9",
        )),
        "whisper-tiny-en" => Some(single_file_whisper(
            "ggml-tiny.en.bin",
            "c78c86eb1a8faa21b369bcd33207cc90d64ae9df",
        )),
        "whisper-tiny-en-q5_1" => Some(single_file_whisper(
            "ggml-tiny.en-q5_1.bin",
            "3fb92ec865cbbc769f08137f22470d6b66e071b6",
        )),
        "whisper-tiny-en-q8_0" => Some(single_file_whisper(
            "ggml-tiny.en-q8_0.bin",
            "802d6668e7d411123e672abe4cb6c18f12306abb",
        )),

        // ── Whisper Base ──────────────────────────────────────────────────────
        "whisper-base" => Some(single_file_whisper(
            "ggml-base.bin",
            "465707469ff3a37a2b9b8d8f89f2f99de7299dac",
        )),
        "whisper-base-q5_1" => Some(single_file_whisper(
            "ggml-base-q5_1.bin",
            "a3733eda680ef76256db5fc5dd9de8629e62c5e7",
        )),
        "whisper-base-q8_0" => Some(single_file_whisper(
            "ggml-base-q8_0.bin",
            "7bb89bb49ed6955013b166f1b6a6c04584a20fbe",
        )),
        "whisper-base-en" => Some(single_file_whisper(
            "ggml-base.en.bin",
            "137c40403d78fd54d454da0f9bd998f78703390c",
        )),
        "whisper-base-en-q5_1" => Some(single_file_whisper(
            "ggml-base.en-q5_1.bin",
            "d26d7ce5a1b6e57bea5d0431b9c20ae49423c94a",
        )),
        "whisper-base-en-q8_0" => Some(single_file_whisper(
            "ggml-base.en-q8_0.bin",
            "bb1574182e9b924452bf0cd1510ac034d323e948",
        )),

        // ── Whisper Small ─────────────────────────────────────────────────────
        "whisper-small" => Some(single_file_whisper(
            "ggml-small.bin",
            "55356645c2b361a969dfd0ef2c5a50d530afd8d5",
        )),
        "whisper-small-q5_1" => Some(single_file_whisper(
            "ggml-small-q5_1.bin",
            "6fe57ddcfdd1c6b07cdcc73aaf620810ce5fc771",
        )),
        "whisper-small-q8_0" => Some(single_file_whisper(
            "ggml-small-q8_0.bin",
            "bcad8a2083f4e53d648d586b7dbc0cd673d8afad",
        )),
        "whisper-small-en" => Some(single_file_whisper(
            "ggml-small.en.bin",
            "db8a495a91d927739e50b3fc1cc4c6b8f6c2d022",
        )),
        "whisper-small-en-q5_1" => Some(single_file_whisper(
            "ggml-small.en-q5_1.bin",
            "20f54878d608f94e4a8ee3ae56016571d47cba34",
        )),
        "whisper-small-en-q8_0" => Some(single_file_whisper(
            "ggml-small.en-q8_0.bin",
            "9d75ff4ccfa0a8217870d7405cf8cef0a5579852",
        )),
        "whisper-small-en-tdrz" => Some(single_file_whisper(
            "ggml-small.en-tdrz.bin",
            "b6c6e7e89af1a35c08e6de56b66ca6a02a2fdfa1",
        )),

        // ── Whisper Medium ────────────────────────────────────────────────────
        "whisper-medium" => Some(single_file_whisper(
            "ggml-medium.bin",
            "fd9727b6e1217c2f614f9b698455c4ffd82463b4",
        )),
        "whisper-medium-q5_0" => Some(single_file_whisper(
            "ggml-medium-q5_0.bin",
            "7718d4c1ec62ca96998f058114db98236937490e",
        )),
        "whisper-medium-q8_0" => Some(single_file_whisper(
            "ggml-medium-q8_0.bin",
            "e66645948aff4bebbec71b3485c576f3d63af5d6",
        )),
        "whisper-medium-en" => Some(single_file_whisper(
            "ggml-medium.en.bin",
            "8c30f0e44ce9560643ebd10bbe50cd20eafd3723",
        )),
        "whisper-medium-en-q5_0" => Some(single_file_whisper(
            "ggml-medium.en-q5_0.bin",
            "bb3b5281bddd61605d6fc76bc5b92d8f20284c3b",
        )),
        "whisper-medium-en-q8_0" => Some(single_file_whisper(
            "ggml-medium.en-q8_0.bin",
            "b1cf48c12c807e14881f634fb7b6c6ca867f6b38",
        )),

        // ── Whisper Large ─────────────────────────────────────────────────────
        "whisper-large-v1" => Some(single_file_whisper(
            "ggml-large-v1.bin",
            "b1caaf735c4cc1429223d5a74f0f4d0b9b59a299",
        )),
        "whisper-large-v2" => Some(single_file_whisper(
            "ggml-large-v2.bin",
            "0f4c8e34f21cf1a914c59d8b3ce882345ad349d6",
        )),
        "whisper-large-v2-q5_0" => Some(single_file_whisper(
            "ggml-large-v2-q5_0.bin",
            "00e39f2196344e901b3a2bd5814807a769bd1630",
        )),
        "whisper-large-v2-q8_0" => Some(single_file_whisper(
            "ggml-large-v2-q8_0.bin",
            "da97d6ca8f8ffbeeb5fd147f79010eeea194ba38",
        )),
        "whisper-large-v3" => Some(single_file_whisper(
            "ggml-large-v3.bin",
            "ad82bf6a9043ceed055076d0fd39f5f186ff8062",
        )),
        "whisper-large-v3-q5_0" => Some(single_file_whisper(
            "ggml-large-v3-q5_0.bin",
            "e6e2ed78495d403bef4b7cff42ef4aaadcfea8de",
        )),
        "whisper-large-v3-turbo" => Some(single_file_whisper(
            "ggml-large-v3-turbo.bin",
            "4af2b29d7ec73d781377bfd1758ca957a807e941",
        )),
        "whisper-large-v3-turbo-q5_0" => Some(single_file_whisper(
            "ggml-large-v3-turbo-q5_0.bin",
            "e050f7970618a659205450ad97eb95a18d69c9ee",
        )),
        "whisper-large-v3-turbo-q8_0" => Some(single_file_whisper(
            "ggml-large-v3-turbo-q8_0.bin",
            "01bf15bedffe9f39d65c1b6ff9b687ea91f59e0e",
        )),

        // ── Whisper CoreML Encoders (macOS Apple Silicon) ─────────────────────
        // These zip files extract to a .mlmodelc directory alongside the .bin.
        // whisper.cpp automatically uses CoreML when the directory is present.
        "whisper-tiny-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-tiny-encoder.mlmodelc",
                remote_path: "ggml-tiny-encoder.mlmodelc.zip",
                sha1: "",
            }],
            subdirectory: None,
        }),
        "whisper-tiny-en-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-tiny.en-encoder.mlmodelc",
                remote_path: "ggml-tiny.en-encoder.mlmodelc.zip",
                sha1: "",
            }],
            subdirectory: None,
        }),
        "whisper-base-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-base-encoder.mlmodelc",
                remote_path: "ggml-base-encoder.mlmodelc.zip",
                sha1: "",
            }],
            subdirectory: None,
        }),
        "whisper-base-en-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-base.en-encoder.mlmodelc",
                remote_path: "ggml-base.en-encoder.mlmodelc.zip",
                sha1: "",
            }],
            subdirectory: None,
        }),
        "whisper-small-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-small-encoder.mlmodelc",
                remote_path: "ggml-small-encoder.mlmodelc.zip",
                sha1: "",
            }],
            subdirectory: None,
        }),
        "whisper-small-en-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-small.en-encoder.mlmodelc",
                remote_path: "ggml-small.en-encoder.mlmodelc.zip",
                sha1: "",
            }],
            subdirectory: None,
        }),
        "whisper-medium-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-medium-encoder.mlmodelc",
                remote_path: "ggml-medium-encoder.mlmodelc.zip",
                sha1: "",
            }],
            subdirectory: None,
        }),
        "whisper-medium-en-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-medium.en-encoder.mlmodelc",
                remote_path: "ggml-medium.en-encoder.mlmodelc.zip",
                sha1: "",
            }],
            subdirectory: None,
        }),
        "whisper-large-v3-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-large-v3-encoder.mlmodelc",
                remote_path: "ggml-large-v3-encoder.mlmodelc.zip",
                sha1: "",
            }],
            subdirectory: None,
        }),
        "whisper-large-v3-turbo-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-large-v3-turbo-encoder.mlmodelc",
                remote_path: "ggml-large-v3-turbo-encoder.mlmodelc.zip",
                sha1: "",
            }],
            subdirectory: None,
        }),

        // ── Parakeet ──────────────────────────────────────────────────────────
        "parakeet-nemotron" => Some(ModelConfig {
            repo: "altunenes/parakeet-rs",
            branch: "main",
            files: vec![
                ModelFile {
                    filename: "decoder_joint.onnx",
                    remote_path: "nemotron-speech-streaming-en-0.6b/decoder_joint.onnx",
                    sha1: "",
                },
                ModelFile {
                    filename: "encoder.onnx",
                    remote_path: "nemotron-speech-streaming-en-0.6b/encoder.onnx",
                    sha1: "",
                },
                ModelFile {
                    filename: "encoder.onnx.data",
                    remote_path: "nemotron-speech-streaming-en-0.6b/encoder.onnx.data",
                    sha1: "",
                },
                ModelFile {
                    filename: "tokenizer.model",
                    remote_path: "nemotron-speech-streaming-en-0.6b/tokenizer.model",
                    sha1: "",
                },
            ],
            subdirectory: Some("parakeet-nemotron"),
        }),

        // ── Spell Check ───────────────────────────────────────────────────────
        "symspell-en-82k" => Some(ModelConfig {
            repo: "github:wolfgarbe/SymSpell",
            branch: "master",
            files: vec![ModelFile {
                filename: "frequency_dictionary_en_82_765.txt",
                remote_path: "frequency_dictionary_en_82_765.txt",
                sha1: "",
            }],
            subdirectory: None,
        }),

        // ── LLM ───────────────────────────────────────────────────────────────
        "qwen2.5-0.5b-instruct" => Some(ModelConfig {
            repo: "Qwen/Qwen2.5-0.5B-Instruct-GGUF",
            branch: "main",
            files: vec![ModelFile {
                filename: "qwen2.5-0.5b-instruct-q4_k_m.gguf",
                remote_path: "qwen2.5-0.5b-instruct-q4_k_m.gguf",
                sha1: "",
            }],
            subdirectory: Some("Qwen2.5-0.5B-Instruct"),
        }),
        "qwen2.5-0.5b-instruct-tokenizer" => Some(ModelConfig {
            repo: "Qwen/Qwen2.5-0.5B-Instruct",
            branch: "main",
            files: vec![
                ModelFile {
                    filename: "tokenizer.json",
                    remote_path: "tokenizer.json",
                    sha1: "",
                },
                ModelFile {
                    filename: "tokenizer_config.json",
                    remote_path: "tokenizer_config.json",
                    sha1: "",
                },
                ModelFile {
                    filename: "vocab.json",
                    remote_path: "vocab.json",
                    sha1: "",
                },
                ModelFile {
                    filename: "merges.txt",
                    remote_path: "merges.txt",
                    sha1: "",
                },
            ],
            subdirectory: Some("Qwen2.5-0.5B-Instruct"),
        }),
        "qwen2.5-0.5b-safetensors" => Some(ModelConfig {
            repo: "Qwen/Qwen2.5-0.5B",
            branch: "main",
            files: vec![
                ModelFile {
                    filename: "model.safetensors",
                    remote_path: "model.safetensors",
                    sha1: "",
                },
                ModelFile {
                    filename: "config.json",
                    remote_path: "config.json",
                    sha1: "",
                },
                ModelFile {
                    filename: "generation_config.json",
                    remote_path: "generation_config.json",
                    sha1: "",
                },
                ModelFile {
                    filename: "tokenizer.json",
                    remote_path: "tokenizer.json",
                    sha1: "",
                },
                ModelFile {
                    filename: "tokenizer_config.json",
                    remote_path: "tokenizer_config.json",
                    sha1: "",
                },
                ModelFile {
                    filename: "vocab.json",
                    remote_path: "vocab.json",
                    sha1: "",
                },
                ModelFile {
                    filename: "merges.txt",
                    remote_path: "merges.txt",
                    sha1: "",
                },
            ],
            subdirectory: Some("Qwen2.5-0.5B-GPU"),
        }),

        _ => None,
    }
}
