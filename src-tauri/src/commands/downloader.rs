use crate::types::CommandResult;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use zip::ZipArchive;

// ── Cancellation registry ─────────────────────────────────────────────────────

struct DownloadCancelState {
    flag: Arc<AtomicBool>,
    finishing: bool,
}

static CANCEL_FLAGS: OnceLock<Mutex<HashMap<String, DownloadCancelState>>> = OnceLock::new();

fn cancel_flags() -> &'static Mutex<HashMap<String, DownloadCancelState>> {
    CANCEL_FLAGS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_cancel_flag(model_id: &str) -> Result<Arc<AtomicBool>, String> {
    let flag = Arc::new(AtomicBool::new(false));
    let mut flags = cancel_flags().lock().unwrap();
    if flags.contains_key(model_id) {
        return Err(format!("A download is already active for {}", model_id));
    }
    flags.insert(model_id.to_string(), DownloadCancelState {
        flag: Arc::clone(&flag),
        finishing: false,
    });
    Ok(flag)
}

fn begin_finish_cancel_flag(model_id: &str, flag: &Arc<AtomicBool>) -> bool {
    let mut flags = cancel_flags().lock().unwrap();
    if let Some(active) = flags.get_mut(model_id) {
        if Arc::ptr_eq(&active.flag, flag) {
            active.finishing = true;
            return flag.load(Ordering::Relaxed);
        }
    }
    false
}

