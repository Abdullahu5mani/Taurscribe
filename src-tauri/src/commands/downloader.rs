use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::time::Duration;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Emitter};
use zip::ZipArchive;

// ── Cancellation registry ─────────────────────────────────────────────────────

static CANCEL_FLAGS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

fn cancel_flags() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    CANCEL_FLAGS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_cancel_flag(model_id: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    cancel_flags()
        .lock()
        .unwrap()
        .insert(model_id.to_string(), Arc::clone(&flag));
    flag
}

fn unregister_cancel_flag(model_id: &str) {
    cancel_flags().lock().unwrap().remove(model_id);
}

/// Delete all files/directories belonging to a model (used on cancel or hash mismatch).
fn delete_model_files(config: &ModelConfig, base_dir: &std::path::Path) {
    for fs in &config.files {
        let p = base_dir.join(fs.filename);
        if p.is_dir() {
            let _ = std::fs::remove_dir_all(&p);
        } else {
            let _ = std::fs::remove_file(&p);
        }
    }
    if config.subdirectory.is_some() {
        let _ = std::fs::remove_dir(base_dir);
    }
}

/// Cancel an in-progress download. Deletes all partial files for that model.
#[tauri::command]
pub async fn cancel_download(model_id: String) -> Result<(), String> {
    if let Some(flag) = cancel_flags().lock().unwrap().get(&model_id) {
        flag.store(true, Ordering::Relaxed);
    }
    Ok(())
}

use super::model_registry::{get_model_config, ModelConfig, ModelFile};

// ── Verification store ────────────────────────────────────────────────────────

/// One entry in verified.json per model.
#[derive(Clone, Serialize, Deserialize)]
struct VerifiedEntry {
    /// Concatenated SHA-256 hashes of all files, joined with "+".
    /// Matches against the current registry hashes to detect stale records.
    fingerprint: String,
    verified_at: String,
}

type VerifiedStore = HashMap<String, VerifiedEntry>;

/// Path to verified.json inside the models directory.
fn verified_store_path() -> Result<std::path::PathBuf, String> {
    let dir = crate::utils::get_models_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("verified.json"))
}

fn load_verified_store() -> VerifiedStore {
    let path = match verified_store_path() {
        Ok(p) => p,
        Err(_) => return HashMap::new(),
    };
    let Ok(data) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };
    serde_json::from_str(&data).unwrap_or_default()
}

fn save_verified_store(store: &VerifiedStore) {
    let Ok(path) = verified_store_path() else {
        return;
    };
    if let Ok(json) = serde_json::to_string_pretty(store) {
        let _ = std::fs::write(&path, json);
    }
}

/// Build the expected fingerprint from the registry (all sha256 values joined with "+").
/// Returns an empty string if no file has a hash (verification disabled).
fn registry_fingerprint(files: &[ModelFile]) -> String {
    files
        .iter()
        .map(|f| f.sha1) // sha1 field now holds SHA-256
        .collect::<Vec<_>>()
        .join("+")
}

/// Returns true if all hashes in the fingerprint are empty (verification disabled).
fn fingerprint_is_empty(fp: &str) -> bool {
    fp.split('+').all(|h| h.is_empty())
}

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Clone, Serialize, Deserialize)]
pub struct DownloadProgressPayload {
    pub model_id: String,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub status: String, // "downloading" | "verifying" | "done" | "error"
    pub current_file: u32,
    pub total_files: u32,
}

/// Delete partial model files and emit `error` to the download manager UI.
fn emit_download_error_and_cleanup(
    app: &AppHandle,
    model_id: &str,
    config: &ModelConfig,
    base_dir: &std::path::Path,
    current_file: u32,
    files_count: u32,
    message: &str,
) -> String {
    delete_model_files(config, base_dir);
    let _ = app.emit(
        "download-progress",
        DownloadProgressPayload {
            model_id: model_id.to_string(),
            total_bytes: 0,
            downloaded_bytes: 0,
            status: "error".to_string(),
            current_file,
            total_files: files_count,
        },
    );
    eprintln!("[DOWNLOAD] {}", message);
    message.to_string()
}

