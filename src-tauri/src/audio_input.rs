//! Open input devices in their native sample format, then normalize for the pipeline.
use cpal::traits::DeviceTrait;
use cpal::{FromSample, Sample, SampleFormat};

fn normalize<T: Sample + Copy>(samples: &[T]) -> Vec<f32>
where
    f32: FromSample<T>,
{
    samples.iter().map(|sample| sample.to_sample::<f32>()).collect()
}

pub fn build_input_stream(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    mut on_audio: impl FnMut(&[f32], &cpal::InputCallbackInfo) + Send + 'static,
    on_error: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    macro_rules! typed_stream {
        ($sample:ty) => {
            device.build_input_stream(
                &config.config(),
                move |data: &[$sample], info| on_audio(&normalize(data), info),
                on_error,
                None,
            )
        };
    }
    match config.sample_format() {
        SampleFormat::F32 => device.build_input_stream(&config.config(), on_audio, on_error, None),
        SampleFormat::I8 => typed_stream!(i8),
        SampleFormat::I16 => typed_stream!(i16),
        SampleFormat::I32 => typed_stream!(i32),
        SampleFormat::I64 => typed_stream!(i64),
        SampleFormat::U8 => typed_stream!(u8),
        SampleFormat::U16 => typed_stream!(u16),
        SampleFormat::U32 => typed_stream!(u32),
        SampleFormat::U64 => typed_stream!(u64),
        SampleFormat::F64 => typed_stream!(f64),
        _ => Err(cpal::BuildStreamError::StreamConfigNotSupported),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_microphone_samples_preserve_silence_and_polarity() {
        assert_eq!(normalize(&[i16::MIN, -16384, 0, 16384]), [-1.0, -0.5, 0.0, 0.5]);
        assert_eq!(normalize(&[0_u16, 16384, 32768, 49152]), [-1.0, -0.5, 0.0, 0.5]);
        assert_eq!(normalize(&[0_u8, 128, 192]), [-1.0, 0.0, 0.5]);
    }
}
