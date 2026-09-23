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
    Ok(dirs::data_local_dir().ok_or("Could not find AppData directory")?.join("Taurscribe"))
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
        std::fs::create_dir_all(parent).map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
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
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;
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
        path.components().next().map(|c| PathBuf::from(c.as_os_str()).join("\\").exists()).unwrap_or(false)
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        nearest_existing(path).map(|p| p != Path::new("/")).unwrap_or(false)
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
    let Some(existing) = nearest_existing(path) else { return false };
    let existing = existing.canonicalize().unwrap_or(existing);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let Some(home) = dirs::home_dir() else { return false };
        match (std::fs::metadata(&existing), std::fs::metadata(&home)) {
            (Ok(a), Ok(b)) => a.dev() != b.dev(),
            _ => false,
        }
    }
    #[cfg(windows)]
    {
        let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()).to_ascii_uppercase();
        let s = existing.to_string_lossy().trim_start_matches(r"\\?\").to_ascii_uppercase();
        !s.starts_with(&system)
    }
}

fn disk_info(path: &Path) -> (Option<u64>, Option<String>, bool) {
    let Some(existing) = nearest_existing(path) else { return (None, None, false) };
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
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
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
    let (free_bytes, drive_name, is_removable) = if available { disk_info(&path) } else { (None, None, false) };
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
    tauri::async_runtime::spawn_blocking(|| Ok(vec![area_info(Area::Models)?, area_info(Area::Recordings)?]))
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
            eprintln!("[STORAGE] Could not allow {} for playback: {e}", dir.display());
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
        let tmp = dst.with_extension("taurscribe-moving");
        let mut input = std::fs::File::open(src).map_err(|e| format!("Could not read {}: {e}", src.display()))?;
        let mut output = std::fs::File::create(&tmp).map_err(|e| format!("Could not write {}: {e}", tmp.display()))?;
        let mut buf = vec![0u8; 4 << 20];
        loop {
            let n = input.read(&mut buf).map_err(|e| format!("Read failed for {}: {e}", src.display()))?;
            if n == 0 {
                break;
            }
            output.write_all(&buf[..n]).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                format!("Write failed for {}: {e}", dst.display())
            })?;
            self.advance(n as u64);
        }
        output.sync_all().map_err(|e| format!("Could not flush {}: {e}", dst.display()))?;
        drop(output);
        std::fs::rename(&tmp, dst).map_err(|e| format!("Could not finish {}: {e}", dst.display()))?;
        Ok(())
    }

    /// Moves every entry of `src` into `dst`. Files are copied and verified by size
    /// before the original is removed, so an interrupted move never loses data.
    fn move_contents(&mut self, src: &Path, dst: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dst).map_err(|e| format!("Could not create {}: {e}", dst.display()))?;
        let entries = std::fs::read_dir(src).map_err(|e| format!("Could not read {}: {e}", src.display()))?;
        for entry in entries.filter_map(|e| e.ok()) {
            let from = entry.path();
            let to = dst.join(entry.file_name());
            let ft = entry.file_type().map_err(|e| e.to_string())?;
            if ft.is_dir() {
                self.move_contents(&from, &to)?;
                let _ = std::fs::remove_dir(&from);
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
            // Same volume: a rename is instant.
            if !to.exists() && std::fs::rename(&from, &to).is_ok() {
                self.advance(len);
                continue;
            }
            // Already there (e.g. the user picked a folder that has these models): keep theirs.
            if to.exists() && std::fs::metadata(&to).map(|m| m.len()).unwrap_or(u64::MAX) == len {
                std::fs::remove_file(&from).map_err(|e| format!("Could not remove {}: {e}", from.display()))?;
                self.advance(len);
                continue;
            }
            self.copy_file(&from, &to)?;
            if std::fs::metadata(&to).map(|m| m.len()).unwrap_or(0) != len {
                return Err(format!("Copy of {} came out the wrong size; the original was kept.", from.display()));
            }
            std::fs::remove_file(&from).map_err(|e| format!("Could not remove {}: {e}", from.display()))?;
        }
        Ok(())
    }
}

fn same_place(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Recordings keep absolute paths in the history database; point them at the new folder.
fn rewrite_recording_paths(old: &Path, new: &Path) -> Result<(), String> {
    let db = app_data_root()?.join("transcript_history.db");
    if !db.exists() {
        return Ok(());
    }
    let conn = rusqlite::Connection::open(&db).map_err(|e| e.to_string())?;
    let old_s = old.to_string_lossy().into_owned();
    let new_s = new.to_string_lossy().into_owned();
    // JSON columns store backslashes escaped (Windows paths).
    let old_json = old_s.replace('\\', "\\\\");
    let new_json = new_s.replace('\\', "\\\\");
    let columns = [
        ("meetings", "audio_path"),
        ("meeting_turns", "snippet_path"),
        ("meeting_turns", "candidate_snippets"),
        ("speaker_vault", "snippet_path"),
        ("speaker_vault", "candidate_snippets"),
    ];
    for (table, col) in columns {
        let sql = format!("UPDATE {table} SET {col} = REPLACE(REPLACE({col}, ?1, ?2), ?3, ?4) WHERE {col} IS NOT NULL");
        // Tables may not exist yet on a fresh install.
        let _ = conn.execute(&sql, rusqlite::params![old_s, new_s, old_json, new_json]);
    }
    Ok(())
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
        std::fs::write(&probe, b"ok").map_err(|e| format!("Taurscribe can't write to {}: {e}", new.display()))?;
        let _ = std::fs::remove_file(&probe);

        if move_files && old.exists() && !same_place(&old, &new) {
            if new.canonicalize().ok().zip(old.canonicalize().ok()).is_some_and(|(n, o)| n.starts_with(&o)) {
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
                let _ = app.emit("storage-move-progress", MoveProgress { area, done_bytes, total_bytes });
            };
            let mut mover = Mover { on_progress: &emit, total, done: 0, last_emit: Instant::now() };
            mover.move_contents(&old, &new)?;
            if area == Area::Recordings {
                rewrite_recording_paths(&old, &new)?;
            }
        }

        let mut cfg = load_config();
        let is_default = same_place(&new, &default_dir(area)?);
        cfg.set(area, if is_default { None } else { Some(new) });
        save_config(&cfg)?;
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
        std::fs::OpenOptions::new().read(true).custom_flags(FILE_FLAG_NO_BUFFERING).open(path)
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
            let mut f = std::fs::File::create(&file).map_err(|e| format!("Can't write to {}: {e}", dir.display()))?;
            for _ in 0..TEST_BYTES / CHUNK {
                f.write_all(&buf[off..off + CHUNK]).map_err(|e| e.to_string())?;
            }
            f.sync_all().map_err(|e| e.to_string())?;
        }
        let write_s = start.elapsed().as_secs_f64();

        let start = Instant::now();
        let mut f = open_uncached(&file).map_err(|e| e.to_string())?;
        let mut read = 0usize;
        loop {
            let n = f.read(&mut buf[off..off + CHUNK]).map_err(|e| e.to_string())?;
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
pub async fn measure_storage_speed(area: Area, path: Option<String>) -> Result<SpeedResult, String> {
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
        let mut m = Mover { on_progress: &|_, _| {}, total: 11, done: 0, last_emit: Instant::now() };
        m.move_contents(&src, &dst).unwrap();
        assert_eq!(std::fs::read(dst.join("sub").join("two.bin")).unwrap(), b"world!");
        assert!(!src.join("one.bin").exists() && !src.join("sub").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
