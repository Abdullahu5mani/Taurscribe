use std::path::Path;
use taurscribe_lib::qwen3::{Qwen3Manager, MODEL_ID_QWEN3_1_7B_ONNX};

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let model_dir = args
        .next()
        .ok_or("usage: qwen3_transcribe_probe MODEL_DIR AUDIO [MODEL_ID]")?;
    let audio_path = args
        .next()
        .ok_or("usage: qwen3_transcribe_probe MODEL_DIR AUDIO [MODEL_ID]")?;
    let model_id = args.next();
    std::env::set_var("TAURSCRIBE_QWEN3_MODEL_DIR", &model_dir);

    let (audio, sample_rate) =
        taurscribe_lib::audio_decode::decode_audio_mono_f32(Path::new(&audio_path))?;
    let audio = if sample_rate == 16_000 {
        audio
    } else {
        taurscribe_lib::audio_preprocess::resample_mono_to_16k(&audio, sample_rate)?
    };
    let mut manager = Qwen3Manager::new();
    println!(
        "{}",
        manager.initialize(model_id.as_deref().or(Some(MODEL_ID_QWEN3_1_7B_ONNX)), false)?
    );
    let text = manager.transcribe_audio_data(&audio, Some("President Kennedy"))?;
    println!(
        "RESULT_JSON:{}",
        serde_json::json!({ "transcript": text, "status": manager.get_status() })
    );
    manager.unload();
    Ok(())
}
