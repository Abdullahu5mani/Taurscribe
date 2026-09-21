//! Dumps meeting-record's raw view (audio processes + unfiltered meetings).
//! Diagnostic for scripts/harness meeting detection tests.
//! meeting-record is a macOS/Windows dependency only.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn main() {
    use meeting_record::meetings;
    for p in meetings::audio_processes() {
        if p.is_using_mic || p.is_playing_audio {
            println!("proc pid={} name={} bundle={} in={} out={}", p.pid, p.name, p.bundle_id, p.is_using_mic, p.is_playing_audio);
        }
    }
    for m in meetings::scan() {
        println!("meeting platform={} pid={} app={} title={:?} url={:?} mic={} out={} conf={}",
            m.platform.as_str(), m.pid, m.app_name, m.title, m.url, m.is_using_mic, m.is_playing_audio, m.confidence);
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn main() {}
