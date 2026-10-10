//! Optional, user-invoked two-source listening aid. Never substitutes a source
//! for the original transcript or enrolls an inferred identity automatically.
use std::path::Path;
pub const MODEL_ID:&str="voice-separation";
pub const MODEL_FILE:&str="sepformer_int8.onnx";

pub fn separate_pcm(model:&Path, pcm:&[f32])->Result<[Vec<f32>;2],String>{
    if pcm.is_empty() || pcm.len()>16_000*15 {return Err("Choose a voice preview of at most 15 seconds".into());}
    let mut session=ort::session::Session::builder().map_err(|e|e.to_string())?
        .with_intra_threads(4).map_err(|e|e.to_string())?
        .commit_from_file(model).map_err(|e|format!("Voice separation model could not load. Re-download it: {e}"))?;
    let input_audio=crate::audio_preprocess::resample_mono_ratio(pcm,16000,8000)?;
    let input=ort::value::Value::from_array((vec![1usize,input_audio.len()],input_audio.clone())).map_err(|e|e.to_string())?;
    let outputs=session.run(ort::inputs![input]).map_err(|e|format!("Voice separation failed: {e}"))?;
    let (_,value)=outputs.iter().next().ok_or("No separated audio returned")?;
    let (shape,data)=value.try_extract_tensor::<f32>().map_err(|e|e.to_string())?;
    if shape.len()!=3 || shape[0]!=1 || shape[2]!=2 || shape[1]<(input_audio.len() as i64) {return Err("Unexpected voice-separation output shape".into());}
    if data.iter().any(|value|!value.is_finite()) {return Err("The separator produced invalid audio; original preview is unchanged".into());}
    let mut sources=[Vec::with_capacity(pcm.len()),Vec::with_capacity(pcm.len())];
    for (index,source) in sources.iter_mut().enumerate() {
        let low_rate:Vec<_>=data.chunks_exact(2).take(input_audio.len()).map(|frame|frame[index]).collect();
        *source=crate::audio_preprocess::resample_mono_ratio(&low_rate,8000,16000)?;
        source.resize(pcm.len(),0.0);source.truncate(pcm.len());

    }
    // SI-SNR-trained separators do not promise playback-calibrated amplitude.
    // Apply one common gain after resampling; clipping each stream independently
    // distorts the speech and changes relative speaker levels.
    let output_peak=sources.iter().flatten().map(|sample|sample.abs()).fold(0.0f32,f32::max);
    let target_peak=pcm.iter().map(|sample|sample.abs()).fold(0.0f32,f32::max).min(0.95);
    if output_peak>0.0 {
        let gain=(target_peak/output_peak).min(1.0);
        for sample in sources.iter_mut().flatten(){*sample *= gain;}
    }
    Ok(sources)
}

#[tauri::command]
pub fn voice_separation_available()->bool{
    crate::utils::get_models_dir().ok().is_some_and(|root|root.join(MODEL_ID).join(MODEL_FILE).is_file())
}

