use std::path::PathBuf;

/// VAD (Voice Activity Detection) Manager
///
/// NOTE: This is currently a stub implementation that will be enhanced
/// once we resolve the ONNX Runtime dependency issues.
/// For now, it provides a simple energy-based voice detection.
pub struct VADManager {
    threshold: f32,
}

impl VADManager {
    /// Create a new VAD manager
    pub fn new() -> Result<Self, String> {
        // Get the models directory path (optional - just for future Silero integration)
        if let Ok(models_dir) = Self::get_models_dir() {
            let vad_model_path = models_dir.join("silero_vad.onnx");

            if vad_model_path.exists() {
                println!(
                    "[VAD] Found Silero VAD model at: {}",
                    vad_model_path.display()
                );
            } else {
                println!(
                    "[VAD] Silero model not found at: {} (continuing with energy-based VAD)",
                    vad_model_path.display()
                );
            }
        } else {
            println!("[VAD] Models directory not accessible (using energy-based VAD)");
        }

        println!("[VAD] Using simple energy-based VAD (Silero integration pending)");

        Ok(Self {
            threshold: 0.005, // Energy threshold - lowered for better speech detection
        })
    }

    /// Get the models directory path (same logic as WhisperManager)
    fn get_models_dir() -> Result<PathBuf, String> {
        let possible_paths = [
            "taurscribe-runtime/models",
            "../taurscribe-runtime/models",
            "../../taurscribe-runtime/models",
        ];

        for path in possible_paths {
            if let Ok(canonical) = std::fs::canonicalize(path) {
                if canonical.is_dir() {
                    // Check if this directory contains silero_vad.onnx OR ggml models
                    if canonical.join("silero_vad.onnx").exists() {
                        return Ok(canonical);
                    }
                }
            }
        }

        Err("Could not find models directory".to_string())
    }

    /// Check if an audio chunk contains speech using simple energy detection
    /// Returns speech probability (0.0 = silence, 1.0 = definitely speech)
    ///
    /// NOTE: This is a simple implementation. Will be upgraded to use
    /// Silero VAD model once ONNX Runtime integration is complete.
    pub fn is_speech(&mut self, audio: &[f32]) -> Result<f32, String> {
        // Calculate RMS (Root Mean Square) energy
        let sum_squares: f32 = audio.iter().map(|&x| x * x).sum();
        let rms = (sum_squares / audio.len() as f32).sqrt();

        // Gradual probability (better than binary)
        let prob = if rms < self.threshold {
            0.0 // Very quiet - silence
        } else if rms > self.threshold * 5.0 {
            1.0 // Loud - definitely speech
        } else {
            // Maps threshold..threshold*5 to 0.0..1.0
            ((rms - self.threshold) / (self.threshold * 4.0)).min(1.0)
        };

        Ok(prob)
    }

    /// Get speech timestamps from full audio
    /// Returns list of (start_time, end_time) in seconds
    ///
    /// NOTE: This uses simple energy detection. Will be upgraded to use
    /// Silero VAD once ONNX Runtime integration is complete.
    #[allow(dead_code)]
    pub fn get_speech_timestamps(
        &mut self,
        audio: &[f32],
        padding_ms: usize,
    ) -> Result<Vec<(f32, f32)>, String> {
        const SAMPLE_RATE: f32 = 16000.0;
        const FRAME_SIZE: usize = 512; // ~32ms at 16kHz
        const MIN_SPEECH_FRAMES: usize = 5; // ~150ms minimum (reduced from 8)

        // Calculate padding in frames
        let frame_ms = (FRAME_SIZE as f32 / SAMPLE_RATE * 1000.0) as usize;
        let padding_frames = padding_ms / frame_ms;

        let mut segments = Vec::new();
        let mut speech_start: Option<usize> = None;
        let mut consecutive_speech_frames = 0;
        let mut silence_frames = 0;

        // Process audio in frames
        for (i, chunk) in audio.chunks(FRAME_SIZE).enumerate() {
            let is_speech = self.is_speech(chunk)? > 0.5;

            match (is_speech, speech_start) {
                (true, None) => {
                    // Speech potentially starting
                    consecutive_speech_frames = 1;
                    speech_start = Some(i);
                    silence_frames = 0;
                }
                (true, Some(_)) => {
                    // Speech continuing
                    consecutive_speech_frames += 1;
                    silence_frames = 0;
                }
                (false, Some(_)) => {
                    // Silence detected during speech segment
                    silence_frames += 1;

                    // Only end segment if silence exceeds padding
                    if silence_frames > padding_frames {
                        if consecutive_speech_frames >= MIN_SPEECH_FRAMES {
                            // Valid segment found
                            // Apply padding to start (go back padding_frames, but not < 0)
                            let start_idx = speech_start.unwrap().saturating_sub(padding_frames);
                            // Apply padding to end (current index is already padded by the wait)
                            let end_idx = i;

                            let start_time = (start_idx * FRAME_SIZE) as f32 / SAMPLE_RATE;
                            let end_time = (end_idx * FRAME_SIZE) as f32 / SAMPLE_RATE;

                            segments.push((start_time, end_time));
                        }

                        // Reset
                        speech_start = None;
                        consecutive_speech_frames = 0;
                        silence_frames = 0;
                    }
                }
                (false, None) => {
                    // Continuing silence
                }
            }
        }

        // Handle final segment if still ongoing
        if let Some(start_idx) = speech_start {
            if consecutive_speech_frames >= MIN_SPEECH_FRAMES {
                let start_idx = start_idx.saturating_sub(padding_frames);
                let start_time = (start_idx * FRAME_SIZE) as f32 / SAMPLE_RATE;
                let end_time = audio.len() as f32 / SAMPLE_RATE;
                segments.push((start_time, end_time));
            }
        }

        // Merge overlapping segments
        let mut merged_segments: Vec<(f32, f32)> = Vec::new();
        for segment in segments {
            if let Some(last) = merged_segments.last_mut() {
                if segment.0 <= last.1 {
                    // Overlap! Extend the previous segment
                    last.1 = segment.1.max(last.1);
                } else {
                    merged_segments.push(segment);
                }
            } else {
                merged_segments.push(segment);
            }
        }

        println!(
            "[VAD] Found {} speech segments (merged)",
            merged_segments.len()
        );
        for (i, (start, end)) in merged_segments.iter().enumerate() {
            println!("  Segment {}: {:.2}s - {:.2}s", i + 1, start, end);
        }

        Ok(merged_segments)
    }
}
