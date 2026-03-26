/// VAD (Voice Activity Detection) Manager
///
/// Uses Silero VAD v4 (ONNX) running on ORT for neural speech detection.
/// Falls back to a simple energy-based VAD if the ORT session fails to load.
///
/// Silero VAD is an LSTM-based model that processes 512-sample (32ms @ 16kHz) frames
/// and returns a speech probability 0.0–1.0 while maintaining GRU hidden state across
/// frames so that context is preserved within a single recording session.
use ort::inputs;
use ort::session::Session;
use ort::value::Tensor;

/// Silero VAD v4 ONNX model compiled into the binary (~2.3 MB).
static SILERO_BYTES: &[u8] = include_bytes!("../resources/silero_vad.onnx");

/// Silero VAD processes 512 samples per frame at 16 kHz (32 ms).
const CHUNK_SIZE: usize = 512;
/// LSTM hidden/cell state size: shape [2, 1, 64] = 128 f32 values.
const STATE_SIZE: usize = 128;

pub struct VADManager {
    /// Loaded ORT session, None if initialization failed (falls back to energy VAD).
    session: Option<Session>,
    /// LSTM hidden state h — shape [2, 1, 64], flattened to 128 f32s.
    h: Vec<f32>,
    /// LSTM cell state c — shape [2, 1, 64], flattened to 128 f32s.
    c: Vec<f32>,
}

impl VADManager {
    pub fn new() -> Result<Self, String> {
        let session = match Session::builder() {
            Ok(builder) => match builder.commit_from_memory(SILERO_BYTES) {
                Ok(s) => {
                    println!("[VAD] Silero VAD initialized (ORT session ready)");
                    Some(s)
                }
                Err(e) => {
                    println!(
                        "[VAD] Silero VAD ORT session failed ({}), using energy-based fallback",
                        e
                    );
                    None
                }
            },
            Err(e) => {
                println!(
                    "[VAD] Silero VAD Session::builder() failed ({}), using energy-based fallback",
                    e
                );
                None
            }
        };

        Ok(Self {
            session,
            h: vec![0.0_f32; STATE_SIZE],
            c: vec![0.0_f32; STATE_SIZE],
        })
    }

    /// Reset the LSTM state — call this at the start of every new recording session
    /// so context from a previous session does not bleed into the next one.
    pub fn reset_state(&mut self) {
        self.h.fill(0.0);
        self.c.fill(0.0);
    }

    /// Return a speech probability for `audio` (0.0 = silence, 1.0 = speech).
    ///
    /// When using Silero VAD, only the first `CHUNK_SIZE` samples are evaluated per
    /// call (to match the model's fixed input window). The LSTM state is updated in-place
    /// so successive calls share temporal context across a recording session.
    pub fn is_speech(&mut self, audio: &[f32]) -> Result<f32, String> {
        if self.session.is_some() {
            self.run_silero(audio)
        } else {
            Ok(Self::energy_vad(audio))
        }
    }

    /// Run one Silero VAD inference step and update the internal LSTM state.
    fn run_silero(&mut self, audio: &[f32]) -> Result<f32, String> {
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| "Silero session not initialized".to_string())?;
        // Pad or truncate to exactly CHUNK_SIZE samples
        let mut chunk = vec![0.0_f32; CHUNK_SIZE];
        let copy_len = audio.len().min(CHUNK_SIZE);
        chunk[..copy_len].copy_from_slice(&audio[..copy_len]);

        // input: [1, CHUNK_SIZE]
        let input_tensor = Tensor::from_array(([1_usize, CHUNK_SIZE], chunk.into_boxed_slice()))
            .map_err(|e| format!("Silero input tensor error: {}", e))?;

        // sr: int64 — 0-dim scalar tensor (scalar has empty shape)
        let sr_tensor = Tensor::from_array((Vec::<i64>::new(), vec![16000_i64]))
            .map_err(|e| format!("Silero sr tensor error: {}", e))?;

        // h / c: [2, 1, 64] — copy into Box<[f32]> in one allocation (no intermediate Vec)
        let h_tensor = Tensor::from_array(([2_usize, 1, 64], Box::from(self.h.as_slice())))
            .map_err(|e| format!("Silero h tensor error: {}", e))?;
        let c_tensor = Tensor::from_array(([2_usize, 1, 64], Box::from(self.c.as_slice())))
            .map_err(|e| format!("Silero c tensor error: {}", e))?;

        let outputs = session
            .run(inputs![
                "input" => input_tensor,
                "sr"    => sr_tensor,
                "h"     => h_tensor,
                "c"     => c_tensor,
            ])
            .map_err(|e| format!("Silero run error: {}", e))?;

        // Extract speech probability [1, 1]
        let out_tensor = outputs["output"]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Silero output extract error: {}", e))?;
        let (_, out_data) = out_tensor;
        let prob = match out_data.first().copied() {
            Some(p) => p,
            None => {
                eprintln!("[VAD] Silero output tensor was empty — returning 0.0 for this frame");
                0.0
            }
        };