#[derive(Serialize)]
pub struct ModelStatus {
    pub id: String,
    pub downloaded: bool,
    pub verified: bool,
    pub size_on_disk: u64,
}

// ── Commands ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_download_status(
    _app: AppHandle,
    model_ids: Vec<String>,
) -> Result<Vec<ModelStatus>, String> {
    let models_dir =
        crate::utils::get_models_dir().map_err(|e| format!("Failed to get models dir: {}", e))?;
    let store = load_verified_store();

    let mut statuses = Vec::new();

    for id in model_ids {
        if let Some(config) = get_model_config(&id) {
            let base_dir = if let Some(subdir) = config.subdirectory {
                models_dir.join(subdir)
            } else {
                models_dir.clone()
            };

            // Check all files exist on disk and sum their sizes.
            let mut all_exist = true;
            let mut total_size: u64 = 0;

            for file_spec in &config.files {
                let file_path = base_dir.join(file_spec.filename);
                if file_path.exists() {
                    if file_path.is_dir() {
                        total_size += 1; // CoreML .mlmodelc directories
                    } else if let Ok(metadata) = std::fs::metadata(&file_path) {
                        total_size += metadata.len();
                    } else {
                        all_exist = false;
                    }
                } else {
                    all_exist = false;
                }
            }

            let downloaded = all_exist && total_size > 0;

            // Verification: compare stored fingerprint against the current registry.
            let verified = if !downloaded {
                false
            } else {
                let expected_fp = registry_fingerprint(&config.files);
                if fingerprint_is_empty(&expected_fp) {
                    // No hashes in registry → treat as verified (nothing to check).
                    true
                } else {
                    match store.get(&id) {
                        Some(entry) => entry.fingerprint == expected_fp,
                        None => false,
                    }
                }
            };

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
pub async fn download_model(app: AppHandle, model_id: String) -> Result<String, String> {
    let cancel_flag = register_cancel_flag(&model_id);
    let result = download_model_inner(&app, &model_id, &cancel_flag).await;
    unregister_cancel_flag(&model_id);
    result
}

async fn download_model_inner(
    app: &AppHandle,
    model_id: &str,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<String, String> {
    let config =
        get_model_config(model_id).ok_or_else(|| format!("Unknown model ID: {}", model_id))?;
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

    // ── Download phase ────────────────────────────────────────────────────────
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

        let is_zip = file_spec.remote_path.ends_with(".zip");
        let download_path = if is_zip {
            base_dir.join(format!("{}.zip", file_spec.filename))
        } else {
            base_dir.join(file_spec.filename)
        };

        println!(
            "[DOWNLOAD] {} ({}/{}) from {}",
            model_id,
            i + 1,
            files_count,
            url
        );

        let client = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

        let emit_error = |app: &AppHandle, model_id: &str, i: usize, files_count: usize, msg: &str| {
            emit_download_error_and_cleanup(
                app,
                model_id,
                &config,
                &base_dir,
                (i + 1) as u32,
                files_count as u32,
                msg,
            )
        };

        let res = client.get(&url).send().await.map_err(|e| {
            let reason = if e.is_connect() || e.is_timeout() {
                "No internet connection — check your network and try again."
            } else {
                "Failed to connect to download server."
            };
            emit_error(app, model_id, i, files_count, reason)
        })?;

        if !res.status().is_success() {
            return Err(emit_error(
                app, model_id, i, files_count,
                &format!("Download server returned HTTP {}", res.status()),
            ));
        }

        let total_size = res.content_length().unwrap_or(0);
        let mut file =
            File::create(&download_path).map_err(|e| format!("Failed to create file: {}", e))?;

        let mut downloaded: u64 = 0;
        let mut stream = res.bytes_stream();
        let mut last_emit: u64 = 0;
        let emit_threshold = 1024 * 1024; // 1 MB

        while let Some(item) = stream.next().await {
            let chunk = match item {
                Ok(c) => c,
                Err(e) => {
                    drop(file);
                    let _ = std::fs::remove_file(&download_path);
                    let reason = if e.is_timeout() {
                        "Connection lost — no data received for 30 seconds. Check your internet and try again."
                    } else if e.is_connect() || e.to_string().contains("reset") || e.to_string().contains("connection") {
                        "Connection lost during download. Check your internet and try again."
                    } else {
                        "Download interrupted — a network error occurred."
                    };
                    return Err(emit_error(app, model_id, i, files_count, reason));
                }
            };
            if let Err(e) = file.write_all(&chunk) {
                drop(file);
                let _ = std::fs::remove_file(&download_path);
                return Err(emit_error(
                    app,
                    model_id,
                    i,
                    files_count,
                    &format!("Download failed — could not write file ({})", e),
                ));
            }
            downloaded += chunk.len() as u64;

            if downloaded - last_emit > emit_threshold || downloaded == total_size {
                last_emit = downloaded;
                let _ = app.emit(
                    "download-progress",
                    DownloadProgressPayload {
                        model_id: model_id.to_string(),
                        total_bytes: total_size,
                        downloaded_bytes: downloaded,
                        status: "downloading".to_string(),
                        current_file: (i + 1) as u32,
                        total_files: files_count as u32,
                    },
                );

                // Check for user cancellation at each progress emit.
                if cancel_flag.load(Ordering::Relaxed) {
                    drop(file);
                    let _ = std::fs::remove_file(&download_path);
                    delete_model_files(&config, &base_dir);
                    let _ = app.emit(
                        "download-progress",
                        DownloadProgressPayload {
                            model_id: model_id.to_string(),
                            total_bytes: 0,
                            downloaded_bytes: 0,
                            status: "cancelled".to_string(),
                            current_file: (i + 1) as u32,
                            total_files: files_count as u32,
                        },
                    );
                    return Err("Download cancelled by user".to_string());
                }
            }
        }
        drop(file);

        if is_zip {
            // Emit extraction-start event so the UI can show the purple bar.
            let _ = app.emit(
                "download-progress",
                DownloadProgressPayload {
                    model_id: model_id.to_string(),
                    total_bytes: 0,
                    downloaded_bytes: 0,
                    status: "extracting".to_string(),
                    current_file: (i + 1) as u32,
                    total_files: files_count as u32,
                },
            );

            println!("[DOWNLOAD] Extracting zip: {:?}", download_path);

            let zip_fail = |detail: String| -> String {
                let _ = std::fs::remove_file(&download_path);
                emit_error(app, model_id, i, files_count, &detail)
            };

            let zip_file = File::open(&download_path)
                .map_err(|e| zip_fail(format!("Failed to open zip for extraction: {}", e)))?;
            let mut archive = ZipArchive::new(zip_file)
                .map_err(|e| zip_fail(format!("Failed to read zip archive: {}", e)))?;
            let total_entries = archive.len() as u64;

            for entry_idx in 0..archive.len() {
                let mut entry = archive
                    .by_index(entry_idx)
                    .map_err(|e| zip_fail(format!("Failed to read zip entry: {}", e)))?;
                let outpath = match entry.enclosed_name() {
                    Some(path) => base_dir.join(path),
                    None => continue,
                };
                if entry.is_dir() {
                    std::fs::create_dir_all(&outpath).map_err(|e| {
                        zip_fail(format!("Failed to create dir during extraction: {}", e))
                    })?;
                } else {
                    if let Some(parent) = outpath.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| {
                            zip_fail(format!("Failed to create parent dir: {}", e))
                        })?;
                    }
                    let mut out_file = File::create(&outpath).map_err(|e| {
                        zip_fail(format!("Failed to create extracted file: {}", e))
                    })?;
                    std::io::copy(&mut entry, &mut out_file).map_err(|e| {
                        zip_fail(format!("Failed to write extracted file: {}", e))
                    })?;
                }
                // Progress: bytes = entries done, total = total entries
                let _ = app.emit(
                    "download-progress",
                    DownloadProgressPayload {
                        model_id: model_id.to_string(),
                        total_bytes: total_entries,
                        downloaded_bytes: (entry_idx + 1) as u64,
                        status: "extracting".to_string(),
                        current_file: (i + 1) as u32,
                        total_files: files_count as u32,
                    },
                );

                // Check for cancellation during extraction.
                if cancel_flag.load(Ordering::Relaxed) {
                    let _ = std::fs::remove_file(&download_path);
                    delete_model_files(&config, &base_dir);
                    let _ = app.emit(
                        "download-progress",
                        DownloadProgressPayload {
                            model_id: model_id.to_string(),
                            total_bytes: 0,
                            downloaded_bytes: 0,
                            status: "cancelled".to_string(),
                            current_file: (i + 1) as u32,
                            total_files: files_count as u32,
                        },
                    );
                    return Err("Download cancelled by user".to_string());
                }
            }

            std::fs::remove_file(&download_path).ok();
            println!("[DOWNLOAD] Extraction complete.");
        }
    }

    println!("[DOWNLOAD] Finished downloading {}", model_id);

    // ── Auto-verify phase ─────────────────────────────────────────────────────
    let expected_fp = registry_fingerprint(&config.files);

    if fingerprint_is_empty(&expected_fp) {
        // No hashes registered — skip verification, mark as done.
        let _ = app.emit(
            "download-progress",
            DownloadProgressPayload {
                model_id: model_id.to_string(),
                total_bytes: 100,
                downloaded_bytes: 100,
                status: "done".to_string(),
                current_file: files_count as u32,
                total_files: files_count as u32,
            },
        );
        return Ok(format!("Downloaded to {:?}", base_dir));
    }

    // Pre-calculate total bytes to verify so we can report real progress.
    // Directories (e.g. .mlmodelc CoreML bundles) are skipped — they can't
    // be meaningfully byte-hashed and are trusted after successful extraction.
    let mut total_verify_bytes: u64 = 0;
    for file_spec in &config.files {
        if !file_spec.sha1.is_empty() {
            let file_path = base_dir.join(file_spec.filename);
            if file_path.is_dir() {
                continue;
            }
            if let Ok(meta) = std::fs::metadata(&file_path) {
                total_verify_bytes += meta.len();
            }
        }
    }

    // Hash each file and build the actual fingerprint.
    let mut computed_fp_parts: Vec<String> = Vec::new();
    let mut verified_bytes: u64 = 0;
    let emit_threshold: u64 = 512 * 1024; // emit every 512 KiB

    for (i, file_spec) in config.files.iter().enumerate() {
        // Skip files without a registered hash.
        if file_spec.sha1.is_empty() {
            computed_fp_parts.push(String::new());
            continue;
        }

        let file_path = base_dir.join(file_spec.filename);

        println!(
            "[VERIFY] SHA256 for {} ({}/{})...",
            file_spec.filename,
            i + 1,
            files_count
        );

        // Emit initial progress for this file.
        let _ = app.emit(
            "download-progress",
            DownloadProgressPayload {
                model_id: model_id.to_string(),
                total_bytes: total_verify_bytes,
                downloaded_bytes: verified_bytes,
                status: "verifying".to_string(),
                current_file: (i + 1) as u32,
                total_files: files_count as u32,
            },
        );

        // Directories (e.g. .mlmodelc CoreML bundles) can't be file-hashed.
        // Trust the extraction; push the expected hash so the fingerprint matches.
        if file_path.is_dir() {
            computed_fp_parts.push(file_spec.sha1.to_string());
            continue;
        }

        let mut file = File::open(&file_path).map_err(|e| {
            emit_download_error_and_cleanup(
                app,
                model_id,
                &config,
                &base_dir,
                (i + 1) as u32,
                files_count as u32,
                &format!("Download failed — could not read file for verification ({})", e),
            )
        })?;
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 65536]; // 64 KiB chunks for speed
        let mut last_emit: u64 = verified_bytes;

        loop {
            let count = file.read(&mut buffer).map_err(|e| {
                emit_download_error_and_cleanup(
                    app,
                    model_id,
                    &config,
                    &base_dir,
                    (i + 1) as u32,
                    files_count as u32,
                    &format!("Download failed — verification read error ({})", e),
                )
            })?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
            verified_bytes += count as u64;

            if verified_bytes - last_emit > emit_threshold {
                last_emit = verified_bytes;
                let _ = app.emit(
                    "download-progress",
                    DownloadProgressPayload {
                        model_id: model_id.to_string(),
                        total_bytes: total_verify_bytes,
                        downloaded_bytes: verified_bytes,
                        status: "verifying".to_string(),
                        current_file: (i + 1) as u32,
                        total_files: files_count as u32,
                    },
                );
            }
        }

        // Final emit for this file at 100% of bytes processed so far.
        let _ = app.emit(
            "download-progress",
            DownloadProgressPayload {
                model_id: model_id.to_string(),
                total_bytes: total_verify_bytes,
                downloaded_bytes: verified_bytes,
                status: "verifying".to_string(),
                current_file: (i + 1) as u32,
                total_files: files_count as u32,
            },
        );

        let hash_hex = hex::encode(hasher.finalize());
        println!(
            "[VERIFY] {} — Expected: {}, Got: {}",
            file_spec.filename, file_spec.sha1, hash_hex
        );

        if hash_hex != file_spec.sha1 {
            eprintln!("[VERIFY] Hash mismatch! Deleting corrupted files.");
            let msg = format!(
                "Download failed — file may be corrupted ({}). Try again.",
                file_spec.filename
            );
            return Err(emit_download_error_and_cleanup(
                app,
                model_id,
                &config,
                &base_dir,
                (i + 1) as u32,
                files_count as u32,
                &msg,
            ));
        }

        computed_fp_parts.push(hash_hex);
    }

    // All hashes matched — write to verified.json.
    let computed_fp = computed_fp_parts.join("+");
    let now = chrono::Utc::now().to_rfc3339();

    let mut store = load_verified_store();
    store.insert(
        model_id.to_string(),
        VerifiedEntry {
            fingerprint: computed_fp,
            verified_at: now,
        },
    );
    save_verified_store(&store);

    println!("[VERIFY] {} — all files verified ✅", model_id);

    let _ = app.emit(
        "download-progress",
        DownloadProgressPayload {
            model_id: model_id.to_string(),
            total_bytes: 100,
            downloaded_bytes: 100,
            status: "done".to_string(),
            current_file: files_count as u32,
            total_files: files_count as u32,
        },
    );

    Ok(format!("Downloaded and verified {:?}", base_dir))
}

