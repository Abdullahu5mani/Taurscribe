//! Writes the playback copy of a meeting WAV (diagnostic).
//! usage: cargo run --example meeting_audio_probe -- meeting.wav
fn main() {
    let wav = std::env::args().nth(1).expect("wav path");
    let out = taurscribe_lib::meeting_audio::compress_for_playback(std::path::Path::new(&wav)).expect("compress");
    println!("{}", out.display());
    println!("{:?}", taurscribe_lib::meeting_audio::last_processing_diagnostics());
}