        // Update LSTM state from hn / cn — copy into existing buffers, no allocation
        if let Ok(hn) = outputs["hn"].try_extract_tensor::<f32>() {
            let (_, data) = hn;
            if data.len() == STATE_SIZE {
                self.h.copy_from_slice(data);
            }
        }
        if let Ok(cn) = outputs["cn"].try_extract_tensor::<f32>() {
            let (_, data) = cn;
            if data.len() == STATE_SIZE {
                self.c.copy_from_slice(data);
            }
        }

        Ok(prob)
    }

    /// Simple energy-based fallback used when the ORT session is unavailable.
    fn energy_vad(audio: &[f32]) -> f32 {
        if audio.is_empty() {
            return 0.0;
        }
        let rms = (audio.iter().map(|&x| x * x).sum::<f32>() / audio.len() as f32).sqrt();
        let threshold = 0.005_f32;
        if rms < threshold {
            0.0
        } else if rms > threshold * 5.0 {
            1.0
        } else {
            ((rms - threshold) / (threshold * 4.0)).min(1.0)
        }
    }

    /// Variant of `get_speech_timestamps` that allows overriding the internal
    /// probability threshold for a single pass. Used by file transcription to
    /// run with a more permissive threshold without affecting live VAD.
    pub fn get_speech_timestamps_with_threshold(
        &mut self,
        audio: &[f32],
        padding_ms: usize,
        threshold: f32,
    ) -> Result<Vec<(f32, f32)>, String> {
        self.get_speech_timestamps_hysteresis(audio, padding_ms, threshold, threshold)
    }

    /// Hysteresis-based segment finder — the canonical way to use Silero VAD.
    ///
    /// A segment STARTS when `prob > onset` and ENDS when `prob < offset` persists
    /// for longer than `padding_ms`.  Using a higher onset than offset prevents
    /// background noise (which sits between offset and onset) from either
    /// starting spurious segments or keeping real segments open indefinitely.
    ///
    /// File transcription tuning lives in `commands/file_transcription.rs`
    /// (`assemble_speech_audio`: onset/offset/hangover for speech-in / speech-out).
    pub fn get_speech_timestamps_hysteresis(
        &mut self,
        audio: &[f32],
        padding_ms: usize,
        onset: f32,
        offset: f32,
    ) -> Result<Vec<(f32, f32)>, String> {
        const SAMPLE_RATE: f32 = 16000.0;
        const MIN_SPEECH_FRAMES: usize = 2; // ~64 ms minimum to count as real speech

        let frame_ms = (CHUNK_SIZE as f32 / SAMPLE_RATE * 1000.0) as usize;
        let padding_frames = padding_ms / frame_ms.max(1);

        // Reset LSTM state so this offline pass starts clean
        self.reset_state();

        let mut segments = Vec::new();
        let mut speech_start: Option<usize> = None;
        let mut consecutive_speech = 0usize;
        let mut below_offset_frames = 0usize;
        let mut max_prob: f32 = 0.0;
        let mut frame_count: usize = 0;

        for (i, chunk) in audio.chunks(CHUNK_SIZE).enumerate() {
            let prob = self.is_speech(chunk).unwrap_or(0.0);
            max_prob = max_prob.max(prob);
            frame_count += 1;

            match speech_start {
                None => {
                    // Not in a speech segment: wait for onset threshold
                    if prob > onset {
                        speech_start = Some(i);
                        consecutive_speech = 1;
                        below_offset_frames = 0;
                    }
                }
                Some(_) => {
                    // Inside a speech segment: end when prob stays below offset long enough
                    if prob >= offset {
                        consecutive_speech += 1;
                        below_offset_frames = 0;
                    } else {
                        below_offset_frames += 1;
                        if below_offset_frames > padding_frames {
                            if consecutive_speech >= MIN_SPEECH_FRAMES {
                                let start_idx =
                                    speech_start.unwrap().saturating_sub(padding_frames);
                                let end_idx = i;
                                segments.push((
                                    (start_idx * CHUNK_SIZE) as f32 / SAMPLE_RATE,
                                    (end_idx * CHUNK_SIZE) as f32 / SAMPLE_RATE,
                                ));
                            }
                            speech_start = None;
                            consecutive_speech = 0;
                            below_offset_frames = 0;
                        }
                    }
                }
            }
        }

        // Flush any trailing speech segment
        if let Some(start_idx) = speech_start {
            if consecutive_speech >= MIN_SPEECH_FRAMES {
                let start_idx = start_idx.saturating_sub(padding_frames);
                segments.push((
                    (start_idx * CHUNK_SIZE) as f32 / SAMPLE_RATE,
                    audio.len() as f32 / SAMPLE_RATE,
                ));
            }
        }

        // Merge overlapping or adjacent segments
        let mut merged: Vec<(f32, f32)> = Vec::new();
        for seg in segments {
            if let Some(last) = merged.last_mut() {
                if seg.0 <= last.1 {
                    last.1 = seg.1.max(last.1);
                    continue;
                }
            }
            merged.push(seg);
        }

        println!(
            "[VAD] Found {} speech segment(s) (onset={}, offset={}, max_prob={:.3}, frames={})",
            merged.len(),
            onset,
            offset,
            max_prob,
            frame_count,
        );
        for (i, (s, e)) in merged.iter().enumerate() {
            println!("  Segment {}: {:.2}s – {:.2}s", i + 1, s, e);
        }

        Ok(merged)
    }
}
