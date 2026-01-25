use std::path::PathBuf; // Import PathBuf for handling file system paths safely across different OSs

/// VAD (Voice Activity Detection) Manager
///
/// NOTE: This is currently a simple version (stub) that we will improve later.
/// Right now, it works by checking how "loud" the audio is (energy).
/// In the future, we will use an AI model (Silero) for better accuracy.
pub struct VADManager {
    threshold: f32, // The volume level that counts as "speech". 0.005 is a good default.
}

impl VADManager {
    /// Create a new VAD manager (Constructor)
    pub fn new() -> Result<Self, String> {
        // Try to find where the AI models are stored (for future use)
        if let Ok(models_dir) = Self::get_models_dir() {
            // Check if the Silero VAD model file exists in that directory
            let vad_model_path = models_dir.join("silero_vad.onnx");

            if vad_model_path.exists() {
                // We found the model! (Success case)
                println!(
                    "[VAD] Found Silero VAD model at: {}",
                    vad_model_path.display()
                );
            } else {
                // Model is missing, but that's okay for now
                println!(
                    "[VAD] Silero model not found at: {} (continuing with energy-based VAD)",
                    vad_model_path.display()
                );
            }
        } else {
            // Couldn't even find the model folder
            println!("[VAD] Models directory not accessible (using energy-based VAD)");
        }

        // Announce that we are using the simple energy method
        println!("[VAD] Using simple energy-based VAD (Silero integration pending)");

        // Return the new VADManager object initialized with our threshold
        Ok(Self {
            threshold: 0.005, // Set threshold to 0.005. Lowered this to catch quieter speech.
        })
    }

    /// Helper function to find the 'models' directory
    fn get_models_dir() -> Result<PathBuf, String> {
        // List of places where the models might be hiding relative to our app
        let possible_paths = [
            "taurscribe-runtime/models",       // Check current folder
            "../taurscribe-runtime/models",    // Check one level up
            "../../taurscribe-runtime/models", // Check two levels up
        ];

        // Loop through each possible path to see if it exists
        for path in possible_paths {
            // Try to convert relative path to absolute path (canonicalize)
            if let Ok(canonical) = std::fs::canonicalize(path) {
                // If the path is valid and is a directory...
                if canonical.is_dir() {
                    // Check if it looks like the right folder (contains silero model)
                    if canonical.join("silero_vad.onnx").exists() {
                        return Ok(canonical); // Found it! Return the full path.
                    }
                }
            }
        }

        // If we checked everywhere and found nothing, return an error
        Err("Could not find models directory".to_string())
    }

    /// The Main Function: Check if a chunk of audio is speech
    /// Returns a probability score (0.0 to 1.0)
    /// 0.0 = Silence
    /// 1.0 = Speech
    pub fn is_speech(&mut self, audio: &[f32]) -> Result<f32, String> {
        // Calculate the "Energy" (loudness) of the audio
        // 1. Square every sample (x * x) and add them all up
        let sum_squares: f32 = audio.iter().map(|&x| x * x).sum();

        // 2. Divide by number of samples (Mean) and take Square Root -> RMS (Root Mean Square)
        let rms = (sum_squares / audio.len() as f32).sqrt();

        // Convert that single RMS number into a probability (0.0 - 1.0)
        let prob = if rms < self.threshold {
            0.0 // It's too quiet -> definitely silence
        } else if rms > self.threshold * 5.0 {
            1.0 // It's very loud -> definitely speech
        } else {
            // It's in between. Map it to a value between 0.0 and 1.0
            // logic: (loudness - threshold) / range
            ((rms - self.threshold) / (self.threshold * 4.0)).min(1.0)
        };

        Ok(prob) // Return the result
    }

