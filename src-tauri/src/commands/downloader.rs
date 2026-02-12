use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Write};
use tauri::{AppHandle, Emitter};

#[derive(Clone, Serialize, Deserialize)]
pub struct DownloadProgressPayload {
    pub model_id: String,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub status: String,    // "downloading", "verifying", "done", "error"
    pub current_file: u32, // Current file being downloaded (1-indexed)
    pub total_files: u32,  // Total number of files to download
}

#[derive(Serialize)]
pub struct ModelStatus {
    pub id: String,
    pub downloaded: bool,
    pub verified: bool,
    pub size_on_disk: u64,
}

// Hugging Face Repo Info (Defaults)
const DEFAULT_HF_REPO: &str = "ggerganov/whisper.cpp";
const DEFAULT_HF_BRANCH: &str = "main";

struct ModelFile {
    filename: &'static str, // Local filename (e.g. "ggml-tiny.bin" or "decoder_joint.onnx")
    remote_path: &'static str, // Remote path relative to repo root (e.g. "ggml-tiny.bin" or "nemotron.../decoder_joint.onnx")
    sha1: &'static str,
}

struct ModelConfig {
    repo: &'static str,
    branch: &'static str,
    files: Vec<ModelFile>,
    subdirectory: Option<&'static str>, // Local subdirectory to put files in
}

