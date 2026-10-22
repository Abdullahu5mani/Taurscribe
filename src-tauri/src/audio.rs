use crossbeam_channel::Sender;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

// Wrapper struct to make the Audio Stream "moveable" between threads.
// By default, raw pointers/streams aren't thread-safe.
// We implement Send and Sync manually (unsafe) to tell Rust "Check constraints are met".
// Held in RecordingHandle solely for RAII ownership — dropped when recording stops.
pub struct SendStream(pub cpal::Stream);
unsafe impl Send for SendStream {} // Can be moved to another thread
unsafe impl Sync for SendStream {} // Can be accessed from multiple threads

/// What a recording captures, from the `audio_source` the UI starts it with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    /// The microphone, pasted as a dictation ("mic").
    Dictation,
    /// A call: the microphone and the computer's sound as two channels
    /// ("dual_channel"). Saved to Meetings.
    Call,
    /// A meeting in a room: everyone on one microphone ("in_person"). Saved to
    /// Meetings, with the speakers told apart after it ends.
    InPerson,
}

impl CaptureMode {
    pub fn from_source(source: &str) -> Self {
        match source {
            "dual_channel" => CaptureMode::Call,
            "in_person" => CaptureMode::InPerson,
            _ => CaptureMode::Dictation,
        }
    }

    pub fn is_meeting(self) -> bool {
        self != CaptureMode::Dictation
    }

    pub fn is_dual_channel(self) -> bool {
        self == CaptureMode::Call
    }
}

/// Keeps track of the tools needed while recording involves.
pub struct RecordingHandle {
    pub capture_loss: Arc<CaptureLoss>,
    pub stream: Option<SendStream>, // The actual connection to the microphone hardware (None in dual-channel mode)
    pub file_tx: Sender<Vec<f32>>,  // Pipe to send audio to the "File Writer" thread
    pub whisper_tx: Sender<Vec<f32>>, // Pipe to send audio to the "Whisper AI" thread
    pub writer_thread: std::thread::JoinHandle<Result<(), String>>,
    pub transcriber_thread: std::thread::JoinHandle<()>,
    pub level_stop: Arc<AtomicBool>, // Signal the level-emitter thread to exit
    pub level_thread: std::thread::JoinHandle<()>,
    pub mode: CaptureMode,
    pub dual_channel_stop: Option<Arc<AtomicBool>>,
    pub dual_channel_thread: Option<std::thread::JoinHandle<()>>,
}

/// Per-session diagnostics shared by nonblocking callbacks and Stop. Durable
/// audio loss must survive worker teardown and must never look like completion.
#[derive(Default)]
pub struct CaptureLoss {
    pub file_frames: AtomicU64,
    pub input_frames: AtomicU64,
    pub gap_frames: AtomicU64,
    pub preview_frames: AtomicU64,
    pub preview_failed: AtomicBool,
    pub meeting_warnings: std::sync::Mutex<Vec<String>>,
}

impl CaptureLoss {
    pub fn warn_meeting(&self, warning: String) {
        if let Ok(mut warnings) = self.meeting_warnings.lock() {
            if !warnings.contains(&warning) { warnings.push(warning); }
        }
    }
    /// Returns true only for the first failed write-queue send, so the callback
    /// can notify the UI once without blocking or flooding its event queue.
    pub fn send_file(&self, tx: &Sender<Vec<f32>>, samples: Vec<f32>, channels: usize) -> bool {
        let frames = samples.len() / channels.max(1);
        tx.try_send(samples).is_err()
            && self.file_frames.fetch_add(frames as u64, Ordering::Relaxed) == 0
    }

    /// Audio that was lost. Silence recorded for a track that delivered nothing
    /// (`gap_frames`) is not loss: a process tap can simply go quiet while the
    /// call app plays nothing, and counting it kept the stereo WAV and marked the
    /// meeting incomplete. It stays in the capture diagnostics.
    pub fn warnings(&self) -> Vec<String> {
        let file = self.file_frames.load(Ordering::Relaxed);
        let input = self.input_frames.load(Ordering::Relaxed);
        let mut warnings = self.meeting_warnings.lock().map(|w| w.clone()).unwrap_or_default();
        if file > 0 || input > 0 {
            let mut parts = Vec::new();
            if file > 0 {
                parts.push(format!("{file} frames could not be written"));
            }
            if input > 0 {
                parts.push(format!("{input} input frames were lost"));
            }
            warnings.push(format!("Recording is incomplete: {}.", parts.join(" and ")));
        }
        warnings
    }
}

