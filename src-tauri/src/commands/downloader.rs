use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Write};
use tauri::{AppHandle, Emitter};
use zip::ZipArchive;

use super::model_registry::get_model_config;

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

#[tauri::command]
pub async fn verify_model_hash(app: AppHandle, model_id: String) -> Result<bool, String> {
    let config =
        get_model_config(&model_id).ok_or_else(|| format!("Unknown model ID: {}", model_id))?;

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

    if verified_count > 0 {
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

            let verified_marker = base_dir.join(format!("{}.verified", config.files[0].filename));
            let mut verified = verified_marker.exists();

            let mut all_exist = true;
            let mut total_size: u64 = 0;

            for file_spec in &config.files {
                let file_path = base_dir.join(file_spec.filename);
                if file_path.exists() {
                    if file_path.is_dir() {
                        // CoreML .mlmodelc directories — mark as present with size 1
                        total_size += 1;
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

    for file_spec in &config.files {
        let file_path = base_dir.join(file_spec.filename);
        if file_path.exists() {
            if file_path.is_dir() {
                let _ = std::fs::remove_dir_all(&file_path);
            } else {
                let _ = std::fs::remove_file(&file_path);
            }
        }
    }

    let verified_marker = base_dir.join(format!("{}.verified", config.files[0].filename));
    if verified_marker.exists() {
        let _ = std::fs::remove_file(&verified_marker);
    }

    if config.subdirectory.is_some() {
        let _ = std::fs::remove_dir(&base_dir);
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

        // For zip files (e.g. CoreML encoders), download to a temp .zip path then extract.
        let is_zip = file_spec.remote_path.ends_with(".zip");
        let download_path = if is_zip {
            base_dir.join(format!("{}.zip", file_spec.filename))
        } else {
            base_dir.join(file_spec.filename)
        };
        let target_path = download_path.clone();

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
        let emit_threshold = 1024 * 1024; // 1 MB

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
                        status: "downloading".to_string(),
                        current_file: (i + 1) as u32,
                        total_files: files_count as u32,
                    },
                );
            }
        }
        drop(file); // close file handle before reading it for zip extraction

        // If the downloaded file is a zip (e.g. CoreML encoder), extract it then remove the zip.
        if is_zip {
            println!("[DOWNLOAD] Extracting zip: {:?}", download_path);
            let zip_file = File::open(&download_path)
                .map_err(|e| format!("Failed to open zip for extraction: {}", e))?;
            let mut archive = ZipArchive::new(zip_file)
                .map_err(|e| format!("Failed to read zip archive: {}", e))?;
            archive
                .extract(&base_dir)
                .map_err(|e| format!("Failed to extract zip: {}", e))?;
            std::fs::remove_file(&download_path).ok();
            println!("[DOWNLOAD] Extraction complete, zip removed.");
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