// Map model ID to filename and SHA1 hash
fn get_model_config(model_id: &str) -> Option<ModelConfig> {
    match model_id {
        // --- Whisper Models ---
        // (repo = default, branch = default, single file, no subdir)

        // Tiny
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

        // Base
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

        // Small
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

        // Medium
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

        // Large
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

        // --- Custom/Parakeet Models ---
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

        // --- Spellcheck ---
        "symspell-en-82k" => Some(ModelConfig {
            repo: "github:wolfgarbe/SymSpell",
            branch: "master",
            files: vec![ModelFile {
                filename: "frequency_dictionary_en_82_765.txt",
                remote_path: "SymSpell/frequency_dictionary_en_82_765.txt",
                sha1: "",
            }],
            subdirectory: Some("spellcheck"),
        }),

        // --- LLM ---
        "qwen2.5-0.5b-instruct" => Some(ModelConfig {
            repo: "Qwen/Qwen2.5-0.5B-Instruct-GGUF",
            branch: "main",
            files: vec![ModelFile {
                filename: "qwen2.5-0.5b-instruct-q4_k_m.gguf",
                remote_path: "qwen2.5-0.5b-instruct-q4_k_m.gguf",
                sha1: "", // GGUF files don't have published hashes, skip verification
            }],
            subdirectory: Some("Qwen2.5-0.5B-Instruct"),
        }),
        // Tokenizer files for Qwen (from the non-GGUF repo)
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
        // SafeTensors model for NVIDIA GPU users (full precision, CUDA-compatible)
        // This downloads the full 988 MB model.safetensors for optimal GPU performance
        "qwen2.5-0.5b-safetensors" => Some(ModelConfig {
            repo: "Qwen/Qwen2.5-0.5B",
            branch: "main",
            files: vec![
                ModelFile {
                    filename: "model.safetensors",
                    remote_path: "model.safetensors",
                    sha1: "", // 988 MB - full precision model for GPU
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

        "parakeet-ctc" => Some(single_file_whisper("parakeet-ctc.onnx", "")), // Placeholder for now

        _ => None,
    }
}

// Helper to create a standard Whisper config
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

#[tauri::command]
pub async fn verify_model_hash(app: AppHandle, model_id: String) -> Result<bool, String> {
    let config =
        get_model_config(&model_id).ok_or_else(|| format!("Unknown model ID: {}", model_id))?;

    // For now, if any file has SHA1, we verify it. If a file has empty SHA1, we skip it.
    // If NO files have SHA1, we skip verification entirely.
    let has_any_hash = config.files.iter().any(|f| !f.sha1.is_empty());

    if !has_any_hash {
        return Ok(true);
    }

    let models_dir =
        crate::utils::get_models_dir().map_err(|e| format!("Failed to get models dir: {}", e))?;
    let base_dir = if let Some(subdir) = config.subdirectory {
        models_dir.join(subdir)
    } else {
        models_dir.clone()
    };

    let total_files = config.files.len();
    let mut verified_count = 0;

    for (i, file_spec) in config.files.iter().enumerate() {
        if file_spec.sha1.is_empty() {
            continue;
        }

        let file_path = base_dir.join(file_spec.filename);
        if !file_path.exists() {
            return Err(format!("File not found: {}", file_spec.filename));
        }

        println!(
            "[VERIFY] Calculating SHA1 for {} ({}/{})...",
            file_spec.filename,
            i + 1,
            total_files
        );
        let _ = app.emit(
            "download-progress",
            DownloadProgressPayload {
                model_id: model_id.clone(),
                total_bytes: 0,
                downloaded_bytes: 0,
                status: "verifying".to_string(),
                current_file: (i + 1) as u32,
                total_files: total_files as u32,
            },
        );

        let mut file = File::open(&file_path).map_err(|e| e.to_string())?;
        let mut hasher = sha1::Sha1::new();
        let mut buffer = [0; 8192];
        use sha1::Digest;

        loop {
            let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }

        let result = hasher.finalize();
        let hash_hex = hex::encode(result);

        println!(
            "[VERIFY] {} SHA1: Expected {}, Got {}",
            file_spec.filename, file_spec.sha1, hash_hex
        );

        if hash_hex != file_spec.sha1 {
            return Err(format!(
                "Hash mismatch for {}: Expected {}, Got {}",
                file_spec.filename, file_spec.sha1, hash_hex
            ));
        }
        verified_count += 1;
    }

    // Mark as verified only if we actually verified something
    if verified_count > 0 {
        // Create .verified marker for the main filename (or first file)
        // For multi-file models, we might want a folder-level verified marker, but let's stick to per-file
        // or just use the first filename + .verified
        let verified_marker = base_dir.join(format!("{}.verified", config.files[0].filename));
        if let Ok(mut v_file) = File::create(&verified_marker) {
            let _ = v_file.write_all(b"verified");
        }
    }

    let _ = app.emit(
        "download-progress",
        DownloadProgressPayload {
            model_id: model_id.clone(),
            total_bytes: 100,
            downloaded_bytes: 100,
            status: "done".to_string(),
            current_file: total_files as u32,
            total_files: total_files as u32,
        },
    );

    Ok(true)
}

#[tauri::command]
pub async fn get_download_status(
    _app: AppHandle,
    model_ids: Vec<String>,
) -> Result<Vec<ModelStatus>, String> {
    let models_dir =
        crate::utils::get_models_dir().map_err(|e| format!("Failed to get models dir: {}", e))?;

    let mut statuses = Vec::new();

    for id in model_ids {
        if let Some(config) = get_model_config(&id) {
            let base_dir = if let Some(subdir) = config.subdirectory {
                models_dir.join(subdir)
            } else {
                models_dir.clone()
            };

            // Check verified marker on the first file
            let verified_marker = base_dir.join(format!("{}.verified", config.files[0].filename));
            let mut verified = verified_marker.exists();

            let mut all_exist = true;
            let mut total_size: u64 = 0;

            for file_spec in &config.files {
                let file_path = base_dir.join(file_spec.filename);
                if file_path.exists() {
                    if let Ok(metadata) = std::fs::metadata(&file_path) {
                        total_size += metadata.len();
                    } else {
                        all_exist = false;
                    }
                } else {
                    all_exist = false;
                }
            }

            // Status is downloaded only if ALL files exist and total size > 0
            let downloaded = all_exist && total_size > 0;
            if !downloaded {
                verified = false;
            }

            statuses.push(ModelStatus {
                id,
                downloaded,
                verified,
                size_on_disk: total_size,
            });
        }
    }

    Ok(statuses)
}

#[tauri::command]
pub async fn delete_model(_app: AppHandle, model_id: String) -> Result<String, String> {
    let config =
        get_model_config(&model_id).ok_or_else(|| format!("Unknown model ID: {}", model_id))?;
    let models_dir =
        crate::utils::get_models_dir().map_err(|e| format!("Failed to get models dir: {}", e))?;

    let base_dir = if let Some(subdir) = config.subdirectory {
        models_dir.join(subdir)
    } else {
        models_dir.clone()
    };

    // Delete all files
    for file_spec in &config.files {
        let file_path = base_dir.join(file_spec.filename);
        if file_path.exists() {
            let _ = std::fs::remove_file(&file_path);
        }
    }

    // Delete verified marker
    let verified_marker = base_dir.join(format!("{}.verified", config.files[0].filename));
    if verified_marker.exists() {
        let _ = std::fs::remove_file(&verified_marker);
    }

    // Try to remove subdir if empty
    if config.subdirectory.is_some() {
        let _ = std::fs::remove_dir(&base_dir); // Fails if not empty, which is fine
    }

    Ok(format!("Deleted model {}", model_id))
}

#[tauri::command]
pub async fn download_model(app: AppHandle, model_id: String) -> Result<String, String> {
    let config =
        get_model_config(&model_id).ok_or_else(|| format!("Unknown model ID: {}", model_id))?;
    let models_dir =
        crate::utils::get_models_dir().map_err(|e| format!("Failed to get models dir: {}", e))?;

    let base_dir = if let Some(subdir) = config.subdirectory {
        models_dir.join(subdir)
    } else {
        models_dir.clone()
    };

    if !base_dir.exists() {
        std::fs::create_dir_all(&base_dir)
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }

    let files_count = config.files.len();

    // Note: We don't know total size of all files upfront easily without head requests.
    // We will track downloaded bytes cumulatively, but 'total' will be per-file for progress.
    // Ideally we'd sum them up, but HTTP calls take time.
    // We will just emit progress for each file independently or try to aggregate if we can.
    // For simplicity, we'll emit status text like "Downloading file 1/X..." via the existing payload structure implicitly?
    // Actually, the frontend expects 0->100 %.
    // To keep it simple, we will sequence them.

    for (i, file_spec) in config.files.iter().enumerate() {
        let url = if config.repo.starts_with("github:") {
            let repo_path = config.repo.trim_start_matches("github:");
            format!(
                "https://raw.githubusercontent.com/{}/{}/{}",
                repo_path, config.branch, file_spec.remote_path
            )
        } else {
            format!(
                "https://huggingface.co/{}/resolve/{}/{}",
                config.repo, config.branch, file_spec.remote_path
            )
        };
        let target_path = base_dir.join(file_spec.filename);

        println!(
            "[DOWNLOAD] Starting download for {} ({}/{}) from {}",
            model_id,
            i + 1,
            files_count,
            url
        );

        let client = Client::new();
        let res = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to connect to Hugging Face: {}", e))?;

        let total_size = res.content_length().unwrap_or(0);
        let mut file =
            File::create(&target_path).map_err(|e| format!("Failed to create file: {}", e))?;

        let mut downloaded: u64 = 0;
        let mut stream = res.bytes_stream();
        let mut last_emit = 0;
        let emit_threshold = 1024 * 1024; // 1MB

        // Calculate progress base for this file
        // This is imperfect but works: We will just show 0-100% for EACH file.
        // Or we can try to hack it. Let's just do per-file 0-100% for now.
        // Frontend might see it jump back to 0.

        // Better UX: Send "downloading" status.
        while let Some(item) = stream.next().await {
            let chunk = item.map_err(|e| format!("Error while downloading chunk: {}", e))?;
            file.write_all(&chunk)
                .map_err(|e| format!("Error writing to file: {}", e))?;

            downloaded += chunk.len() as u64;

            if downloaded - last_emit > emit_threshold || downloaded == total_size {
                last_emit = downloaded;
                let _ = app.emit(
                    "download-progress",
                    DownloadProgressPayload {
                        model_id: model_id.clone(),
                        total_bytes: total_size,
                        downloaded_bytes: downloaded,
                        status: "downloading".to_string(), // Frontend just shows % based on these two numbers
                        current_file: (i + 1) as u32,
                        total_files: files_count as u32,
                    },
                );
            }
        }
    }

    println!("[DOWNLOAD] Finished downloading {}", model_id);

    let _ = app.emit(
        "download-progress",
        DownloadProgressPayload {
            model_id: model_id.clone(),
            total_bytes: 100,
            downloaded_bytes: 100,
            status: "done".to_string(),
            current_file: files_count as u32,
            total_files: files_count as u32,
        },
    );

    Ok(format!("Downloaded to {:?}", base_dir))
}
