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

/// Build a Whisper model config that bundles the weight binary with its
/// companion CoreML encoder archive (`ggml-{stem}-encoder.mlmodelc.zip`).
///
/// On Apple Silicon the downloader fetches + extracts both files so
/// `whisper.rs` finds the `ggml-{stem}-encoder.mlmodelc` directory next to
/// the `.bin` and activates the CoreML ANE backend automatically. On other
/// platforms the downloader skips the `.zip` entry (see `downloader.rs`).
fn whisper_with_coreml(
    bin_filename: &'static str,
    bin_sha256: &'static str,
    encoder_dirname: &'static str,
    encoder_zip: &'static str,
    encoder_sha256: &'static str,
) -> ModelConfig {
    ModelConfig {
        repo: DEFAULT_HF_REPO,
        branch: DEFAULT_HF_BRANCH,
        files: vec![
            ModelFile {
                filename: bin_filename,
                remote_path: bin_filename,
                sha1: bin_sha256,
            },
            ModelFile {
                filename: encoder_dirname,
                remote_path: encoder_zip,
                sha1: encoder_sha256,
            },
        ],
        subdirectory: None,
    }
}

/// Returns true when a registry file entry is a CoreML encoder bundle
/// (a `.mlmodelc` directory delivered as a `.zip` archive).
pub fn is_coreml_bundle_file(filename: &str, remote_path: &str) -> bool {
    filename.ends_with(".mlmodelc") || remote_path.ends_with(".mlmodelc.zip")
}

/// True only on macOS Apple Silicon builds, where the ANE can execute the
/// CoreML encoder graph. Everywhere else the companion bundle is skipped.
pub fn coreml_companion_supported() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

/// Look up the download configuration for a model by its ID.
/// Returns `None` if the model ID is not recognised.
/// Speaker recognition (voiceprints): CAM++ trained on VoxCeleb, from 3D-Speaker,
/// as published by the sherpa-onnx project. Input: 80-band fbank; output: 192-d.
pub const SPEAKER_MODEL_ID: &str = "speaker-campplus-en";
pub const SPEAKER_MODEL_DIR: &str = "speaker";
pub const SPEAKER_MODEL_FILE: &str = "3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx";

/// Speaker diarization ("who spoke when") for the call channel: NVIDIA
/// Nemotron-3 Diarization. One model choice everywhere; the download is the
/// MLX BF16 conversion on Apple Silicon and the F16 GGUF (transcribe.cpp)
/// elsewhere. Neither is quantized.
pub const DIARIZATION_MODEL_ID: &str = "diarization-nemotron3";
pub const DIARIZATION_MODEL_DIR: &str = "diarization-nemotron3";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub const DIARIZATION_MODEL_FILE: &str = "model.safetensors";
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
pub const DIARIZATION_MODEL_FILE: &str = "nemotron-3-diarization-F16.gguf";

fn diarization_model_config() -> ModelConfig {
    // Pinned commits: both conversions are new and may be re-exported.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    let (repo, commit, sha) = (
        "mlx-community/Nemotron-3-Diarization",
        "59ed2dbfc1346dcea9d423c71306a3a2499c568f",
        "21e8427d1795c9c46c5800f56b16061734ffd0dcadd71d9bcf0b4d6ef7261da5",
    );
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    let (repo, commit, sha) = (
        "Glimpse-Dictation/Nemotron-3-Diarization-gguf",
        "3bf8566b2a36b54e5951142298f806053865a297",
        "5513da21cc39fc3ab5a36bd945324aeb013369b63172849b4ef174686e15f27c",
    );
    ModelConfig {
        repo,
        branch: commit,
        files: vec![ModelFile { filename: DIARIZATION_MODEL_FILE, remote_path: DIARIZATION_MODEL_FILE, sha1: sha }],
        subdirectory: Some(DIARIZATION_MODEL_DIR),
    }
}

