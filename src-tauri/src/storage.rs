//! Where Taurscribe keeps its large files.
//!
//! Two areas can live outside the app data folder (for example on an external
//! drive): downloaded **models** and **recordings** (meeting audio, speaker
//! snippets, continuation buffers). The choice is stored in
//! `<data_local>/Taurscribe/storage.json`; the history database, settings and
//! temporary dictation audio always stay in the app data folder.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use std::time::Instant;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Area {
    Models,
    Recordings,
}

impl Area {
    fn default_leaf(self) -> &'static str {
        match self {
            Area::Models => "models",
            Area::Recordings => "meetings",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StorageConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    models_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recordings_dir: Option<PathBuf>,
}

impl StorageConfig {
    fn get(&self, area: Area) -> Option<&PathBuf> {
        match area {
            Area::Models => self.models_dir.as_ref(),
            Area::Recordings => self.recordings_dir.as_ref(),
        }
    }
    fn set(&mut self, area: Area, dir: Option<PathBuf>) {
        match area {
            Area::Models => self.models_dir = dir,
            Area::Recordings => self.recordings_dir = dir,
        }
    }
}

/// `<data_local>/Taurscribe` — the app data folder.
pub fn app_data_root() -> Result<PathBuf, String> {
    Ok(dirs::data_local_dir()
        .ok_or("Could not find AppData directory")?
        .join("Taurscribe"))
}

fn config_path() -> Result<PathBuf, String> {
    Ok(app_data_root()?.join("storage.json"))
}

fn config() -> &'static RwLock<Option<StorageConfig>> {
    static CONFIG: OnceLock<RwLock<Option<StorageConfig>>> = OnceLock::new();
    CONFIG.get_or_init(|| RwLock::new(None))
}

fn load_config() -> StorageConfig {
    if let Some(cfg) = config().read().unwrap().as_ref() {
        return cfg.clone();
    }
    let cfg = config_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<StorageConfig>(&s).ok())
        .unwrap_or_default();
    *config().write().unwrap() = Some(cfg.clone());
    cfg
}

fn save_config(cfg: &StorageConfig) -> Result<(), String> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("Could not save storage settings: {e}"))?;
    *config().write().unwrap() = Some(cfg.clone());
    Ok(())
}

pub fn default_dir(area: Area) -> Result<PathBuf, String> {
    Ok(app_data_root()?.join(area.default_leaf()))
}

/// The folder currently used for `area` (custom if set, else the default). Not created.
pub fn current_dir(area: Area) -> Result<PathBuf, String> {
    match load_config().get(area) {
        Some(dir) => Ok(dir.clone()),
        None => default_dir(area),
    }
}

/// The folder for `area`, created if needed. A custom folder whose drive is
/// missing gives a clear error instead of silently creating a new folder.
pub fn resolve_dir(area: Area) -> Result<PathBuf, String> {
    let dir = current_dir(area)?;
    let is_custom = load_config().get(area).is_some();
    if is_custom && !dir.exists() && !drive_present(&dir) {
        return Err(format!(
            "The {} folder {} isn't available. Is the drive connected?",
            match area {
                Area::Models => "models",
                Area::Recordings => "recordings",
            },
            dir.display()
        ));
    }
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;
    Ok(dir)
}

/// True when the volume that would hold `path` is mounted.
fn drive_present(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        // /Volumes/<Name>/... → the volume root must exist.
        let mut comps = path.components();
        let first_two: PathBuf = comps.by_ref().take(3).collect();
        if path.starts_with("/Volumes") {
            return first_two.exists();
        }
        true
    }
    #[cfg(target_os = "windows")]
    {
        path.components()
            .next()
            .map(|c| PathBuf::from(c.as_os_str()).join("\\").exists())
            .unwrap_or(false)
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        nearest_existing(path)
            .map(|p| p != Path::new("/"))
            .unwrap_or(false)
    }
}

fn nearest_existing(path: &Path) -> Option<PathBuf> {
    let mut p = path.to_path_buf();
    loop {
        if p.exists() {
            return Some(p);
        }
        if !p.pop() {
            return None;
        }
    }
}

