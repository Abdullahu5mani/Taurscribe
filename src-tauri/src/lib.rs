use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{unbounded, Sender};
use std::sync::Mutex;
use tauri::State;

// Wrapper to make cpal::Stream Send/Sync.
// Safety: We only use this to keep the stream alive and drop it.
#[allow(dead_code)]
struct SendStream(cpal::Stream);
unsafe impl Send for SendStream {}
unsafe impl Sync for SendStream {}

struct AudioState {
    recording_handle: Mutex<Option<(SendStream, Sender<Vec<f32>>)>>,
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn start_recording(state: State<AudioState>) -> Result<String, String> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or("No input device")?;
    let config: cpal::StreamConfig = device
        .default_input_config()
        .map_err(|e| e.to_string())?
        .into();

    let filename = format!("recording_{}.wav", chrono::Utc::now().timestamp());
    let path = std::path::Path::new(".").join(&filename);

    let spec = hound::WavSpec {
        channels: config.channels,
        sample_rate: config.sample_rate.0,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };

    let writer = hound::WavWriter::create(&path, spec).map_err(|e| e.to_string())?;
    let (tx, rx) = unbounded::<Vec<f32>>();
    let tx_audio = tx.clone();

    // Spawn a thread to write audio to disk
    std::thread::spawn(move || {
        let mut writer = writer;
        while let Ok(samples) = rx.recv() {
            for sample in samples {
                writer.write_sample(sample).ok();
            }
        }
        writer.finalize().ok();
        println!("WAV file saved.");
    });

    let stream = device
        .build_input_stream(
            &config,
            move |data: &[f32], _: &_| {
                tx_audio.send(data.to_vec()).ok();
            },
            move |err| {
                eprintln!("Audio input error: {}", err);
            },
            None,
        )
        .map_err(|e| e.to_string())?;

    stream.play().map_err(|e| e.to_string())?;

    *state.recording_handle.lock().unwrap() = Some((SendStream(stream), tx));

    Ok(format!("Recording started: {}", path.display()))
}

#[tauri::command]
fn stop_recording(state: State<AudioState>) -> Result<String, String> {
    let mut handle = state.recording_handle.lock().unwrap();
    if let Some((stream, tx)) = handle.take() {
        drop(stream); // Stop capturing
        drop(tx); // Close channel -> Writer thread finishes
        Ok("Recording saved.".to_string())
    } else {
        Err("Not recording".to_string())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AudioState {
            recording_handle: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            start_recording,
            stop_recording
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