pub fn get_model_config(model_id: &str) -> Option<ModelConfig> {
    match model_id {
        DIARIZATION_MODEL_ID => Some(diarization_model_config()),
        SPEAKER_MODEL_ID => Some(ModelConfig {
            repo: "github-release:k2-fsa/sherpa-onnx",
            branch: "speaker-recongition-models", // (sic) the release tag's spelling
            files: vec![ModelFile {
                filename: SPEAKER_MODEL_FILE,
                remote_path: SPEAKER_MODEL_FILE,
                // SHA-256 of the 29,596,978-byte release asset (no digest is published
                // upstream; pinned from the first download, size matched GitHub's API).
                sha1: "357a834f702b80161e5b981182c038e18553c1f2ca752ed6cec2052365d4129b",
            }],
            subdirectory: Some(SPEAKER_MODEL_DIR),
        }),
        // ── Whisper Tiny ──────────────────────────────────────────────────────
        // Full-precision variants bundle the CoreML ANE encoder so a single
        // "Download Model" click pulls both files on Apple Silicon.
        "whisper-tiny" => Some(whisper_with_coreml(
            "ggml-tiny.bin",
            "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
            "ggml-tiny-encoder.mlmodelc",
            "ggml-tiny-encoder.mlmodelc.zip",
            "c88cbd2648e1f5415092bcf5256add463a0f19943e6938f46e8d4ffdebd47739",
        )),
        "whisper-tiny-q5_1" => Some(single_file_whisper(
            "ggml-tiny-q5_1.bin",
            "818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7",
        )),
        "whisper-tiny-q8_0" => Some(single_file_whisper(
            "ggml-tiny-q8_0.bin",
            "c2085835d3f50733e2ff6e4b41ae8a2b8d8110461e18821b09a15c40c42d1cca",
        )),
        "whisper-tiny-en" => Some(whisper_with_coreml(
            "ggml-tiny.en.bin",
            "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
            "ggml-tiny.en-encoder.mlmodelc",
            "ggml-tiny.en-encoder.mlmodelc.zip",
            "82b32eef73c94bb0c432a776a047b757d9525c26d84038a15d8798d7c8d1ee58",
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
        "whisper-base" => Some(whisper_with_coreml(
            "ggml-base.bin",
            "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
            "ggml-base-encoder.mlmodelc",
            "ggml-base-encoder.mlmodelc.zip",
            "7e6ab77041942572f239b5b602f8aaa1c3ed29d73e3d8f20abea03a773541089",
        )),
        "whisper-base-q5_1" => Some(single_file_whisper(
            "ggml-base-q5_1.bin",
            "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
        )),
        "whisper-base-q8_0" => Some(single_file_whisper(
            "ggml-base-q8_0.bin",
            "c577b9a86e7e048a0b7eada054f4dd79a56bbfa911fbdacf900ac5b567cbb7d9",
        )),
        "whisper-base-en" => Some(whisper_with_coreml(
            "ggml-base.en.bin",
            "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
            "ggml-base.en-encoder.mlmodelc",
            "ggml-base.en-encoder.mlmodelc.zip",
            "8cf860309e2449e2bdc8be834cf838ab2565747ecc8c0ef914ef5975115e192b",
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
        "whisper-small" => Some(whisper_with_coreml(
            "ggml-small.bin",
            "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
            "ggml-small-encoder.mlmodelc",
            "ggml-small-encoder.mlmodelc.zip",
            "de43fb9fed471e95c19e60ae67575c2bf09e8fb607016da171b06ddad313988b",
        )),
        "whisper-small-q5_1" => Some(single_file_whisper(
            "ggml-small-q5_1.bin",
            "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
        )),
        "whisper-small-q8_0" => Some(single_file_whisper(
            "ggml-small-q8_0.bin",
            "49c8fb02b65e6049d5fa6c04f81f53b867b5ec9540406812c643f177317f779f",
        )),
        "whisper-small-en" => Some(whisper_with_coreml(
            "ggml-small.en.bin",
            "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
            "ggml-small.en-encoder.mlmodelc",
            "ggml-small.en-encoder.mlmodelc.zip",
            "b2ef1c506378b825b4b4341979a93e1656b5d6c129f17114cfb8fb78aabc2f89",
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
        "whisper-medium" => Some(whisper_with_coreml(
            "ggml-medium.bin",
            "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
            "ggml-medium-encoder.mlmodelc",
            "ggml-medium-encoder.mlmodelc.zip",
            "79b0b8d436d47d3f24dd3afc91f19447dd686a4f37521b2f6d9c30a642133fbd",
        )),
        "whisper-medium-q5_0" => Some(single_file_whisper(
            "ggml-medium-q5_0.bin",
            "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f",
        )),
        "whisper-medium-q8_0" => Some(single_file_whisper(
            "ggml-medium-q8_0.bin",
            "42a1ffcbe4167d224232443396968db4d02d4e8e87e213d3ee2e03095dea6502",
        )),
        "whisper-medium-en" => Some(whisper_with_coreml(
            "ggml-medium.en.bin",
            "cc37e93478338ec7700281a7ac30a10128929eb8f427dda2e865faa8f6da4356",
            "ggml-medium.en-encoder.mlmodelc",
            "ggml-medium.en-encoder.mlmodelc.zip",
            "cdc44fee3c62b5743913e3147ed75f4e8ecfb52dd7a0f0f7387094b406ff0ee6",
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
        "whisper-large-v3" => Some(whisper_with_coreml(
            "ggml-large-v3.bin",
            "64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2",
            "ggml-large-v3-encoder.mlmodelc",
            "ggml-large-v3-encoder.mlmodelc.zip",
            "47837be7594a29429ec08620043390c4d6d467f8bd362df09e9390ace76a55a4",
        )),
        "whisper-large-v3-q5_0" => Some(single_file_whisper(
            "ggml-large-v3-q5_0.bin",
            "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1",
        )),
        "whisper-large-v3-turbo" => Some(whisper_with_coreml(
            "ggml-large-v3-turbo.bin",
            "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
            "ggml-large-v3-turbo-encoder.mlmodelc",
            "ggml-large-v3-turbo-encoder.mlmodelc.zip",
            "84bedfe895bd7b5de6e8e89a0803dfc5addf8c0c5bc4c937451716bf7cf7988a",
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

        "granite-speech-5-nc" => Some(ModelConfig {
            repo: "handy-computer/granite-speech-5.0-470m-turboctc-nc-gguf",
            branch: "main",
            files: vec![ModelFile {
                filename: "granite-speech-5.0-470m-turboctc-nc-F16.gguf",
                remote_path: "granite-speech-5.0-470m-turboctc-nc-F16.gguf",
                sha1: "baceebaaf85210f50463dfec059eb46ae6dbea8ea9a090500fbdee4f92b0c302",
            }],
            subdirectory: Some("granite-speech-5-nc"),
        }),
        "qwen3-asr-1.7b" => Some(ModelConfig {
            repo: "handy-computer/Qwen3-ASR-1.7B-gguf",
            branch: "main",
            files: vec![ModelFile {
                filename: "Qwen3-ASR-1.7B-F16.gguf",
                remote_path: "Qwen3-ASR-1.7B-F16.gguf",
                sha1: "edb09c29b8f73822c639168d5ef72aa2dccdf8b4e48fc4b8518885352ff62c71",
            }],
            subdirectory: Some("qwen3-asr-1.7b"),
        }),
        "qwen3-asr-0.6b" => Some(ModelConfig {
            repo: "handy-computer/Qwen3-ASR-0.6B-gguf",
            branch: "main",
            files: vec![ModelFile {
                filename: "Qwen3-ASR-0.6B-F16.gguf",
                remote_path: "Qwen3-ASR-0.6B-F16.gguf",
                sha1: "5c90e4b1a72a4c59cd12afa5ebb0cc8628848148f2b337d025cc5121ae4d2eea",
            }],
            subdirectory: Some("qwen3-asr-0.6b"),
        }),

        // ── LLM ───────────────────────────────────────────────────────────────
        // SHA-256 sourced from HuggingFace LFS metadata (lfs.oid).
        // FlowScribe v3: Qwen3.5-0.8B fine-tune, F16 (see scripts/flowscribe_train).
        "flowscribe-qwen3.5-0.8b-v3" => Some(ModelConfig {
            repo: "Abdullahu5mani/flowscribe-qwen3.5-0.8b-v3",
            branch: "main",
            files: vec![ModelFile {
                filename: "flowscribe-v3-f16.gguf",
                remote_path: "flowscribe-v3-f16.gguf",
                sha1: "56b813fa3572279edef6da6d2e6bce492548d99e894cdef379ff5924ce5b345d",
            }],
            subdirectory: Some("flowscribe_v3"),
        }),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unquantized_gguf_models_have_pinned_downloads() {
        for (id, repo) in [
            ("granite-speech-5-nc", "handy-computer/granite-speech-5.0-470m-turboctc-nc-gguf"),
            ("qwen3-asr-1.7b", "handy-computer/Qwen3-ASR-1.7B-gguf"),
            ("qwen3-asr-0.6b", "handy-computer/Qwen3-ASR-0.6B-gguf"),
        ] {
            let config = get_model_config(id).expect("GGUF registry entry");
            assert_eq!(config.repo, repo);
            assert_eq!(config.subdirectory, Some(id));
            assert_eq!(config.files.len(), 1);
            assert!(config.files[0].filename.ends_with("-F16.gguf"));
            assert_eq!(config.files[0].sha1.len(), 64);
        }
    }
}

#[cfg(test)]
mod ane_tests {
    use super::*;

    /// Full-precision Whisper models must bundle the ANE encoder so one
    /// click downloads both the weights and the `.mlmodelc` companion.
    const ANE_BUNDLED: &[(&str, &str, &str)] = &[
        (
            "whisper-tiny",
            "ggml-tiny.bin",
            "ggml-tiny-encoder.mlmodelc",
        ),
        (
            "whisper-tiny-en",
            "ggml-tiny.en.bin",
            "ggml-tiny.en-encoder.mlmodelc",
        ),
        (
            "whisper-base",
            "ggml-base.bin",
            "ggml-base-encoder.mlmodelc",
        ),
        (
            "whisper-base-en",
            "ggml-base.en.bin",
            "ggml-base.en-encoder.mlmodelc",
        ),
        (
            "whisper-small",
            "ggml-small.bin",
            "ggml-small-encoder.mlmodelc",
        ),
        (
            "whisper-small-en",
            "ggml-small.en.bin",
            "ggml-small.en-encoder.mlmodelc",
        ),
        (
            "whisper-medium",
            "ggml-medium.bin",
            "ggml-medium-encoder.mlmodelc",
        ),
        (
            "whisper-medium-en",
            "ggml-medium.en.bin",
            "ggml-medium.en-encoder.mlmodelc",
        ),
        (
            "whisper-large-v3",
            "ggml-large-v3.bin",
            "ggml-large-v3-encoder.mlmodelc",
        ),
        (
            "whisper-large-v3-turbo",
            "ggml-large-v3-turbo.bin",
            "ggml-large-v3-turbo-encoder.mlmodelc",
        ),
    ];

    #[test]
    fn full_precision_whisper_models_bundle_coreml_encoder() {
        for (model_id, bin, encoder_dir) in ANE_BUNDLED {
            let config = get_model_config(model_id).expect("ANE-capable registry entry");
            assert_eq!(config.files.len(), 2, "missing companion for {model_id}");
            let weight = &config.files[0];
            assert_eq!(weight.filename, *bin);
            assert!(weight.remote_path.ends_with(".bin"));
            let encoder = &config.files[1];
            assert_eq!(encoder.filename, *encoder_dir);
            assert!(
                encoder.remote_path.ends_with(".mlmodelc.zip"),
                "encoder must be a zip archive for {model_id}"
            );
            assert!(is_coreml_bundle_file(encoder.filename, encoder.remote_path));
            // Both artifacts are checksum-pinned so a truncated download
            // surfaces as a clear failure, not garbled audio.
            for file in &config.files {
                assert_eq!(file.sha1.len(), 64, "unpinned hash for {model_id}");
                assert!(file.sha1.bytes().all(|b| b.is_ascii_hexdigit()));
            }
        }
    }

    #[test]
    fn quantized_whisper_models_stay_single_file() {
        for model_id in [
            "whisper-tiny-q5_1",
            "whisper-base-q5_1",
            "whisper-small-q5_1",
            "whisper-medium-q5_0",
            "whisper-large-v3-turbo-q5_0",
        ] {
            let config = get_model_config(model_id).expect("quantized registry entry");
            assert_eq!(
                config.files.len(),
                1,
                "{model_id} must not auto-pull ANE bundle"
            );
            assert!(!is_coreml_bundle_file(
                config.files[0].filename,
                config.files[0].remote_path
            ));
        }
    }

    #[test]
    fn coreml_bundle_detection_matches_naming_convention() {
        assert!(is_coreml_bundle_file(
            "ggml-small-encoder.mlmodelc",
            "ggml-small-encoder.mlmodelc.zip"
        ));
        assert!(!is_coreml_bundle_file("ggml-small.bin", "ggml-small.bin"));
    }
}