/// True when `path` is on a different drive than the system/home drive.
pub fn is_other_drive(path: &Path) -> bool {
    let Some(existing) = nearest_existing(path) else {
        return false;
    };
    let existing = existing.canonicalize().unwrap_or(existing);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let Some(home) = dirs::home_dir() else {
            return false;
        };
        match (std::fs::metadata(&existing), std::fs::metadata(&home)) {
            (Ok(a), Ok(b)) => a.dev() != b.dev(),
            _ => false,
        }
    }
    #[cfg(windows)]
    {
        let system = std::env::var("SystemDrive")
            .unwrap_or_else(|_| "C:".into())
            .to_ascii_uppercase();
        let s = existing
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_ascii_uppercase();
        !s.starts_with(&system)
    }
}

fn disk_info(path: &Path) -> (Option<u64>, Option<String>, bool) {
    let Some(existing) = nearest_existing(path) else {
        return (None, None, false);
    };
    let existing = existing.canonicalize().unwrap_or(existing);
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let best = disks
        .iter()
        .filter(|d| existing.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len());
    match best {
        Some(d) => (
            Some(d.available_space()),
            Some(d.name().to_string_lossy().into_owned()).filter(|n| !n.is_empty()),
            d.is_removable(),
        ),
        None => (None, None, false),
    }
}

fn dir_size(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

#[derive(Serialize)]
pub struct StorageAreaInfo {
    pub area: Area,
    pub path: String,
    pub default_path: String,
    pub is_custom: bool,
    /// On a different drive than the system drive (external, second disk, network).
    pub is_other_drive: bool,
    pub is_removable: bool,
    /// The folder's drive is connected (always true for the default folder).
    pub available: bool,
    pub used_bytes: u64,
    pub free_bytes: Option<u64>,
    pub drive_name: Option<String>,
}

fn area_info(area: Area) -> Result<StorageAreaInfo, String> {
    let path = current_dir(area)?;
    let default_path = default_dir(area)?;
    let is_custom = load_config().get(area).is_some();
    let available = !is_custom || path.exists() || drive_present(&path);
    let (free_bytes, drive_name, is_removable) = if available {
        disk_info(&path)
    } else {
        (None, None, false)
    };
    Ok(StorageAreaInfo {
        area,
        path: path.to_string_lossy().into_owned(),
        default_path: default_path.to_string_lossy().into_owned(),
        is_custom,
        is_other_drive: available && is_other_drive(&path),
        is_removable,
        available,
        used_bytes: if available { dir_size(&path) } else { 0 },
        free_bytes,
        drive_name,
    })
}

#[tauri::command]
pub async fn get_storage_locations() -> Result<Vec<StorageAreaInfo>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        Ok(vec![area_info(Area::Models)?, area_info(Area::Recordings)?])
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Meeting audio is played through the asset protocol, whose static scope only
/// covers the default folder; allow a custom recordings folder too.
pub fn allow_recordings_in_asset_scope(app: &AppHandle) {
    use tauri::Manager;
    if load_config().get(Area::Recordings).is_none() {
        return;
    }
    if let Ok(dir) = current_dir(Area::Recordings) {
        if let Err(e) = app.asset_protocol_scope().allow_directory(&dir, true) {
            eprintln!(
                "[STORAGE] Could not allow {} for playback: {e}",
                dir.display()
            );
        }
    }
}

// ── Moving ───────────────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
struct MoveProgress {
    area: Area,
    done_bytes: u64,
    total_bytes: u64,
}

struct Mover<'a> {
    on_progress: &'a dyn Fn(u64, u64),
    total: u64,
    done: u64,
    last_emit: Instant,
}