fn unregister_cancel_flag(model_id: &str, flag: &Arc<AtomicBool>) {
    let mut flags = cancel_flags().lock().unwrap();
    if flags.get(model_id).is_some_and(|active| Arc::ptr_eq(&active.flag, flag)) {
        flags.remove(model_id);
    }
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

// ── Download lock files ───────────────────────────────────────────────────────
//
// A `<model_id>.downloading` sentinel is written into the models directory at
// the very start of every download and removed when the download finishes
// (success, network error, hash mismatch, or user cancellation).
//
// If the app is force-quit or crashes while a download is running the sentinel
// is never removed, so on the next launch `scan_and_clean_stale_downloads`
// finds it and wipes the orphaned partial files before the UI is shown.

fn lock_file_path(model_id: &str) -> Option<std::path::PathBuf> {
    crate::utils::get_models_dir()
        .ok()
        .map(|d| d.join(format!("{}.downloading", model_id)))
}

fn write_download_lock(model_id: &str) {
    if let Some(path) = lock_file_path(model_id) {
        let _ = std::fs::write(&path, model_id);
    }
}

fn remove_download_lock(model_id: &str) {
    if let Some(path) = lock_file_path(model_id) {
        let _ = std::fs::remove_file(&path);
    }
}

/// Called once at app startup.  Finds any leftover `*.downloading` sentinel
/// files (from a crash or force-quit during a previous download), wipes the
/// associated partial model files, and removes the sentinel.
pub fn scan_and_clean_stale_downloads() {
    let models_dir = match crate::utils::get_models_dir() {
        Ok(d) => d,
        Err(_) => return,
    };
    let entries = match std::fs::read_dir(&models_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !name.ends_with(".downloading") {
            continue;
        }
        let model_id = name.trim_end_matches(".downloading");
        println!(
            "[STARTUP] Stale download lock found for '{}' — cleaning up orphaned files",
            model_id
        );
        if let Some(config) = super::model_registry::get_model_config(model_id) {
            let base_dir = if let Some(subdir) = config.subdirectory {
                models_dir.join(subdir)
            } else {
                models_dir.clone()
            };
            delete_model_files(&config, &base_dir);
            // Remove the stale verified.json entry if any
            let mut store = load_verified_store();
            if store.remove(model_id).is_some() {
                save_verified_store(&store);
            }
        }
        let _ = std::fs::remove_file(&path);
        println!("[STARTUP] Cleaned up stale download for '{}'", model_id);
    }
}

/// Signal an in-progress download to stop. The worker owns file cleanup.
#[tauri::command]
pub async fn cancel_download(model_id: String) -> Result<bool, String> {
    if let Some(active) = cancel_flags().lock().unwrap().get(&model_id) {
        if !active.finishing {
            active.flag.store(true, Ordering::Relaxed);
            return Ok(true);
        }
    }
    Ok(false)
}

/// True while any model download is running.
pub fn any_download_active() -> bool {
    !cancel_flags().lock().unwrap().is_empty()
}

/// Cancel all in-progress downloads. Called before factory reset so tasks get
/// a clean cancellation signal before the process is killed.
pub fn cancel_all_downloads() {
    for active in cancel_flags().lock().unwrap().values() {
        if !active.finishing {
            active.flag.store(true, Ordering::Relaxed);
        }
    }
}

use super::model_registry::{get_model_config, ModelConfig, ModelFile};

enum ModelSource<'a> {
    HuggingFace,
    GitHub(&'a str),
    /// A GitHub release asset: `config.branch` holds the release tag.
    GitHubRelease(&'a str),
    Local(std::path::PathBuf),
}

fn model_source(config: &ModelConfig) -> Result<ModelSource<'_>, String> {
    if let Some(repo_path) = config.repo.strip_prefix("github-release:") {
        return Ok(ModelSource::GitHubRelease(repo_path));
    }
    if let Some(repo_path) = config.repo.strip_prefix("github:") {
        return Ok(ModelSource::GitHub(repo_path));
    }
    if let Some(local_name) = config.repo.strip_prefix("local:") {
        let root = match std::env::var("TAURSCRIBE_LOCAL_MODEL_SOURCE_DIR") {
            Ok(path) if !path.trim().is_empty() => std::path::PathBuf::from(path),
            _ => {
                let appdata = std::env::var("LOCALAPPDATA")
                    .map_err(|_| "LOCALAPPDATA is not set; set TAURSCRIBE_LOCAL_MODEL_SOURCE_DIR for local model downloads".to_string())?;
                std::path::PathBuf::from(appdata)
                    .join("Taurscribe")
                    .join("local-model-sources")
            }
        };
        let source = root.join(local_name);
        if source.exists() {
            return Ok(ModelSource::Local(source));
        }
        if let Ok(models_dir) = crate::utils::get_models_dir() {
            let existing_model_dir = models_dir.join(local_name);
            if existing_model_dir.exists() {
                return Ok(ModelSource::Local(existing_model_dir));
            }
        }
        return Ok(ModelSource::Local(source));
    }
    Ok(ModelSource::HuggingFace)
}

fn copy_local_model_file(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> Result<u64, String> {
    if source.is_dir() {
        if destination.exists() {
            std::fs::remove_dir_all(destination)
                .map_err(|e| format!("Failed to replace local model directory: {}", e))?;
        }
        std::fs::create_dir_all(destination)
            .map_err(|e| format!("Failed to create local model directory: {}", e))?;
        let mut copied = 0u64;
        for entry in std::fs::read_dir(source)
            .map_err(|e| format!("Failed to read local model directory: {}", e))?
        {
            let entry = entry.map_err(|e| format!("Failed to read local model entry: {}", e))?;
            copied += copy_local_model_file(&entry.path(), &destination.join(entry.file_name()))?;
        }
        return Ok(copied);
    }

    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create local model parent directory: {}", e))?;
    }
    std::fs::copy(source, destination).map_err(|e| {
        format!(
            "Failed to copy local model file {}: {}",
            source.display(),
            e
        )
    })
}

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

/// Files that actually participate in download / status / verification on
/// this build. The CoreML ANE companion bundle (`.mlmodelc.zip`) is only
/// usable on Apple Silicon — everywhere else it is skipped so users don't
/// download hundreds of MB they can never execute.
fn active_files<'a>(config: &'a ModelConfig) -> Vec<&'a ModelFile> {
    use super::model_registry::{coreml_companion_supported, is_coreml_bundle_file};
    let include_coreml = coreml_companion_supported();
    config
        .files
        .iter()
        .filter(|f| include_coreml || !is_coreml_bundle_file(f.filename, f.remote_path))
        .collect()
}