#[tauri::command]
pub async fn delete_model(app: AppHandle, model_id: String) -> Result<String, String> {
    let config =
        get_model_config(&model_id).ok_or_else(|| format!("Unknown model ID: {}", model_id))?;
    let models_dir =
        crate::utils::get_models_dir().map_err(|e| format!("Failed to get models dir: {}", e))?;

    let base_dir = if let Some(subdir) = config.subdirectory {
        models_dir.join(subdir)
    } else {
        models_dir.clone()
    };

    // Pre-calculate total size for progress reporting.
    let mut total_size: u64 = 0;
    for file_spec in &config.files {
        let file_path = base_dir.join(file_spec.filename);
        if file_path.exists() {
            if let Ok(meta) = std::fs::metadata(&file_path) {
                total_size += meta.len();
            }
        }
    }

    let files_count = config.files.len() as u32;
    let mut deleted_bytes: u64 = 0;

    for (i, file_spec) in config.files.iter().enumerate() {
        // Emit progress before each file deletion.
        let _ = app.emit(
            "download-progress",
            DownloadProgressPayload {
                model_id: model_id.clone(),
                total_bytes: total_size,
                downloaded_bytes: deleted_bytes,
                status: "deleting".to_string(),
                current_file: (i + 1) as u32,
                total_files: files_count,
            },
        );

        let file_path = base_dir.join(file_spec.filename);
        if file_path.exists() {
            let size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
            if file_path.is_dir() {
                let _ = std::fs::remove_dir_all(&file_path);
            } else {
                let _ = std::fs::remove_file(&file_path);
            }
            deleted_bytes += size;
        }
    }

    if config.subdirectory.is_some() {
        let _ = std::fs::remove_dir(&base_dir);
    }

    // Remove verification record.
    let mut store = load_verified_store();
    store.remove(&model_id);
    save_verified_store(&store);

    // Emit final progress so frontend can clean up.
    let _ = app.emit(
        "download-progress",
        DownloadProgressPayload {
            model_id: model_id.clone(),
            total_bytes: total_size,
            downloaded_bytes: total_size,
            status: "delete-done".to_string(),
            current_file: files_count,
            total_files: files_count,
        },
    );

    Ok(format!("Deleted model {}", model_id))
}