impl Mover<'_> {
    fn advance(&mut self, bytes: u64) {
        self.done += bytes;
        if self.last_emit.elapsed().as_millis() >= 150 || self.done >= self.total {
            self.last_emit = Instant::now();
            (self.on_progress)(self.done, self.total);
        }
    }

    fn copy_file(&mut self, src: &Path, dst: &Path) -> Result<(), String> {
        let tmp = dst.with_extension(format!("taurscribe-moving-{:016x}", rand::random::<u64>()));
        let mut input = std::fs::File::open(src)
            .map_err(|e| format!("Could not read {}: {e}", src.display()))?;
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| format!("Could not write {}: {e}", tmp.display()))?;
        let result = (|| -> Result<(), String> {
            let mut buf = vec![0u8; 4 << 20];
            loop {
                let n = input
                    .read(&mut buf)
                    .map_err(|e| format!("Read failed for {}: {e}", src.display()))?;
                if n == 0 {
                    break;
                }
                output
                    .write_all(&buf[..n])
                    .map_err(|e| format!("Write failed for {}: {e}", dst.display()))?;
                self.advance(n as u64);
            }
            output
                .sync_all()
                .map_err(|e| format!("Could not flush {}: {e}", dst.display()))?;
            drop(output);
            if !files_equal(src, &tmp)? {
                return Err(format!(
                    "Copy of {} did not match the original; the original was kept.",
                    src.display()
                ));
            }
            if dst.exists() {
                return Err(format!(
                    "{} appeared during the move; both files were kept.",
                    dst.display()
                ));
            }
            std::fs::rename(&tmp, dst)
                .map_err(|e| format!("Could not finish {}: {e}", dst.display()))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    /// Check all destination names before writing anything.
    fn check_conflicts(src: &Path, dst: &Path) -> Result<(), String> {
        for entry in
            std::fs::read_dir(src).map_err(|e| format!("Could not read {}: {e}", src.display()))?
        {
            let entry = entry.map_err(|e| e.to_string())?;
            let from = entry.path();
            let to = dst.join(entry.file_name());
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_dir() {
                if to.exists() && !to.is_dir() {
                    return Err(format!(
                        "{} is a file where a folder is needed; no files were moved.",
                        to.display()
                    ));
                }
                Self::check_conflicts(&from, &to)?;
            } else if kind.is_file() && to.exists() && !files_equal(&from, &to)? {
                return Err(format!(
                    "{} already contains a different file; no files were moved.",
                    to.display()
                ));
            }
        }
        Ok(())
    }

    /// Copy and verify every file. Source files stay put until configuration and
    /// database paths are committed, so a failed migration leaves them usable.
    fn copy_contents(&mut self, src: &Path, dst: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dst)
            .map_err(|e| format!("Could not create {}: {e}", dst.display()))?;
        let entries =
            std::fs::read_dir(src).map_err(|e| format!("Could not read {}: {e}", src.display()))?;
        for entry in entries {
            let entry =
                entry.map_err(|e| format!("Could not read an entry in {}: {e}", src.display()))?;
            let from = entry.path();
            let to = dst.join(entry.file_name());
            let ft = entry.file_type().map_err(|e| e.to_string())?;
            if ft.is_dir() {
                self.copy_contents(&from, &to)?;
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
            if to.exists() {
                if files_equal(&from, &to)? {
                    self.advance(len);
                    continue;
                }
                return Err(format!(
                    "{} already contains a different file; neither file was overwritten.",
                    to.display()
                ));
            }
            self.copy_file(&from, &to)?;
        }
        Ok(())
    }
}

fn remove_verified_sources(src: &Path, dst: &Path) -> Result<(), String> {
    for entry in
        std::fs::read_dir(src).map_err(|e| format!("Could not read {}: {e}", src.display()))?
    {
        let entry = entry.map_err(|e| e.to_string())?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            remove_verified_sources(&from, &to)?;
            let _ = std::fs::remove_dir(&from);
        } else if kind.is_file() && files_equal(&from, &to)? {
            std::fs::remove_file(&from)
                .map_err(|e| format!("Could not remove {}: {e}", from.display()))?;
        }
    }
    Ok(())
}

fn files_equal(a: &Path, b: &Path) -> Result<bool, String> {
    let mut left =
        std::fs::File::open(a).map_err(|e| format!("Could not read {}: {e}", a.display()))?;
    let mut right =
        std::fs::File::open(b).map_err(|e| format!("Could not read {}: {e}", b.display()))?;
    if left.metadata().map_err(|e| e.to_string())?.len()
        != right.metadata().map_err(|e| e.to_string())?.len()
    {
        return Ok(false);
    }
    let mut a_buf = vec![0u8; 4 << 20];
    let mut b_buf = vec![0u8; 4 << 20];
    loop {
        let n = left.read(&mut a_buf).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(true);
        }
        right
            .read_exact(&mut b_buf[..n])
            .map_err(|e| e.to_string())?;
        if a_buf[..n] != b_buf[..n] {
            return Ok(false);
        }
    }
}

