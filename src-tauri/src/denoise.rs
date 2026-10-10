use nnnoiseless::DenoiseState;

/// RNNoise requires exactly 480 samples per frame at 48 kHz.
const FRAME_SIZE: usize = 480;

/// Real-time noise suppressor wrapping RNNoise (nnnoiseless).
///
/// RNNoise is stateful — its internal GRU carries context between frames,
/// so a fresh `Denoiser` must be created for each recording session.
pub struct Denoiser {
    state: Box<DenoiseState<'static>>,
    /// Leftover samples from the previous `process` call that didn't fill a full frame.
    remainder: Vec<f32>,
}

impl Denoiser {
    pub fn new() -> Self {
        Self {
            state: DenoiseState::new(),
            remainder: Vec::with_capacity(FRAME_SIZE),
        }
    }

    /// Denoise an arbitrarily-sized chunk of mono f32 audio at 48 kHz.
    ///
    /// Buffers leftover samples between calls so callers don't need to worry
    /// about frame alignment. Returns all complete denoised frames; any
    /// trailing sub-frame samples are kept for the next call.
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let total = self.remainder.len() + input.len();
        let full_frames = total / FRAME_SIZE;
        let mut output = Vec::with_capacity(full_frames * FRAME_SIZE);

        let mut src: &[f32] = input;

        let mut out_frame = [0.0f32; FRAME_SIZE];

        // If we have leftover samples from last time, complete the first frame.
        if !self.remainder.is_empty() {
            let need = FRAME_SIZE - self.remainder.len();
            if src.len() >= need {
                self.remainder.extend_from_slice(&src[..need]);
                src = &src[need..];

                process_normalized_frame(&mut self.state, &mut out_frame, &self.remainder);
                output.extend_from_slice(&out_frame);
                self.remainder.clear();
            } else {
                self.remainder.extend_from_slice(src);
                return output;
            }
        }

        // Process as many full frames as possible from the remaining input.
        while src.len() >= FRAME_SIZE {
            process_normalized_frame(&mut self.state, &mut out_frame, &src[..FRAME_SIZE]);
            output.extend_from_slice(&out_frame);
            src = &src[FRAME_SIZE..];
        }

        // Stash any leftover sub-frame samples for next call.
        if !src.is_empty() {
            self.remainder.extend_from_slice(src);
        }

        output
    }
}


/// nnnoiseless accepts float samples in the i16 amplitude range. Keep the
/// application's normalized PCM contract on both sides of that API boundary.
fn process_normalized_frame(state: &mut DenoiseState<'_>, output: &mut [f32; FRAME_SIZE], input: &[f32]) {
    let mut scaled = [0.0; FRAME_SIZE];
    for (dst, sample) in scaled.iter_mut().zip(input) { *dst = *sample * 32768.0; }
    state.process_frame(output, &scaled);
    for sample in output { *sample /= 32768.0; }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalized_speech_matches_rnnoise_pcm_contract_across_chunk_boundaries() {
        let mut reader = hound::WavReader::new(std::io::Cursor::new(include_bytes!("../../taurscribe-runtime/samples/jfk.wav").as_slice())).unwrap();
        let speech: Vec<f32> = reader.samples::<i16>().map(|sample|sample.unwrap() as f32 / 32768.0).collect();
        let speech = crate::audio_preprocess::resample_mono_ratio(&speech,16000,48000).unwrap();
        let mut reference=DenoiseState::new();let mut expected=Vec::new();let mut frame=[0.0;FRAME_SIZE];
        for chunk in speech.chunks_exact(FRAME_SIZE) {
            let pcm:Vec<_>=chunk.iter().map(|sample|sample*32768.0).collect();
            reference.process_frame(&mut frame,&pcm);
            expected.extend(frame.iter().map(|sample|sample/32768.0));
        }
        let mut actual=Vec::new();let mut denoiser=Denoiser::new();
        for chunk in speech.chunks(713) { actual.extend(denoiser.process(chunk)); }
        assert_eq!(actual,expected);
        assert!(actual.iter().all(|sample|sample.is_finite()));
        let energy:f64=actual.iter().map(|sample|(*sample as f64).powi(2)).sum();
        let original:f64=speech.iter().map(|sample|(*sample as f64).powi(2)).sum();
        assert!(energy>original*0.05,"speech must not collapse to near silence");
    }
}
