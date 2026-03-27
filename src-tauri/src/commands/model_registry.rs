/// Model file descriptor — a single file that belongs to a model.
pub struct ModelFile {
    pub filename: &'static str,    // Local filename (e.g. "ggml-tiny.bin")
    pub remote_path: &'static str, // Remote path relative to repo root
    /// SHA-256 of the raw file bytes (matches HuggingFace LFS `lfs.oid`).
    /// Leave empty ("") to skip verification for this file.
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
fn single_file_whisper(filename: &'static str, sha256: &'static str) -> ModelConfig {
    ModelConfig {
        repo: DEFAULT_HF_REPO,
        branch: DEFAULT_HF_BRANCH,
        files: vec![ModelFile {
            filename,
            remote_path: filename,
            sha1: sha256, // field kept as sha1 for structural compat; now holds SHA-256
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
            "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
        )),
        "whisper-tiny-q5_1" => Some(single_file_whisper(
            "ggml-tiny-q5_1.bin",
            "818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7",
        )),
        "whisper-tiny-q8_0" => Some(single_file_whisper(
            "ggml-tiny-q8_0.bin",
            "c2085835d3f50733e2ff6e4b41ae8a2b8d8110461e18821b09a15c40c42d1cca",
        )),
        "whisper-tiny-en" => Some(single_file_whisper(
            "ggml-tiny.en.bin",
            "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
        )),
        "whisper-tiny-en-q5_1" => Some(single_file_whisper(
            "ggml-tiny.en-q5_1.bin",
            "c77c5766f1cef09b6b7d47f21b546cbddd4157886b3b5d6d4f709e91e66c7c2b",
        )),
        "whisper-tiny-en-q8_0" => Some(single_file_whisper(
            "ggml-tiny.en-q8_0.bin",
            "5bc2b3860aa151a4c6e7bb095e1fcce7cf12c7b020ca08dcec0c6d018bb7dd94",
        )),

        // ── Whisper Base ──────────────────────────────────────────────────────
        "whisper-base" => Some(single_file_whisper(
            "ggml-base.bin",
            "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        )),
        "whisper-base-q5_1" => Some(single_file_whisper(
            "ggml-base-q5_1.bin",
            "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
        )),
        "whisper-base-q8_0" => Some(single_file_whisper(
            "ggml-base-q8_0.bin",
            "c577b9a86e7e048a0b7eada054f4dd79a56bbfa911fbdacf900ac5b567cbb7d9",
        )),
        "whisper-base-en" => Some(single_file_whisper(
            "ggml-base.en.bin",
            "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
        )),
        "whisper-base-en-q5_1" => Some(single_file_whisper(
            "ggml-base.en-q5_1.bin",
            "4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f",
        )),
        "whisper-base-en-q8_0" => Some(single_file_whisper(
            "ggml-base.en-q8_0.bin",
            "a4d4a0768075e13cfd7e19df3ae2dbc4a68d37d36a7dad45e8410c9a34f8c87e",
        )),

        // ── Whisper Small ─────────────────────────────────────────────────────
        "whisper-small" => Some(single_file_whisper(
            "ggml-small.bin",
            "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        )),
        "whisper-small-q5_1" => Some(single_file_whisper(
            "ggml-small-q5_1.bin",
            "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
        )),
        "whisper-small-q8_0" => Some(single_file_whisper(
            "ggml-small-q8_0.bin",
            "49c8fb02b65e6049d5fa6c04f81f53b867b5ec9540406812c643f177317f779f",
        )),
        "whisper-small-en" => Some(single_file_whisper(
            "ggml-small.en.bin",
            "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
        )),
        "whisper-small-en-q5_1" => Some(single_file_whisper(
            "ggml-small.en-q5_1.bin",
            "bfdff4894dcb76bbf647d56263ea2a96645423f1669176f4844a1bf8e478ad30",
        )),
        "whisper-small-en-q8_0" => Some(single_file_whisper(
            "ggml-small.en-q8_0.bin",
            "67a179f608ea6114bd3fdb9060e762b588a3fb3bd00c4387971be4d177958067",
        )),

        // ── Whisper Medium ────────────────────────────────────────────────────
        "whisper-medium" => Some(single_file_whisper(
            "ggml-medium.bin",
            "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
        )),
        "whisper-medium-q5_0" => Some(single_file_whisper(
            "ggml-medium-q5_0.bin",
            "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f",
        )),
        "whisper-medium-q8_0" => Some(single_file_whisper(
            "ggml-medium-q8_0.bin",
            "42a1ffcbe4167d224232443396968db4d02d4e8e87e213d3ee2e03095dea6502",
        )),
        "whisper-medium-en" => Some(single_file_whisper(
            "ggml-medium.en.bin",
            "cc37e93478338ec7700281a7ac30a10128929eb8f427dda2e865faa8f6da4356",
        )),
        "whisper-medium-en-q5_0" => Some(single_file_whisper(
            "ggml-medium.en-q5_0.bin",
            "76733e26ad8fe1c7a5bf7531a9d41917b2adc0f20f2e4f5531688a8c6cd88eb0",
        )),
        "whisper-medium-en-q8_0" => Some(single_file_whisper(
            "ggml-medium.en-q8_0.bin",
            "43fa2cd084de5a04399a896a9a7a786064e221365c01700cea4666005218f11c",
        )),

        // ── Whisper Large ─────────────────────────────────────────────────────
        "whisper-large-v1" => Some(single_file_whisper(
            "ggml-large-v1.bin",
            "7d99f41a10525d0206bddadd86760181fa920438b6b33237e3118ff6c83bb53d",
        )),
        "whisper-large-v2" => Some(single_file_whisper(
            "ggml-large-v2.bin",
            "9a423fe4d40c82774b6af34115b8b935f34152246eb19e80e376071d3f999487",
        )),
        "whisper-large-v2-q5_0" => Some(single_file_whisper(
            "ggml-large-v2-q5_0.bin",
            "3a214837221e4530dbc1fe8d734f302af393eb30bd0ed046042ebf4baf70f6f2",
        )),
        "whisper-large-v2-q8_0" => Some(single_file_whisper(
            "ggml-large-v2-q8_0.bin",
            "fef54e6d898246a65c8285bfa83bd1807e27fadf54d5d4e81754c47634737e8c",
        )),
        "whisper-large-v3" => Some(single_file_whisper(
            "ggml-large-v3.bin",
            "64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2",
        )),
        "whisper-large-v3-q5_0" => Some(single_file_whisper(
            "ggml-large-v3-q5_0.bin",
            "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1",
        )),
        "whisper-large-v3-turbo" => Some(single_file_whisper(
            "ggml-large-v3-turbo.bin",
            "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        )),
        "whisper-large-v3-turbo-q5_0" => Some(single_file_whisper(
            "ggml-large-v3-turbo-q5_0.bin",
            "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        )),
        "whisper-large-v3-turbo-q8_0" => Some(single_file_whisper(
            "ggml-large-v3-turbo-q8_0.bin",
            "317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1",
        )),

        // ── Whisper CoreML Encoders (macOS Apple Silicon) ─────────────────────
        // SHA-256 sourced from HuggingFace LFS metadata (lfs.oid).
        // whisper.cpp automatically uses CoreML when the .mlmodelc directory is present.
        "whisper-tiny-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-tiny-encoder.mlmodelc",
                remote_path: "ggml-tiny-encoder.mlmodelc.zip",
                sha1: "c88cbd2648e1f5415092bcf5256add463a0f19943e6938f46e8d4ffdebd47739",
            }],
            subdirectory: None,
        }),
        "whisper-tiny-en-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-tiny.en-encoder.mlmodelc",
                remote_path: "ggml-tiny.en-encoder.mlmodelc.zip",
                sha1: "82b32eef73c94bb0c432a776a047b757d9525c26d84038a15d8798d7c8d1ee58",
            }],
            subdirectory: None,
        }),
        "whisper-base-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-base-encoder.mlmodelc",
                remote_path: "ggml-base-encoder.mlmodelc.zip",
                sha1: "7e6ab77041942572f239b5b602f8aaa1c3ed29d73e3d8f20abea03a773541089",
            }],
            subdirectory: None,
        }),
        "whisper-base-en-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-base.en-encoder.mlmodelc",
                remote_path: "ggml-base.en-encoder.mlmodelc.zip",
                sha1: "8cf860309e2449e2bdc8be834cf838ab2565747ecc8c0ef914ef5975115e192b",
            }],
            subdirectory: None,
        }),
        "whisper-small-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-small-encoder.mlmodelc",
                remote_path: "ggml-small-encoder.mlmodelc.zip",
                sha1: "de43fb9fed471e95c19e60ae67575c2bf09e8fb607016da171b06ddad313988b",
            }],
            subdirectory: None,
        }),
        "whisper-small-en-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-small.en-encoder.mlmodelc",
                remote_path: "ggml-small.en-encoder.mlmodelc.zip",
                sha1: "b2ef1c506378b825b4b4341979a93e1656b5d6c129f17114cfb8fb78aabc2f89",
            }],
            subdirectory: None,
        }),
        "whisper-medium-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-medium-encoder.mlmodelc",
                remote_path: "ggml-medium-encoder.mlmodelc.zip",
                sha1: "79b0b8d436d47d3f24dd3afc91f19447dd686a4f37521b2f6d9c30a642133fbd",
            }],
            subdirectory: None,
        }),
        "whisper-medium-en-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-medium.en-encoder.mlmodelc",
                remote_path: "ggml-medium.en-encoder.mlmodelc.zip",
                sha1: "cdc44fee3c62b5743913e3147ed75f4e8ecfb52dd7a0f0f7387094b406ff0ee6",
            }],
            subdirectory: None,
        }),
        "whisper-large-v3-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-large-v3-encoder.mlmodelc",
                remote_path: "ggml-large-v3-encoder.mlmodelc.zip",
                sha1: "47837be7594a29429ec08620043390c4d6d467f8bd362df09e9390ace76a55a4",
            }],
            subdirectory: None,
        }),
        "whisper-large-v3-turbo-coreml" => Some(ModelConfig {
            repo: DEFAULT_HF_REPO,
            branch: DEFAULT_HF_BRANCH,
            files: vec![ModelFile {
                filename: "ggml-large-v3-turbo-encoder.mlmodelc",
                remote_path: "ggml-large-v3-turbo-encoder.mlmodelc.zip",
                sha1: "84bedfe895bd7b5de6e8e89a0803dfc5addf8c0c5bc4c937451716bf7cf7988a",
            }],
            subdirectory: None,
        }),

        // ── Parakeet ──────────────────────────────────────────────────────────
        // SHA-256 sourced from HuggingFace LFS metadata (lfs.oid).
        "parakeet-nemotron" => Some(ModelConfig {
            repo: "altunenes/parakeet-rs",
            branch: "main",
            files: vec![
                ModelFile {
                    filename: "decoder_joint.onnx",
                    remote_path: "nemotron-speech-streaming-en-0.6b/decoder_joint.onnx",
                    sha1: "8bcfde85fa9039a70caeb90204273f837923d63a706c186bd33e2ada25a91700",
                },
                ModelFile {
                    filename: "encoder.onnx",
                    remote_path: "nemotron-speech-streaming-en-0.6b/encoder.onnx",
                    sha1: "5c5110ca2e961c3ff5edc2b0ff49f29888b5213287624f7865c60f7384ac02f0",
                },
                ModelFile {
                    filename: "encoder.onnx.data",
                    remote_path: "nemotron-speech-streaming-en-0.6b/encoder.onnx.data",
                    sha1: "44f65771e1570546f61106b3d0c604a60b398d061476fda8042bb05432601bd4",
                },
                ModelFile {
                    filename: "tokenizer.model",
                    remote_path: "nemotron-speech-streaming-en-0.6b/tokenizer.model",
                    sha1: "07d4e5a63840a53ab2d4d106d2874768143fb3fbdd47938b3910d2da05bfb0a9",
                },
            ],
            subdirectory: Some("parakeet-nemotron"),
        }),

        // ── LLM ───────────────────────────────────────────────────────────────
        // SHA-256 sourced from HuggingFace LFS metadata (lfs.oid).
        "flowscribe-qwen2.5-0.5b" => Some(ModelConfig {
            repo: "Abdullahu5mani/flowscribe-qwen2.5-0.5b",
            branch: "main",
            files: vec![ModelFile {
                filename: "model_q4_k_m.gguf",
                remote_path: "model_q4_k_m.gguf",
                sha1: "26655766ab6d63ef33a023eb486fb0a020aa8fbcd7041a7fdb3347127fbde5d2",
            }],
            subdirectory: Some("qwen_finetuned_gguf"),
        }),

        // ── Granite Speech ─────────────────────────────────────────────────────
        // q4: FP32 I/O, runs on any hardware (~1.8 GB)
        "granite-speech-1b-cpu" => Some(ModelConfig {
            repo: "onnx-community/granite-4.0-1b-speech-ONNX",
            branch: "main",
            files: vec![
                ModelFile { filename: "audio_encoder_q4.onnx",             remote_path: "onnx/audio_encoder_q4.onnx",             sha1: "" },
                ModelFile { filename: "audio_encoder_q4.onnx_data",        remote_path: "onnx/audio_encoder_q4.onnx_data",        sha1: "" },
                ModelFile { filename: "embed_tokens_q4.onnx",              remote_path: "onnx/embed_tokens_q4.onnx",              sha1: "" },
                ModelFile { filename: "embed_tokens_q4.onnx_data",         remote_path: "onnx/embed_tokens_q4.onnx_data",         sha1: "" },
                ModelFile { filename: "decoder_model_merged_q4.onnx",      remote_path: "onnx/decoder_model_merged_q4.onnx",      sha1: "" },
                ModelFile { filename: "decoder_model_merged_q4.onnx_data", remote_path: "onnx/decoder_model_merged_q4.onnx_data", sha1: "" },
                ModelFile { filename: "tokenizer.json",                    remote_path: "tokenizer.json",                         sha1: "" },
            ],
            subdirectory: Some("granite-speech-1b"),
        }),

        // Full FP16 ONNX (~4.6 GB) — Windows + NVIDIA CUDA oriented; same APIs as INT4 bundle.
        "granite-speech-1b-fp16-cuda" => Some(ModelConfig {
            repo: "onnx-community/granite-4.0-1b-speech-ONNX",
            branch: "main",
            files: vec![
                ModelFile { filename: "audio_encoder_fp16.onnx", remote_path: "onnx/audio_encoder_fp16.onnx", sha1: "" },
                ModelFile { filename: "audio_encoder_fp16.onnx_data", remote_path: "onnx/audio_encoder_fp16.onnx_data", sha1: "" },
                ModelFile { filename: "embed_tokens_fp16.onnx", remote_path: "onnx/embed_tokens_fp16.onnx", sha1: "" },
                ModelFile { filename: "embed_tokens_fp16.onnx_data", remote_path: "onnx/embed_tokens_fp16.onnx_data", sha1: "" },
                ModelFile { filename: "decoder_model_merged_fp16.onnx", remote_path: "onnx/decoder_model_merged_fp16.onnx", sha1: "" },
                ModelFile { filename: "decoder_model_merged_fp16.onnx_data", remote_path: "onnx/decoder_model_merged_fp16.onnx_data", sha1: "" },
                ModelFile { filename: "decoder_model_merged_fp16.onnx_data_1", remote_path: "onnx/decoder_model_merged_fp16.onnx_data_1", sha1: "" },
                ModelFile { filename: "tokenizer.json", remote_path: "tokenizer.json", sha1: "" },
            ],
            subdirectory: Some("granite-speech-1b-fp16"),
        }),
        _ => None,
    }
}