fn same_place(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn relocated_path(value: &str, old: &Path, new: &Path) -> Option<String> {
    Path::new(value)
        .strip_prefix(old)
        .ok()
        .map(|relative| {
            if relative.as_os_str().is_empty() {
                new.to_string_lossy().into_owned()
            } else {
                new.join(relative).to_string_lossy().into_owned()
            }
        })
}

/// Recordings keep absolute paths in the history database; point only paths
/// inside the old directory at the new one. Update every column in one transaction.
fn rewrite_recording_paths_in_db(
    conn: &mut rusqlite::Connection,
    old: &Path,
    new: &Path,
) -> Result<(), String> {
    use rusqlite::OptionalExtension;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let columns = [
        ("meetings", "audio_path", false),
        ("meeting_turns", "snippet_path", false),
        ("meeting_turns", "candidate_snippets", true),
        ("speaker_vault", "snippet_path", false),
        ("speaker_vault", "candidate_snippets", true),
    ];
    for (table, column, is_json) in columns {
        let table_exists: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if table_exists.is_none() {
            continue;
        }
        let has_column = {
            let mut stmt = tx
                .prepare(&format!("PRAGMA table_info({table})"))
                .map_err(|e| e.to_string())?;
            let columns = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(|e| e.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            columns.iter().any(|name| name == column)
        };
        if !has_column {
            continue;
        }
        let rows: Vec<(i64, String)> = {
            let mut stmt = tx
                .prepare(&format!(
                    "SELECT rowid, {column} FROM {table} WHERE {column} IS NOT NULL"
                ))
                .map_err(|e| e.to_string())?;
            let collected = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|e| e.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            collected
        };
        for (rowid, value) in rows {
            let replacement = if is_json {
                if value.trim().is_empty() {
                    None
                } else {
                    let mut paths: Vec<String> = serde_json::from_str(&value).map_err(|e| {
                        format!("Invalid {table}.{column} JSON at row {rowid}: {e}")
                    })?;
                    let mut changed = false;
                    for path in &mut paths {
                        if let Some(next) = relocated_path(path, old, new) {
                            *path = next;
                            changed = true;
                        }
                    }
                    if changed {
                        Some(serde_json::to_string(&paths).map_err(|e| e.to_string())?)
                    } else {
                        None
                    }
                }
            } else {
                relocated_path(&value, old, new)
            };
            if let Some(replacement) = replacement {
                tx.execute(
                    &format!("UPDATE {table} SET {column} = ?1 WHERE rowid = ?2"),
                    rusqlite::params![replacement, rowid],
                )
                .map_err(|e| format!("Could not update {table}.{column}: {e}"))?;
            }
        }
    }
    tx.commit()
        .map_err(|e| format!("Could not save recording paths: {e}"))
}

fn rewrite_recording_paths(old: &Path, new: &Path) -> Result<(), String> {
    let db = app_data_root()?.join("transcript_history.db");
    if !db.exists() {
        return Ok(());
    }
    let mut conn = rusqlite::Connection::open(&db).map_err(|e| e.to_string())?;
    rewrite_recording_paths_in_db(&mut conn, old, new)
}

/// Point `area` at `path` (None = back to the default folder). With
/// `move_files`, existing files are moved across first.
#[tauri::command]
pub async fn set_storage_location(
    app: AppHandle,
    state: tauri::State<'_, crate::state::AudioState>,
    area: Area,
    path: Option<String>,
    move_files: bool,
) -> Result<StorageAreaInfo, String> {
    let _recording_transition = state.begin_recording_transition()?;
    if state.recording_handle.lock().unwrap().is_some() {
        return Err("Stop the current recording before changing where files are stored.".into());
    }
    if area == Area::Models && crate::commands::any_download_active() {
        return Err("Wait for model downloads to finish before moving the models folder.".into());
    }
    if area == Area::Models && move_files {
        use std::sync::atomic::Ordering;
        if state.engine_loading.load(Ordering::Relaxed) {
            return Err("A model is loading. Try again in a moment.".into());
        }
        // Loaded weights keep their files open (and locked on Windows).
        let unloaded = state.unload_all_loaded_asr()?;
        if !unloaded.is_empty() {
            crate::tray::reconcile_model_loaded_tray(&app, &state);
            let _ = app.emit("model-unloaded", ());
        }
    }
    tauri::async_runtime::spawn_blocking(move || {
        let old = current_dir(area)?;
        let new = match &path {
            Some(p) => PathBuf::from(p),
            None => default_dir(area)?,
        };
        if new.as_os_str().is_empty() || !new.is_absolute() {
            return Err("Pick a full folder path.".into());
        }
        std::fs::create_dir_all(&new).map_err(|e| format!("Can't use {}: {e}", new.display()))?;
        // Writable?
        let probe = new.join(".taurscribe-write-test");
        std::fs::write(&probe, b"ok")
            .map_err(|e| format!("Taurscribe can't write to {}: {e}", new.display()))?;
        let _ = std::fs::remove_file(&probe);

        let copying = move_files && old.exists() && !same_place(&old, &new);
        if copying {
            if new
                .canonicalize()
                .ok()
                .zip(old.canonicalize().ok())
                .is_some_and(|(n, o)| n.starts_with(&o))
            {
                return Err("The new folder can't be inside the current one.".into());
            }
            let total = dir_size(&old);
            if let (Some(free), _, _) = disk_info(&new) {
                if free < total {
                    return Err(format!(
                        "Not enough space: {:.1} GB needed, {:.1} GB free.",
                        total as f64 / 1e9,
                        free as f64 / 1e9
                    ));
                }
            }
            let emit = |done_bytes, total_bytes| {
                let _ = app.emit(
                    "storage-move-progress",
                    MoveProgress {
                        area,
                        done_bytes,
                        total_bytes,
                    },
                );
            };
            let mut mover = Mover {
                on_progress: &emit,
                total,
                done: 0,
                last_emit: Instant::now(),
            };
            Mover::check_conflicts(&old, &new)?;
            mover.copy_contents(&old, &new)?;
            if area == Area::Recordings {
                rewrite_recording_paths(&old, &new)?;
            }
        }

        let mut cfg = load_config();
        let is_default = same_place(&new, &default_dir(area)?);
        cfg.set(area, if is_default { None } else { Some(new.clone()) });
        save_config(&cfg)?;
        if copying {
            if let Err(e) = remove_verified_sources(&old, &new) {
                eprintln!(
                    "[STORAGE] Files were switched but old copies could not all be removed: {e}"
                );
            }
        }
        if area == Area::Recordings {
            allow_recordings_in_asset_scope(&app);
        }
        if area == Area::Models {
            if let Err(e) = crate::watcher::start_models_watcher(app.clone()) {
                eprintln!("[STORAGE] Could not watch the new models folder: {e}");
            }
            let _ = app.emit("models-changed", ());
        }
        area_info(area)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ── Speed test ───────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct SpeedResult {
    pub path: String,
    pub write_mb_s: f64,
    pub read_mb_s: f64,
}

const TEST_BYTES: usize = 256 << 20;
const CHUNK: usize = 4 << 20;

/// 4 KiB-aligned buffer (unbuffered I/O on Windows needs sector alignment).
fn aligned_buf(len: usize) -> (Vec<u8>, usize) {
    let v = vec![0u8; len + 4096];
    let off = (4096 - (v.as_ptr() as usize % 4096)) % 4096;
    (v, off)
}

fn open_uncached(path: &Path) -> std::io::Result<std::fs::File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_NO_BUFFERING: u32 = 0x2000_0000;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_NO_BUFFERING)
            .open(path)
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::io::AsRawFd;
        let f = std::fs::File::open(path)?;
        // Bypass the page cache so we time the drive, not RAM.
        unsafe { libc::fcntl(f.as_raw_fd(), libc::F_NOCACHE, 1) };
        Ok(f)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::os::unix::io::AsRawFd;
        let f = std::fs::File::open(path)?;
        unsafe { libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) };
        Ok(f)
    }
}

fn measure(dir: &Path) -> Result<SpeedResult, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Can't use {}: {e}", dir.display()))?;
    let file = dir.join(".taurscribe-speed-test");
    let result = (|| {
        let (mut buf, off) = aligned_buf(CHUNK);
        // Incompressible-ish data so drive/filesystem compression doesn't flatter the result.
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        for b in buf[off..off + CHUNK].iter_mut() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *b = x as u8;
        }
        let start = Instant::now();
        {
            let mut f = std::fs::File::create(&file)
                .map_err(|e| format!("Can't write to {}: {e}", dir.display()))?;
            for _ in 0..TEST_BYTES / CHUNK {
                f.write_all(&buf[off..off + CHUNK])
                    .map_err(|e| e.to_string())?;
            }
            f.sync_all().map_err(|e| e.to_string())?;
        }
        let write_s = start.elapsed().as_secs_f64();

        let start = Instant::now();
        let mut f = open_uncached(&file).map_err(|e| e.to_string())?;
        let mut read = 0usize;
        loop {
            let n = f
                .read(&mut buf[off..off + CHUNK])
                .map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            read += n;
        }
        let read_s = start.elapsed().as_secs_f64();
        let mb = TEST_BYTES as f64 / (1024.0 * 1024.0);
        Ok(SpeedResult {
            path: dir.to_string_lossy().into_owned(),
            write_mb_s: mb / write_s.max(1e-6),
            read_mb_s: (read as f64 / (1024.0 * 1024.0)) / read_s.max(1e-6),
        })
    })();
    let _ = std::fs::remove_file(&file);
    result
}