/// Most samples one WAV written through hound can hold. Its data length is a
/// u32 that wraps silently past 4 GiB, and a wrapped header makes every reader
/// see only part of the file (a 3.5-hour stereo call read back as ~24 minutes).
/// A little room is left for the header chunks counted in the RIFF size.
pub fn wav_sample_capacity(spec: &hound::WavSpec) -> u64 {
    let bytes_per_sample = (spec.bits_per_sample as u64).div_ceil(8).max(1);
    (u32::MAX as u64 - 4096) / bytes_per_sample
}

/// The capacity of `spec` as a length people understand ("about 3.1 hours").
pub fn wav_capacity_label(spec: &hound::WavSpec) -> String {
    let frames = wav_sample_capacity(spec) / spec.channels.max(1) as u64;
    let hours = frames as f64 / spec.sample_rate.max(1) as f64 / 3600.0;
    format!("about {:.1} hours", (hours * 10.0).floor() / 10.0)
}

/// Error for a WAV that would hold more than `wav_sample_capacity` samples.
pub fn ensure_wav_fits(spec: &hound::WavSpec, samples: u64, what: &str) -> Result<(), String> {
    if samples > wav_sample_capacity(spec) {
        return Err(format!("{what} is longer than a single recording file can hold ({})", wav_capacity_label(spec)));
    }
    Ok(())
}

/// Sample formats `build_input_stream_f32` can open (everything it converts).
pub fn is_supported_input_format(format: cpal::SampleFormat) -> bool {
    use cpal::SampleFormat as F;
    matches!(format, F::F32 | F::F64 | F::I8 | F::I16 | F::I32 | F::I64 | F::U8 | F::U16 | F::U32 | F::U64)
}

/// Converts device samples to f32 in `out` (cleared first).
pub fn convert_samples_to_f32<T>(src: &[T], out: &mut Vec<f32>)
where
    T: cpal::Sample,
    f32: cpal::FromSample<T>,
{
    use cpal::FromSample;
    out.clear();
    out.extend(src.iter().map(|&s| f32::from_sample_(s)));
}

/// Opens an input stream in the device's own sample format and hands
/// `on_data` f32 samples. Asking cpal for f32 from a device configuration that
/// only delivers integers (common for USB mics and on Windows/Linux) fails to
/// open the stream.
pub fn build_input_stream_f32<D, E>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    on_data: D,
    on_error: E,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    D: FnMut(&[f32]) + Send + 'static,
    E: FnMut(cpal::StreamError) + Send + 'static,
{
    use cpal::traits::DeviceTrait;
    use cpal::SampleFormat as F;
    let mut on_data = on_data;
    match format {
        F::F32 => device.build_input_stream(config, move |data: &[f32], _: &cpal::InputCallbackInfo| on_data(data), on_error, None),
        F::F64 => build_converted::<f64, _, _>(device, config, on_data, on_error),
        F::I8 => build_converted::<i8, _, _>(device, config, on_data, on_error),
        F::I16 => build_converted::<i16, _, _>(device, config, on_data, on_error),
        F::I32 => build_converted::<i32, _, _>(device, config, on_data, on_error),
        F::I64 => build_converted::<i64, _, _>(device, config, on_data, on_error),
        F::U8 => build_converted::<u8, _, _>(device, config, on_data, on_error),
        F::U16 => build_converted::<u16, _, _>(device, config, on_data, on_error),
        F::U32 => build_converted::<u32, _, _>(device, config, on_data, on_error),
        F::U64 => build_converted::<u64, _, _>(device, config, on_data, on_error),
        _ => Err(cpal::BuildStreamError::StreamConfigNotSupported),
    }
}

