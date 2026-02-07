use crossbeam_channel::Sender;

// Wrapper struct to make the Audio Stream "moveable" between threads.
// By default, raw pointers/streams aren't thread-safe.
// We implement Send and Sync manually (unsafe) to tell Rust "Check constraints are met".
#[allow(dead_code)]
pub struct SendStream(pub cpal::Stream);
unsafe impl Send for SendStream {} // Can be moved to another thread
unsafe impl Sync for SendStream {} // Can be accessed from multiple threads

/// Keeps track of the tools needed while recording involves.
pub struct RecordingHandle {
    pub stream: SendStream,           // The actual connection to the microphone hardware
    pub file_tx: Sender<Vec<f32>>,    // Pipe to send audio to the "File Writer" thread
    pub whisper_tx: Sender<Vec<f32>>, // Pipe to send audio to the "Whisper AI" thread
    pub writer_thread: std::thread::JoinHandle<()>,
    pub transcriber_thread: std::thread::JoinHandle<()>,
}