/// Measures how fast Taurscribe can write and read a folder (256 MB test file,
/// bypassing the OS cache). Pass no path to test the folder `area` uses now.
#[tauri::command]
pub async fn measure_storage_speed(
    area: Area,
    path: Option<String>,
) -> Result<SpeedResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = match path {
            Some(p) => PathBuf::from(p),
            None => resolve_dir(area)?,
        };
        measure(&dir)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Opens the folder `area` uses in the file manager.
#[tauri::command]
pub fn open_storage_location(app: AppHandle, area: Area) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = resolve_dir(area)?;
    app.opener()
        .open_path(dir.to_string_lossy().as_ref(), None::<&str>)
        .map_err(|e| format!("Failed to open folder: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_test_runs_and_cleans_up() {
        let dir = std::env::temp_dir().join(format!("ts-speed-{}", std::process::id()));
        let r = measure(&dir).unwrap();
        assert!(r.write_mb_s > 0.0 && r.read_mb_s > 0.0);
        assert!(!dir.join(".taurscribe-speed-test").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn db_with_paths(audio: &[&str], snippets: &str) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::commands::meetings::ensure_meetings_schema(&conn).unwrap();
        for (i, a) in audio.iter().enumerate() {
            conn.execute(
                "INSERT INTO meetings (session_id, title, platform, app_name, url, created_at, duration_ms, audio_path, transcript_raw, category)
                 VALUES (?1, 't', 'p', 'a', '', '', 0, ?2, '', 'general')",
                rusqlite::params![format!("s{i}"), a],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO meeting_turns (meeting_id, speaker_id, speaker_name, start_ms, end_ms, channel, text, snippet_path, candidate_snippets)
             VALUES (1, 'x', 'X', 0, 1, 1, '', NULL, ?1)",
            [snippets],
        )
        .unwrap();
        conn
    }

    fn audio_paths(conn: &rusqlite::Connection) -> Vec<String> {
        conn.prepare("SELECT audio_path FROM meetings ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn rewrite_paths_moves_only_files_under_the_old_folder() {
        let mut conn = db_with_paths(
            &["/data/meetings/a.webm", "/data/meetings-old/b.webm", "/data/meetings"],
            r#"["/data/meetings/s1.wav","/data/meetings-old/s2.wav"]"#,
        );
        rewrite_recording_paths_in_db(&mut conn, Path::new("/data/meetings"), Path::new("/ext/rec")).unwrap();
        assert_eq!(audio_paths(&conn), ["/ext/rec/a.webm", "/data/meetings-old/b.webm", "/ext/rec"]);
        let cands: String = conn.query_row("SELECT candidate_snippets FROM meeting_turns", [], |r| r.get(0)).unwrap();
        assert_eq!(cands, r#"["/ext/rec/s1.wav","/data/meetings-old/s2.wav"]"#);
    }

    #[cfg(unix)]
    #[test]
    fn rewrite_paths_does_not_repeat_when_new_path_extends_old() {
        let mut conn = db_with_paths(&["/data/meetings/a.webm"], r#"["/data/meetings/s1.wav"]"#);
        rewrite_recording_paths_in_db(&mut conn, Path::new("/data/meetings/"), Path::new("/data/meetings-ext")).unwrap();
        assert_eq!(audio_paths(&conn), ["/data/meetings-ext/a.webm"]);
        let cands: String = conn.query_row("SELECT candidate_snippets FROM meeting_turns", [], |r| r.get(0)).unwrap();
        assert_eq!(cands, r#"["/data/meetings-ext/s1.wav"]"#);
    }

    #[test]
    fn rewrite_paths_tolerates_missing_tables() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        rewrite_recording_paths_in_db(&mut conn, Path::new("/a"), Path::new("/b")).unwrap();
    }

    #[test]
    fn nearest_existing_walks_up_to_a_real_folder() {
        let tmp = std::env::temp_dir();
        assert_eq!(nearest_existing(&tmp.join("no-such-dir-ts").join("deeper")), Some(tmp.clone()));
        assert_eq!(nearest_existing(&tmp), Some(tmp));
    }

    #[test]
    fn dir_size_counts_nested_files() {
        let root = std::env::temp_dir().join(format!("ts-size-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("x/y")).unwrap();
        std::fs::write(root.join("a"), [0u8; 10]).unwrap();
        std::fs::write(root.join("x/y/b"), [0u8; 5]).unwrap();
        assert_eq!(dir_size(&root), 15);
        assert_eq!(dir_size(&root.join("missing")), 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn move_contents_moves_nested_files_and_keeps_existing() {
        let root = std::env::temp_dir().join(format!("ts-move-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (src, dst) = (root.join("a"), root.join("b"));
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(src.join("one.bin"), b"hello").unwrap();
        std::fs::write(src.join("sub").join("two.bin"), b"world!").unwrap();
        std::fs::write(dst.join("one.bin"), b"hello").unwrap(); // already present
        let mut m = Mover {
            on_progress: &|_, _| {},
            total: 11,
            done: 0,
            last_emit: Instant::now(),
        };
        m.copy_contents(&src, &dst).unwrap();
        assert_eq!(
            std::fs::read(dst.join("sub").join("two.bin")).unwrap(),
            b"world!"
        );
        assert!(src.join("one.bin").exists());
        remove_verified_sources(&src, &dst).unwrap();
        assert!(!src.join("one.bin").exists() && !src.join("sub").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn move_refuses_same_size_different_content() {
        let root = std::env::temp_dir().join(format!("ts-move-conflict-{}", rand::random::<u64>()));
        let (src, dst) = (root.join("a"), root.join("b"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(src.join("recording.wav"), b"one!").unwrap();
        std::fs::write(dst.join("recording.wav"), b"two!").unwrap();
        let mut mover = Mover {
            on_progress: &|_, _| {},
            total: 4,
            done: 0,
            last_emit: Instant::now(),
        };
        assert!(Mover::check_conflicts(&src, &dst).is_err());
        assert!(mover.copy_contents(&src, &dst).is_err());
        assert_eq!(std::fs::read(src.join("recording.wav")).unwrap(), b"one!");
        assert_eq!(std::fs::read(dst.join("recording.wav")).unwrap(), b"two!");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn path_migration_updates_only_old_folder_once_and_preserves_json() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE meetings (audio_path TEXT);
             CREATE TABLE meeting_turns (snippet_path TEXT, candidate_snippets TEXT);
             CREATE TABLE speaker_vault (snippet_path TEXT, candidate_snippets TEXT);
             INSERT INTO meetings VALUES ('/tmp/meetings/audio/one.wav');
             INSERT INTO meetings VALUES ('/tmp/meetings-old/audio/two.wav');
             INSERT INTO meeting_turns VALUES ('/tmp/meetings/snippets/a.wav', '[\"/tmp/meetings/snippets/a.wav\",\"/tmp/meetings-old/snippets/b.wav\"]');"
        ).unwrap();
        rewrite_recording_paths_in_db(
            &mut conn,
            Path::new("/tmp/meetings"),
            Path::new("/tmp/meetings2"),
        )
        .unwrap();
        let paths: Vec<String> = conn
            .prepare("SELECT audio_path FROM meetings ORDER BY rowid")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            paths,
            [
                "/tmp/meetings2/audio/one.wav",
                "/tmp/meetings-old/audio/two.wav"
            ]
        );
        let (snippet, candidates): (String, String) = conn
            .query_row(
                "SELECT snippet_path, candidate_snippets FROM meeting_turns",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(snippet, "/tmp/meetings2/snippets/a.wav");
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&candidates).unwrap(),
            [
                "/tmp/meetings2/snippets/a.wav",
                "/tmp/meetings-old/snippets/b.wav"
            ]
        );
    }

    #[test]
    fn path_migration_rolls_back_if_any_column_update_fails() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE meetings (audio_path TEXT);
             CREATE TABLE speaker_vault (snippet_path TEXT, candidate_snippets TEXT);
             INSERT INTO meetings VALUES ('/tmp/meetings/audio/a.wav');
             INSERT INTO speaker_vault VALUES ('/tmp/meetings/snippets/a.wav', NULL);
             CREATE TRIGGER reject_snippet_update BEFORE UPDATE OF snippet_path ON speaker_vault
             BEGIN SELECT RAISE(ABORT, 'blocked'); END;",
        )
        .unwrap();
        assert!(rewrite_recording_paths_in_db(
            &mut conn,
            Path::new("/tmp/meetings"),
            Path::new("/tmp/meetings2")
        )
        .is_err());
        let meeting_path: String = conn
            .query_row("SELECT audio_path FROM meetings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(meeting_path, "/tmp/meetings/audio/a.wav");
    }

    #[test]
    fn vm_nested_conflict_is_found_before_any_file_is_copied() {
        let root = std::env::temp_dir().join(format!("ts-vm-conflict-{}", rand::random::<u64>()));
        let (src, dst) = (root.join("source"), root.join("destination"));
        std::fs::create_dir_all(src.join("nested")).unwrap();
        std::fs::create_dir_all(dst.join("nested")).unwrap();
        std::fs::write(src.join("first.wav"), b"safe to copy").unwrap();
        std::fs::write(src.join("nested").join("last.wav"), b"original").unwrap();
        std::fs::write(dst.join("nested").join("last.wav"), b"conflict").unwrap();

        assert!(Mover::check_conflicts(&src, &dst).is_err());
        assert!(!dst.join("first.wav").exists());
        assert_eq!(
            std::fs::read(src.join("nested").join("last.wav")).unwrap(),
            b"original"
        );
        assert_eq!(
            std::fs::read(dst.join("nested").join("last.wav")).unwrap(),
            b"conflict"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn vm_failed_copy_keeps_both_files_and_removes_staging_file() {
        let root = std::env::temp_dir().join(format!("ts-vm-stage-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        let src = root.join("source.wav");
        let dst = root.join("destination.wav");
        std::fs::write(&src, b"original audio").unwrap();
        std::fs::write(&dst, b"other audio").unwrap();
        let mut mover = Mover {
            on_progress: &|_, _| {},
            total: 14,
            done: 0,
            last_emit: Instant::now(),
        };
        assert!(mover.copy_file(&src, &dst).is_err());
        assert_eq!(std::fs::read(&src).unwrap(), b"original audio");
        assert_eq!(std::fs::read(&dst).unwrap(), b"other audio");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn vm_recording_path_migration_uses_native_paths_and_is_repeatable() {
        let root = std::env::temp_dir().join(format!("ts-vm-paths-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        let old = root.join("recordings");
        let new = root.join("moved recordings");
        let old_audio = old.join("audio").join("meeting.wav");
        let new_audio = new.join("audio").join("meeting.wav");
        let outside = root.join("recordings-old").join("outside.wav");
        let db = root.join("history.db");
        let mut conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE meetings (audio_path TEXT); CREATE TABLE meeting_turns (candidate_snippets TEXT);").unwrap();
        conn.execute(
            "INSERT INTO meetings VALUES (?1)",
            [old_audio.to_str().unwrap()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO meetings VALUES (?1)",
            [outside.to_str().unwrap()],
        )
        .unwrap();
        let candidates =
            serde_json::to_string(&[old_audio.to_str().unwrap(), outside.to_str().unwrap()])
                .unwrap();
        conn.execute("INSERT INTO meeting_turns VALUES (?1)", [candidates])
            .unwrap();

        for _ in 0..2 {
            rewrite_recording_paths_in_db(&mut conn, &old, &new).unwrap();
        }
        let paths: Vec<String> = conn
            .prepare("SELECT audio_path FROM meetings ORDER BY rowid")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            paths,
            [new_audio.to_string_lossy(), outside.to_string_lossy()]
        );
        let json: String = conn
            .query_row("SELECT candidate_snippets FROM meeting_turns", [], |row| {
                row.get(0)
            })
            .unwrap();
        let actual: Vec<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(
            actual,
            [new_audio.to_string_lossy(), outside.to_string_lossy()]
        );
        drop(conn);
        std::fs::remove_dir_all(root).unwrap();
    }
}
