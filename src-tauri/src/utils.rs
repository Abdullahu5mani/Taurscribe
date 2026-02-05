

/// Simple Post-Processing to clean up raw ASR artifacts
pub fn clean_transcript(text: &str) -> String {
    let mut cleaned = text.trim().to_string();

    // Fix floating punctuation
    cleaned = cleaned.replace(" ,", ",");
    cleaned = cleaned.replace(" .", ".");
    cleaned = cleaned.replace(" ?", "?");
    cleaned = cleaned.replace(" !", "!");

    // Fix percent signs
    cleaned = cleaned.replace(" %", "%");

    // Fix double spaces
    while cleaned.contains("  ") {
        cleaned = cleaned.replace("  ", " ");
    }

    // Capitalize first letter
    if let Some(first) = cleaned.chars().next() {
        if first.is_lowercase() {
            let mut c = cleaned.chars();
            cleaned = match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            };
        }
    }

    cleaned
}

/// Helper: Find or create the directory to save recordings
pub fn get_recordings_dir() -> Result<std::path::PathBuf, String> {
    // Get the standard AppData folder (C:\Users\Name\AppData\Local)
    let app_data = dirs::data_local_dir().ok_or("Could not find AppData directory")?;

    // Append our specific folder: ...\Taurscribe\temp
    let recordings_dir = app_data.join("Taurscribe").join("temp");

    // Create folder if it doesn't exist
    std::fs::create_dir_all(&recordings_dir)
        .map_err(|e| format!("Failed to create recordings directory: {}", e))?;

    Ok(recordings_dir)
}

/// Helper: Find or create the directory to save models
pub fn get_models_dir() -> Result<std::path::PathBuf, String> {
    // Get the standard AppData folder (C:\Users\Name\AppData\Local)
    let app_data = dirs::data_local_dir().ok_or("Could not find AppData directory")?;

    // Append our specific folder: ...\Taurscribe\models
    let models_dir = app_data.join("Taurscribe").join("models");

    // Create folder if it doesn't exist
    std::fs::create_dir_all(&models_dir)
        .map_err(|e| format!("Failed to create models directory: {}", e))?;

    Ok(models_dir)
}