/// Fingerprint over exactly the files in `active_files`.
fn active_fingerprint(active: &[&ModelFile]) -> String {
    active
        .iter()
        .map(|f| f.sha1) // sha1 field now holds SHA-256
        .collect::<Vec<_>>()
        .join("+")
}

/// Returns true if all hashes in the fingerprint are empty (verification disabled).
fn fingerprint_is_empty(fp: &str) -> bool {
    fp.split('+').all(|h| h.is_empty())
}

/// Verify an existing installation against the registry without downloading it
/// again. This migrates models installed before verified.json receipts existed.
fn verify_installed_model_files(
    active: &[&ModelFile],
    base_dir: &std::path::Path,
) -> Result<String, String> {
    use sha2::{Digest, Sha256};

    let mut fingerprint_parts = Vec::with_capacity(active.len());
    let mut buffer = [0u8; 65536];

    for file_spec in active {
        let expected_hash = file_spec.sha1;
        if expected_hash.is_empty() {
            fingerprint_parts.push(String::new());
            continue;
        }

        let file_path = base_dir.join(file_spec.filename);
        if file_path.is_dir() {
            fingerprint_parts.push(expected_hash.to_string());
            continue;
        }

        let mut file = File::open(&file_path)
            .map_err(|e| format!("Could not open {}: {e}", file_path.display()))?;
        let mut hasher = Sha256::new();
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|e| format!("Could not read {}: {e}", file_path.display()))?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        let actual_hash = hex::encode(hasher.finalize());
        if actual_hash != expected_hash {
            return Err(format!(
                "SHA-256 mismatch for {}: expected {}, got {}",
                file_spec.filename, expected_hash, actual_hash
            ));
        }
        fingerprint_parts.push(actual_hash);
    }

    Ok(fingerprint_parts.join("+"))
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
    let mut store = load_verified_store();
    let mut store_changed = false;

    let mut statuses = Vec::new();

    for id in model_ids {
        if let Some(config) = get_model_config(&id) {
            let base_dir = if let Some(subdir) = config.subdirectory {
                models_dir.join(subdir)
            } else {
                models_dir.clone()
            };

            // If download is currently in progress or actively being cancelled,
            // the model is definitely not downloaded.
            let is_downloading = lock_file_path(&id).map(|p| p.exists()).unwrap_or(false)
                || cancel_flags().lock().unwrap().contains_key(&id);

            // Only the files relevant on this platform count (the ANE
            // companion is Apple-Silicon-only and skipped elsewhere).
            let active = active_files(&config);

            // Check all files exist on disk and sum their sizes.
            let mut all_exist = !is_downloading;
            let mut total_size: u64 = 0;

            if !is_downloading {
                for file_spec in &active {
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
            }

            let downloaded = all_exist && total_size > 0;

            // Trust a current receipt. Older installs and stale receipts are
            // rehashed once against the pinned registry values, then migrated.
            let verified = if !downloaded {
                false
            } else {
                let expected_fp = active_fingerprint(&active);
                if fingerprint_is_empty(&expected_fp) {
                    true
                } else if store
                    .get(&id)
                    .is_some_and(|entry| entry.fingerprint == expected_fp)
                {
                    true
                } else {
                    match verify_installed_model_files(&active, &base_dir) {
                        Ok(computed_fp) if computed_fp == expected_fp => {
                            println!("[VERIFY] Migrated existing verified model: {id}");
                            store.insert(
                                id.clone(),
                                VerifiedEntry {
                                    fingerprint: computed_fp,
                                    verified_at: chrono::Utc::now().to_rfc3339(),
                                },
                            );
                            store_changed = true;
                            true
                        }
                        Ok(_) => false,
                        Err(err) => {
                            eprintln!("[VERIFY] Existing model {id} is unverified: {err}");
                            false
                        }
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

    if store_changed {
        save_verified_store(&store);
    }

    Ok(statuses)
}

#[tauri::command]
pub async fn download_model(app: AppHandle, model_id: String) -> Result<String, String> {
    let cancel_flag = register_cancel_flag(&model_id)?;
    write_download_lock(&model_id);
    let result = download_model_inner(&app, &model_id, &cancel_flag).await;
    let cancelled = begin_finish_cancel_flag(&model_id, &cancel_flag);
    if cancelled {
        if let (Some(config), Ok(models_dir)) =
            (get_model_config(&model_id), crate::utils::get_models_dir())
        {
            let base_dir = config.subdirectory.map_or(models_dir.clone(), |subdir| models_dir.join(subdir));
            delete_model_files(&config, &base_dir);
            let mut store = load_verified_store();
            if store.remove(&model_id).is_some() {
                save_verified_store(&store);
            }
        }
    }
    remove_download_lock(&model_id);
    let status = if cancelled { "cancelled" } else if result.is_ok() { "done" } else { "error" };
    if cancelled || result.is_ok() {
        let _ = app.emit("download-progress", DownloadProgressPayload {
            model_id: model_id.clone(),
            total_bytes: if cancelled { 0 } else { 100 },
            downloaded_bytes: if cancelled { 0 } else { 100 },
            status: status.to_string(),
            current_file: 0,
            total_files: 0,
        });
    }
    unregister_cancel_flag(&model_id, &cancel_flag);
    if cancelled { Err("Download cancelled by user".to_string()) } else { result }
}

/// Fetches the LFS pointer for a HuggingFace file and returns its SHA-256 hash.
/// Returns None if the fetch fails or the response is not an LFS pointer.
async fn fetch_hf_lfs_sha256(
    client: &Client,
    repo: &str,
    branch: &str,
    remote_path: &str,
) -> Option<String> {
    let url = format!(
        "https://huggingface.co/{}/raw/{}/{}",
        repo, branch, remote_path
    );
    let res = client.get(&url).send().await.ok()?;
    if !res.status().is_success() {
        return None;
    }
    let text = res.text().await.ok()?;
    for line in text.lines() {
        if let Some(hash) = line.strip_prefix("oid sha256:") {
            let hash = hash.trim();
            if hash.len() == 64 {
                return Some(hash.to_string());
            }
        }
    }
    None
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

    // Clean up any orphaned files from a previous partial/crashed download before
    // starting fresh.  This covers the case where the app was force-quit after
    // some files had already been written but before the lock was removed.
    delete_model_files(&config, &base_dir);
    // Also clear any stale verified.json entry so the UI won't flash "Verified"
    // for a fraction of a second before the new download completes.
    {
        let mut store = load_verified_store();
        if store.remove(model_id).is_some() {
            save_verified_store(&store);
        }
    }

    if !base_dir.exists() {
        std::fs::create_dir_all(&base_dir)
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }

    // Platform-active files only: the ANE companion is skipped off Apple
    // Silicon so other platforms never fetch an unusable ~1 GB bundle.
    let active = active_files(&config);
    let files_count = active.len();
    let source = model_source(&config)?;
    let is_hf_repo = matches!(source, ModelSource::HuggingFace);

    let client = Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    // ── Download phase ────────────────────────────────────────────────────────
    for (i, file_spec) in active.iter().enumerate() {
        let is_zip = file_spec.remote_path.ends_with(".zip");
        let download_path = if is_zip {
            base_dir.join(format!("{}.zip", file_spec.filename))
        } else {
            base_dir.join(file_spec.filename)
        };

        let emit_error =
            |app: &AppHandle, model_id: &str, i: usize, files_count: usize, msg: &str| {
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

        if let ModelSource::Local(local_dir) = &source {
            let source_path = local_dir.join(file_spec.remote_path);
            println!(
                "[DOWNLOAD] {} ({}/{}) from local {}",
                model_id,
                i + 1,
                files_count,
                source_path.display()
            );
            if !source_path.exists() {
                return Err(emit_error(
                    app,
                    model_id,
                    i,
                    files_count,
                    &format!("Local model file not found: {}", source_path.display()),
                ));
            }
            let copied = copy_local_model_file(&source_path, &download_path)
                .map_err(|e| emit_error(app, model_id, i, files_count, &e))?;
            let _ = app.emit(
                "download-progress",
                DownloadProgressPayload {
                    model_id: model_id.to_string(),
                    total_bytes: copied,
                    downloaded_bytes: copied,
                    status: "downloading".to_string(),
                    current_file: (i + 1) as u32,
                    total_files: files_count as u32,
                },
            );
            if cancel_flag.load(Ordering::Relaxed) {
                delete_model_files(&config, &base_dir);
                return Err("Download cancelled by user".to_string());
            }
        } else {
            let url = match &source {
                ModelSource::GitHub(repo_path) => format!(
                    "https://raw.githubusercontent.com/{}/{}/{}",
                    repo_path, config.branch, file_spec.remote_path
                ),
                ModelSource::GitHubRelease(repo_path) => format!(
                    "https://github.com/{}/releases/download/{}/{}",
                    repo_path, config.branch, file_spec.remote_path
                ),
                ModelSource::HuggingFace => format!(
                    "https://huggingface.co/{}/resolve/{}/{}",
                    config.repo, config.branch, file_spec.remote_path
                ),
                ModelSource::Local(_) => unreachable!(),
            };

            println!(
                "[DOWNLOAD] {} ({}/{}) from {}",
                model_id,
                i + 1,
                files_count,
                url
            );

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
                    app,
                    model_id,
                    i,
                    files_count,
                    &format!("Download server returned HTTP {}", res.status()),
                ));
            }

            let total_size = res.content_length().unwrap_or(0);
            let mut file = File::create(&download_path)
                .map_err(|e| format!("Failed to create file: {}", e))?;

            let mut downloaded: u64 = 0;
            let mut stream = res.bytes_stream();
            let mut last_emit: u64 = 0;
            let emit_threshold = 1024 * 1024; // 1 MB

            while let Some(item) = stream.next().await {
                // Check for user cancellation on every chunk immediately
                if cancel_flag.load(Ordering::Relaxed) {
                    drop(file);
                    let _ = std::fs::remove_file(&download_path);
                    delete_model_files(&config, &base_dir);
                    return Err("Download cancelled by user".to_string());
                }

                let chunk = match item {
                    Ok(c) => c,
                    Err(e) => {
                        drop(file);
                        let _ = std::fs::remove_file(&download_path);
                        let reason = if e.is_timeout() {
                            "Connection lost — no data received for 30 seconds. Check your internet and try again."
                        } else if e.is_connect()
                            || e.to_string().contains("reset")
                            || e.to_string().contains("connection")
                        {
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
                }
            }
            drop(file);
        }

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
                        std::fs::create_dir_all(parent)
                            .map_err(|e| zip_fail(format!("Failed to create parent dir: {}", e)))?;
                    }
                    let mut out_file = File::create(&outpath)
                        .map_err(|e| zip_fail(format!("Failed to create extracted file: {}", e)))?;
                    std::io::copy(&mut entry, &mut out_file)
                        .map_err(|e| zip_fail(format!("Failed to write extracted file: {}", e)))?;
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
                    return Err("Download cancelled by user".to_string());
                }
            }

            std::fs::remove_file(&download_path).ok();
            println!("[DOWNLOAD] Extraction complete.");
        }
    }

    println!("[DOWNLOAD] Finished downloading {}", model_id);

    // ── Auto-verify phase ─────────────────────────────────────────────────────
    let expected_fp = active_fingerprint(&active);

    // Only skip verification entirely for non-HuggingFace repos with no hashes.
    // HuggingFace entries without pinned hashes use live LFS pointer metadata.
    if fingerprint_is_empty(&expected_fp) && !is_hf_repo {
        return Ok(format!("Downloaded to {:?}", base_dir));
    }

    // Pre-calculate total bytes for progress reporting (all non-directory files).
    let mut total_verify_bytes: u64 = 0;
    for file_spec in &active {
        let file_path = base_dir.join(file_spec.filename);
        if file_path.is_dir() {
            continue;
        }
        if let Ok(meta) = std::fs::metadata(&file_path) {
            total_verify_bytes += meta.len();
        }
    }

    // Hash each file and build the actual fingerprint.
    let mut computed_fp_parts: Vec<String> = Vec::new();
    let mut verified_bytes: u64 = 0;
    let emit_threshold: u64 = 512 * 1024; // emit every 512 KiB

    for (i, file_spec) in active.iter().enumerate() {
        // Pinned registry hashes are authoritative. Only unpinned HuggingFace
        // files fall back to live LFS metadata.
        let expected_hash: String = if !file_spec.sha1.is_empty() {
            file_spec.sha1.to_string()
        } else if is_hf_repo {
            match fetch_hf_lfs_sha256(&client, config.repo, config.branch, file_spec.remote_path)
                .await
            {
                Some(h) => h,
                None => String::new(),
            }
        } else {
            String::new()
        };

        if expected_hash.is_empty() {
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
            computed_fp_parts.push(expected_hash);
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
                &format!(
                    "Download failed — could not read file for verification ({})",
                    e
                ),
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
            file_spec.filename, expected_hash, hash_hex
        );

        if hash_hex != expected_hash {
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

    Ok(format!("Downloaded and verified {:?}", base_dir))
}

fn encoder_has_other_weights(
    models_dir: &std::path::Path,
    encoder_name: &str,
    deleting: &ModelConfig,
) -> Result<bool, String> {
    let base = encoder_name.trim_end_matches("-encoder.mlmodelc");
    let entries = std::fs::read_dir(models_dir)
        .map_err(|e| format!("Could not inspect models directory: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let is_variant = name == format!("{base}.bin")
            || (name.starts_with(&format!("{base}-q")) && name.ends_with(".bin"));
        if is_variant && !deleting.files.iter().any(|f| f.filename == name) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn encoder_has_standalone_owner(encoder_name: &str, deleting_id: &str) -> bool {
    load_verified_store().keys().any(|id| {
        id != deleting_id
            && id.ends_with("-coreml")
            && get_model_config(id).is_some_and(|config| {
                config.files.iter().any(|file| file.filename == encoder_name)
            })
    })
}

#[tauri::command]
pub async fn delete_model(
    app: AppHandle,
    model_id: String,
) -> Result<CommandResult<String>, String> {
    let config = match get_model_config(&model_id) {
        Some(config) => config,
        None => {
            return Ok(CommandResult::err(
                "model_missing",
                format!("Unknown model ID: {}", model_id),
            ))
        }
    };
    let models_dir =
        crate::utils::get_models_dir().map_err(|e| format!("Failed to get models dir: {}", e))?;

    let base_dir = if let Some(subdir) = config.subdirectory {
        models_dir.join(subdir)
    } else {
        models_dir.clone()
    };

    // A bundled encoder is shared by full and quantized weights. A standalone
    // encoder cannot be removed while any installed weights still reference it.
    if model_id.ends_with("-coreml") {
        if let Some(encoder) = config.files.iter().find(|f| f.filename.ends_with("-encoder.mlmodelc")) {
            if encoder_has_other_weights(&models_dir, encoder.filename, &config)? {
                return Ok(CommandResult::err("model_in_use", "Other installed Whisper models use this CoreML encoder"));
            }
        }
    }

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
            if file_spec.filename.ends_with("-encoder.mlmodelc")
                && (encoder_has_other_weights(&models_dir, file_spec.filename, &config)?
                    || encoder_has_standalone_owner(file_spec.filename, &model_id))
            {
                continue;
            }
            let size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
            if file_path.is_dir() {
                std::fs::remove_dir_all(&file_path)
                    .map_err(|e| format!("Could not delete {}: {e}", file_path.display()))?;
            } else {
                std::fs::remove_file(&file_path)
                    .map_err(|e| format!("Could not delete {}: {e}", file_path.display()))?;
            }
            deleted_bytes += size;
        }
    }

    if config.subdirectory.is_some() {
        if base_dir.exists() {
            std::fs::remove_dir(&base_dir)
                .map_err(|e| format!("Could not delete {}: {e}", base_dir.display()))?;
        }
    }

    // Verification of other models is independent, including shared encoders.
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

    Ok(CommandResult::ok(format!("Deleted model {}", model_id)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::model_registry::ModelFile;

    #[test]
    fn encoder_owner_check_respects_installed_quantized_weights() {
        let dir = std::env::temp_dir().join(format!("taurscribe-encoder-owners-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ggml-small.en.bin"), b"full").unwrap();
        std::fs::write(dir.join("ggml-small.en-q5_1.bin"), b"quantized").unwrap();
        let full = get_model_config("whisper-small-en").unwrap();
        assert!(encoder_has_other_weights(&dir, "ggml-small.en-encoder.mlmodelc", &full).unwrap());
        std::fs::remove_file(dir.join("ggml-small.en-q5_1.bin")).unwrap();
        assert!(!encoder_has_other_weights(&dir, "ggml-small.en-encoder.mlmodelc", &full).unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn download_cancellation_is_scoped_to_an_active_worker() {
        let id = format!("cancel-test-{}", rand::random::<u64>());
        assert!(!cancel_download(id.clone()).await.unwrap());
        let first = register_cancel_flag(&id).unwrap();
        assert!(register_cancel_flag(&id).is_err());
        assert!(cancel_download(id.clone()).await.unwrap());
        assert!(begin_finish_cancel_flag(&id, &first));
        assert!(cancel_flags().lock().unwrap().get(&id).unwrap().finishing);
        assert!(!cancel_download(id.clone()).await.unwrap());
        unregister_cancel_flag(&id, &first);
        assert!(!cancel_download(id.clone()).await.unwrap());

        let second = register_cancel_flag(&id).unwrap();
        assert!(!second.load(Ordering::Relaxed));
        assert!(!begin_finish_cancel_flag(&id, &second));
        unregister_cancel_flag(&id, &second);
    }

    #[test]
    fn existing_install_verification_accepts_match_and_rejects_mismatch() {
        let dir =
            std::env::temp_dir().join(format!("taurscribe-verify-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create verification test dir");
        std::fs::write(dir.join("model.bin"), b"hello").expect("write verification fixture");

        let valid = ModelConfig {
            repo: "test/repo",
            branch: "main",
            files: vec![ModelFile {
                filename: "model.bin",
                remote_path: "model.bin",
                sha1: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
            }],
            subdirectory: Some("test"),
        };
        let valid_active: Vec<&ModelFile> = valid.files.iter().collect();
        assert_eq!(
            verify_installed_model_files(&valid_active, &dir).expect("matching SHA-256"),
            valid.files[0].sha1
        );

        let invalid = ModelConfig {
            repo: valid.repo,
            branch: valid.branch,
            files: vec![ModelFile {
                filename: "model.bin",
                remote_path: "model.bin",
                sha1: "0000000000000000000000000000000000000000000000000000000000000000",
            }],
            subdirectory: valid.subdirectory,
        };
        let invalid_active: Vec<&ModelFile> = invalid.files.iter().collect();
        assert!(verify_installed_model_files(&invalid_active, &dir).is_err());

        let _ = std::fs::remove_dir_all(dir);
    }
}