fn build_converted<T, D, E>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut on_data: D,
    on_error: E,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
    D: FnMut(&[f32]) + Send + 'static,
    E: FnMut(cpal::StreamError) + Send + 'static,
{
    use cpal::traits::DeviceTrait;
    let mut buf: Vec<f32> = Vec::new();
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            convert_samples_to_f32(data, &mut buf);
            on_data(&buf);
        },
        on_error,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_capacity_matches_the_u32_data_length() {
        let call = hound::WavSpec { channels: 2, sample_rate: 48_000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        assert!(wav_sample_capacity(&call) * 4 <= u32::MAX as u64);
        assert_eq!(wav_capacity_label(&call), "about 3.1 hours");
        let import = hound::WavSpec { channels: 1, ..call };
        assert_eq!(wav_capacity_label(&import), "about 6.2 hours");
        assert!(ensure_wav_fits(&import, wav_sample_capacity(&import), "Import").is_ok());
        assert!(ensure_wav_fits(&import, wav_sample_capacity(&import) + 1, "Import").is_err());
    }

        #[test]
    fn full_file_queue_counts_frames_and_notifies_once() {
        let loss = CaptureLoss::default();
        let (tx, _rx) = crossbeam_channel::bounded(1);
        assert!(!loss.send_file(&tx, vec![0.0; 1920], 2));
        assert!(loss.send_file(&tx, vec![0.0; 1920], 2));
        assert!(!loss.send_file(&tx, vec![0.0; 1920], 2));
        assert_eq!(loss.file_frames.load(Ordering::Relaxed), 1920);
        assert!(loss.warnings()[0].contains("1920 frames"));
    }

    #[test]
    fn silence_filled_for_a_quiet_track_is_not_audio_loss() {
        let loss = CaptureLoss::default();
        loss.gap_frames.store(96_000, Ordering::Relaxed);
        assert!(loss.warnings().is_empty());
    }

    #[test]
    fn meeting_capture_failures_survive_teardown_without_duplicate_warnings() {
        let loss = CaptureLoss::default();
        loss.warn_meeting("Call audio interrupted".into());
        loss.warn_meeting("Call audio interrupted".into());
        assert_eq!(loss.warnings(), vec!["Call audio interrupted"]);
        loss.input_frames.store(960, Ordering::Relaxed);
        assert_eq!(loss.warnings().len(), 2);
    }

    #[test]
    fn loss_text_mentions_only_the_audio_that_was_actually_lost() {
        let input_only = CaptureLoss::default();
        input_only.input_frames.store(960, Ordering::Relaxed);
        let input_text = input_only.warnings().join(" ");
        assert!(input_text.contains("960"));
        assert!(!input_text.contains("0 frames could not be written"), "{input_text}");

        let file_only = CaptureLoss::default();
        file_only.file_frames.store(480, Ordering::Relaxed);
        let file_text = file_only.warnings().join(" ");
        assert!(file_text.contains("480"));
        assert!(!file_text.contains("0 input frames"), "{file_text}");
    }

        #[test]
    fn disconnected_writer_is_audio_loss_but_recoverable_preview_loss_is_not() {
        let loss = CaptureLoss::default();
        loss.preview_frames.store(5000, Ordering::Relaxed);
        assert!(loss.warnings().is_empty());
        let (tx, rx) = crossbeam_channel::bounded(1);
        drop(rx);
        assert!(loss.send_file(&tx, vec![0.0; 160], 1));
        assert_eq!(loss.file_frames.load(Ordering::Relaxed), 160);
        assert_eq!(loss.warnings().len(), 1);
    }

    #[test]
    fn integer_samples_convert_to_unit_range() {
        let mut out = Vec::new();
        convert_samples_to_f32(&[i16::MIN, 0, i16::MAX], &mut out);
        assert_eq!(out[0], -1.0);
        assert_eq!(out[1], 0.0);
        assert!((out[2] - 1.0).abs() < 1e-4);

        convert_samples_to_f32(&[0u16, 32768, u16::MAX], &mut out);
        assert_eq!(out.len(), 3, "buffer is reused, not appended to");
        assert_eq!(out[0], -1.0);
        assert_eq!(out[1], 0.0);

        convert_samples_to_f32(&[i32::MIN, i32::MAX / 2], &mut out);
        assert_eq!(out[0], -1.0);
        assert!((out[1] - 0.5).abs() < 1e-6);

        convert_samples_to_f32(&[0.25f32], &mut out);
        assert_eq!(out, vec![0.25]);
    }

    #[test]
    fn common_device_formats_are_supported() {
        use cpal::SampleFormat as F;
        for f in [F::F32, F::I16, F::I32, F::U16, F::U8] {
            assert!(is_supported_input_format(f), "{f:?}");
        }
    }
}