#[tauri::command]
pub async fn separate_meeting_sample(path:String)->Result<Vec<String>,String>{
    tauri::async_runtime::spawn_blocking(move||{
        static SEPARATION:std::sync::Mutex<()>=std::sync::Mutex::new(());
        let _operation=SEPARATION.try_lock().map_err(|_|"Voice separation is already running")?;
        // Read app-managed clips only; do not accept arbitrary large uploads.
        let root=crate::commands::meetings::get_meetings_dir()?;
        let file=Path::new(&path);
        let canonical=file.canonicalize().map_err(|e|e.to_string())?;
        let allowed=[root.join("snippets"),root.join("voicebank")].iter()
            .filter_map(|dir|dir.canonicalize().ok()).any(|dir|canonical.starts_with(dir));
        if !allowed {return Err("Select a meeting voice preview before isolating voices".into());}
        if std::fs::metadata(&canonical).map_err(|e|e.to_string())?.len()>16*1024*1024 {return Err("Voice preview is too large".into());}
        let (pcm,rate)=crate::audio_decode::decode_audio_mono_f32(&canonical)?;
        let pcm=crate::audio_preprocess::resample_mono_to_16k(&pcm,rate)?;
        let model=crate::utils::get_models_dir()?.join(MODEL_ID).join(MODEL_FILE);
        let sources=separate_pcm(&model,&pcm)?;
        let dir=root.join("voicebank");std::fs::create_dir_all(&dir).map_err(|e|e.to_string())?;
        use sha2::{Digest,Sha256};let mut hash=Sha256::new();hash.update(MODEL_ID.as_bytes());
        for sample in &pcm {hash.update(sample.to_le_bytes());}
        let key=format!("{:x}",hash.finalize());
        let mut written=Vec::new();
        for (index,source) in sources.iter().enumerate(){
            let destination=dir.join(format!("sample-separated-{key}-{index}.wav"));
            let result=(||{
                let spec=hound::WavSpec{channels:1,sample_rate:16000,bits_per_sample:32,sample_format:hound::SampleFormat::Float};
                let mut writer=hound::WavWriter::create(&destination,spec).map_err(|e|e.to_string())?;
                for sample in source {writer.write_sample(*sample).map_err(|e|e.to_string())?;}
                writer.finalize().map_err(|e|e.to_string())
            })();
            if let Err(error)=result{let _=std::fs::remove_file(&destination);for path in &written{let _=std::fs::remove_file(path);}return Err(error);}
            written.push(destination.to_string_lossy().into_owned());
        }
        Ok(written)
    }).await.map_err(|e|e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_or_unbounded_input_is_rejected_before_model_loading(){
        assert!(separate_pcm(Path::new("missing"),&[]).unwrap_err().contains("15 seconds"));
        assert!(separate_pcm(Path::new("missing"),&vec![0.0;16_000*16]).unwrap_err().contains("15 seconds"));
    }
    #[test]
    #[ignore="requires downloaded ONNX fixture; tests real two-voice separation"]
    fn native_separation_produces_two_aligned_distinct_finite_streams(){
        let model=std::env::var("TAURSCRIBE_SEPARATION_TEST_MODEL").expect("test model path");
        let root=Path::new(env!("CARGO_MANIFEST_DIR")).join("../taurscribe-runtime/samples");
        let load=|name:&str|{let (pcm,rate)=crate::audio_decode::decode_audio_mono_f32(&root.join(name)).unwrap();crate::audio_preprocess::resample_mono_to_16k(&pcm,rate).unwrap()};
        let load_external=|key:&str,name:&str| std::env::var(key).ok().map(|path|{
            let (pcm,rate)=crate::audio_decode::decode_audio_mono_f32(Path::new(&path)).unwrap();crate::audio_preprocess::resample_mono_to_16k(&pcm,rate).unwrap()
        }).unwrap_or_else(||load(name));
        let a=load_external("TAURSCRIBE_SEPARATION_TEST_SPEAKER_A","jfk.wav");
        let b=load_external("TAURSCRIBE_SEPARATION_TEST_SPEAKER_B","otherjack.wav");
        let length=a.len().min(b.len()).min(16_000*10);
        let mixture:Vec<_>=a[..length].iter().zip(&b[..length]).map(|(a,b)|a*0.5+b*0.5).collect();
        let start=std::time::Instant::now();
        let sources=separate_pcm(Path::new(&model),&mixture).unwrap();
        for source in &sources {assert_eq!(source.len(),length);assert!(source.iter().all(|v|v.is_finite()));assert!(source.iter().any(|v|v.abs()>0.01));}
        let difference=sources[0].iter().zip(&sources[1]).map(|(a,b)|(a-b).abs()).sum::<f32>()/length as f32;
        assert!(difference>0.0001,"streams are identical");
        let peak=sources.iter().flatten().map(|v|v.abs()).fold(0.0f32,f32::max);
        let input_peak=mixture.iter().map(|v|v.abs()).fold(0.0f32,f32::max);
        assert!(peak<=input_peak+0.001 && peak<=0.95,"preview gain is not calibrated");
        fn si(estimated:&[f32],reference:&[f32])->f32 {
            let dot=estimated.iter().zip(reference).map(|(a,b)|a*b).sum::<f32>();
            let ref_energy=reference.iter().map(|v|v*v).sum::<f32>().max(1e-12);
            let scale=dot/ref_energy;
            let signal=scale*scale*ref_energy;
            let noise=estimated.iter().zip(reference).map(|(a,b)|(a-scale*b).powi(2)).sum::<f32>();
            10.0*(signal.max(1e-12)/noise.max(1e-12)).log10()
        }
        let direct=(si(&sources[0],&a[..length])+si(&sources[1],&b[..length]))/2.0;
        let swapped=(si(&sources[1],&a[..length])+si(&sources[0],&b[..length]))/2.0;
        let baseline=(si(&mixture,&a[..length])+si(&mixture,&b[..length]))/2.0;
        let improvement=direct.max(swapped)-baseline;
        if std::env::var("TAURSCRIBE_SEPARATION_TEST_SPEAKER_A").is_ok() {assert!(improvement>6.0,"separation did not improve two-voice fixture: {improvement} dB");}
        println!("NATIVE_SEPARATION improvement_db={improvement} peak={peak}");
        println!("NATIVE_SEPARATION seconds={} elapsed={} difference={difference}",length as f32/16000.0,start.elapsed().as_secs_f32());
    }
}
