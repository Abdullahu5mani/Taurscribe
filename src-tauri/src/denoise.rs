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

                self.state.process_frame(&mut out_frame, &self.remainder);
                output.extend_from_slice(&out_frame);
                self.remainder.clear();
            } else {
                self.remainder.extend_from_slice(src);
                return output;
            }
        }

        // Process as many full frames as possible from the remaining input.
        while src.len() >= FRAME_SIZE {
            self.state.process_frame(&mut out_frame, &src[..FRAME_SIZE]);
            output.extend_from_slice(&out_frame);
            src = &src[FRAME_SIZE..];
        }

        // Stash any leftover sub-frame samples for next call.
        if !src.is_empty() {
            self.remainder.extend_from_slice(src);
        }

        output
    }

    /// Flush any buffered remainder by zero-padding to a full frame.
    /// Call once at end-of-stream if you need every last sample.
    #[allow(dead_code)]
    pub fn flush(&mut self) -> Vec<f32> {
        if self.remainder.is_empty() {
            return Vec::new();
        }
        let valid = self.remainder.len();
        self.remainder.resize(FRAME_SIZE, 0.0);
        let mut out_frame = [0.0f32; FRAME_SIZE];
        self.state.process_frame(&mut out_frame, &self.remainder);
        self.remainder.clear();
        out_frame[..valid].to_vec()
    }
}
