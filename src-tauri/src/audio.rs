use crossbeam_channel::Sender;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

// Wrapper struct to make the Audio Stream "moveable" between threads.
// By default, raw pointers/streams aren't thread-safe.
// We implement Send and Sync manually (unsafe) to tell Rust "Check constraints are met".
// Held in RecordingHandle solely for RAII ownership — dropped when recording stops.
pub struct SendStream(pub cpal::Stream);
unsafe impl Send for SendStream {} // Can be moved to another thread
unsafe impl Sync for SendStream {} // Can be accessed from multiple threads

/// Keeps track of the tools needed while recording involves.
pub struct RecordingHandle {
    pub stream: Option<SendStream>, // The actual connection to the microphone hardware (None in dual-channel mode)
    pub file_tx: Sender<Vec<f32>>, // Pipe to send audio to the "File Writer" thread
    pub whisper_tx: Sender<Vec<f32>>, // Pipe to send audio to the "Whisper AI" thread
    pub writer_thread: std::thread::JoinHandle<()>,
    pub transcriber_thread: std::thread::JoinHandle<()>,
    pub level_stop: Arc<AtomicBool>, // Signal the level-emitter thread to exit
    pub level_thread: std::thread::JoinHandle<()>,
    pub is_dual_channel: bool,
    pub dual_channel_stop: Option<Arc<AtomicBool>>,
    pub dual_channel_thread: Option<std::thread::JoinHandle<()>>,
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