    /// Advanced Function: Find exactly WHEN speech happens in a full file
    /// Returns a list of (start_time, end_time) pairs in seconds
    #[allow(dead_code)] // Suppress warning if this function isn't used yet
    pub fn get_speech_timestamps(
        &mut self,
        audio: &[f32],     // The full audio recording data
        padding_ms: usize, // How much extra time to add around speech (for safety)
    ) -> Result<Vec<(f32, f32)>, String> {
        const SAMPLE_RATE: f32 = 16000.0; // Assume 16kHz audio (standard for AI)
        const FRAME_SIZE: usize = 512; // Check audio in chunks of 512 samples (~32ms)
        const MIN_SPEECH_FRAMES: usize = 5; // Must have ~150ms of speech to count as a real segment

        // Convert padding from milliseconds to number of frames
        // e.g., 500ms padding -> ~15 frames
        let frame_ms = (FRAME_SIZE as f32 / SAMPLE_RATE * 1000.0) as usize;
        let padding_frames = padding_ms / frame_ms;

        let mut segments = Vec::new(); // Where we'll store the results
        let mut speech_start: Option<usize> = None; // Start frame of current speech block
        let mut consecutive_speech_frames = 0; // How long have we been speaking?
        let mut silence_frames = 0; // How long has it been silent?

        // Loop through the audio in small "frame" chunks
        for (i, chunk) in audio.chunks(FRAME_SIZE).enumerate() {
            // Is this tiny chunk speech? (> 50% probability)
            let is_speech = self.is_speech(chunk)? > 0.5;

            // State Machine to track speech detection
            match (is_speech, speech_start) {
                (true, None) => {
                    // NEW SPEECH DETECTED! We weren't speaking, now we are.
                    consecutive_speech_frames = 1;
                    speech_start = Some(i); // Mark the start frame index
                    silence_frames = 0;
                }
                (true, Some(_)) => {
                    // STILL SPEAKING... We were already speaking.
                    consecutive_speech_frames += 1;
                    silence_frames = 0;
                }
                (false, Some(_)) => {
                    // SPEECH STOPPED (Temporarily?). usage: sentence pauses.
                    silence_frames += 1;

                    // If that pause lasts too long (more than our padding)... end the segment
                    if silence_frames > padding_frames {
                        // Was it a real sentence? (Was it long enough?)
                        if consecutive_speech_frames >= MIN_SPEECH_FRAMES {
                            // Yes! It was valid speech. Save it.

                            // Calculate start index (go back a bit for padding)
                            let start_idx = speech_start.unwrap().saturating_sub(padding_frames);
                            // End index is where we are now (current frame `i`)
                            let end_idx = i;

                            // Convert frame numbers to seconds (frame * size / rate)
                            let start_time = (start_idx * FRAME_SIZE) as f32 / SAMPLE_RATE;
                            let end_time = (end_idx * FRAME_SIZE) as f32 / SAMPLE_RATE;

                            // Add to our list
                            segments.push((start_time, end_time));
                        }

                        // Reset everything to look for next sentence
                        speech_start = None;
                        consecutive_speech_frames = 0;
                        silence_frames = 0;
                    }
                }
                (false, None) => {
                    // STILL SILENT. Nothing happening.
                }
            }
        }

        // Check if file ended while we were still speaking (handle the last segment)
        if let Some(start_idx) = speech_start {
            if consecutive_speech_frames >= MIN_SPEECH_FRAMES {
                let start_idx = start_idx.saturating_sub(padding_frames);
                let start_time = (start_idx * FRAME_SIZE) as f32 / SAMPLE_RATE;
                let end_time = audio.len() as f32 / SAMPLE_RATE;
                segments.push((start_time, end_time));
            }
        }

        // Clean up: Merge segments that are overlapping or touching
        // Example: Seg1 ends at 5.0s, Seg2 starts at 4.8s -> Merge them!
        let mut merged_segments: Vec<(f32, f32)> = Vec::new();
        for segment in segments {
            if let Some(last) = merged_segments.last_mut() {
                // If this segment starts before the last one ends...
                if segment.0 <= last.1 {
                    // Merge them! Extend the last one to cover this one too.
                    last.1 = segment.1.max(last.1);
                } else {
                    // No overlap, add as a new separate segment
                    merged_segments.push(segment);
                }
            } else {
                // First segment
                merged_segments.push(segment);
            }
        }

        // Print debug info about what we found
        println!(
            "[VAD] Found {} speech segments (merged)",
            merged_segments.len()
        );
        for (i, (start, end)) in merged_segments.iter().enumerate() {
            println!("  Segment {}: {:.2}s - {:.2}s", i + 1, start, end);
        }

        Ok(merged_segments) // Return the final list
    }
}
