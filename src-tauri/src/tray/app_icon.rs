//! macOS: the icon of the app a meeting is captured from (Chrome, Zoom, Slack…),
//! with a status dot, for the menu-bar tray icon.
//!
//! The icon comes from the app bundle's `.icns` (converted by `sips`, cached per
//! app), so no third-party logos ship with Taurscribe.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::image::Image;

use super::status::{draw_dot, Dot};

const PX: u32 = 44; // @2x menu-bar size

static CACHE: Mutex<Option<HashMap<(PathBuf, Dot), Image<'static>>>> = Mutex::new(None);

/// The meeting app's icon with a status dot, or None when the app or its icon
/// can't be found (the caller then uses a generic call icon).
pub fn meeting_app_icon(app_name: Option<&str>, pid: Option<u32>, dot: Dot) -> Option<Image<'static>> {
    let bundle = bundle_for(pid, app_name)?;
    let mut guard = CACHE.lock().ok()?;
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some(img) = cache.get(&(bundle.clone(), dot)) {
        return Some(img.clone());
    }
    let img = compose(&icon_png(&bundle)?, dot)?;
    cache.insert((bundle, dot), img.clone());
    Some(img)
}

/// Display name of the app bundle behind a meeting (e.g. "Google Chrome").
pub fn app_display_name(app_name: Option<&str>, pid: Option<u32>) -> Option<String> {
    bundle_for(pid, app_name)?
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
}

fn bundle_for(pid: Option<u32>, app_name: Option<&str>) -> Option<PathBuf> {
    if let Some(pid) = pid {
        let mut sys = sysinfo::System::new();
        let spid = sysinfo::Pid::from_u32(pid);
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[spid]), true);
        if let Some(exe) = sys.process(spid).and_then(|p| p.exe()) {
            let bundle = crate::audio_dual_channel::app_bundle_path(exe);
            if bundle.extension().is_some_and(|e| e == "app") {
                return Some(bundle);
            }
        }
    }
    let name = app_name?.trim();
    if name.is_empty() {
        return None;
    }
    let home = dirs::home_dir().unwrap_or_default();
    [PathBuf::from("/Applications"), PathBuf::from("/System/Applications"), home.join("Applications")]
        .into_iter()
        .map(|dir| dir.join(format!("{name}.app")))
        .find(|p| p.exists())
}

/// PNG bytes of the bundle's icon at PX×PX.
fn icon_png(bundle: &Path) -> Option<Vec<u8>> {
    let plist = bundle.join("Contents/Info.plist");
    let out = std::process::Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :CFBundleIconFile"])
        .arg(&plist)
        .output()
        .ok()?;
    let mut name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if name.is_empty() {
        return None;
    }
    if !name.ends_with(".icns") {
        name.push_str(".icns");
    }
    let icns = bundle.join("Contents/Resources").join(name);
    let dest = std::env::temp_dir().join(format!(
        "taurscribe-tray-{:x}.png",
        fxhash(bundle.to_string_lossy().as_bytes())
    ));
    let ok = std::process::Command::new("/usr/bin/sips")
        .args(["-s", "format", "png", "-z", &PX.to_string(), &PX.to_string()])
        .arg(&icns)
        .arg("--out")
        .arg(&dest)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ok {
        return None;
    }
    std::fs::read(&dest).ok()
}

/// Decodes the icon and draws the status dot on it.
fn compose(png: &[u8], dot: Dot) -> Option<Image<'static>> {
    let img = Image::from_bytes(png).ok()?;
    let (w, h) = (img.width(), img.height());
    let mut rgba = img.rgba().to_vec();
    draw_dot(&mut rgba, w, h, dot);
    Some(Image::new_owned(rgba, w, h))
}

fn fxhash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}
