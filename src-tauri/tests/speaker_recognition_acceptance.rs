//! Explicit neural speaker acceptance check on local human-voice fixtures.
//! TAURSCRIBE_SPEAKER_FIXTURES points to JSON containing three rows:
//! [{"speaker_id":"...", "chapter":"...", "path":"/absolute/clip.wav"}, ...]
//! Rows 1 and 2 are the same reader, different chapters; row 3 a different reader.
use std::path::PathBuf;
use taurscribe_lib::{audio_decode, speaker_embedding::{self, SpeakerEmbeddingEngine, VaultSpeakerEmbedding}};

#[derive(serde::Deserialize)]
struct Fixture { speaker_id: String, chapter: String, path: PathBuf }

#[test]
#[ignore = "Requires installed neural speaker model and a local human-voice fixture manifest"]
fn human_voice_matches_across_words_and_rejects_a_different_reader() {
    let manifest = std::env::var("TAURSCRIBE_SPEAKER_FIXTURES").expect("set TAURSCRIBE_SPEAKER_FIXTURES");
    let fixtures: Vec<Fixture> = serde_json::from_slice(&std::fs::read(manifest).unwrap()).unwrap();
    assert_eq!(fixtures.len(), 3);
    assert_eq!(fixtures[0].speaker_id, fixtures[1].speaker_id);
    assert_ne!(fixtures[0].chapter, fixtures[1].chapter);
    assert_ne!(fixtures[0].speaker_id, fixtures[2].speaker_id);
    let mut engine = SpeakerEmbeddingEngine::new();
    engine.set_model_path(SpeakerEmbeddingEngine::default_model_path().expect("platform model directory"));
    assert!(engine.uses_neural_model(), "install the neural speaker model first");
    let mut embeddings = Vec::new();
    for fixture in &fixtures {
        let (pcm, rate) = audio_decode::decode_audio_mono_f32(&fixture.path).unwrap();
        assert!(pcm.len() as f32 / rate as f32 >= speaker_embedding::MIN_VOICEPRINT_SECONDS);
        let embedding = engine.compute_embedding(&pcm, rate).expect("real neural inference must succeed");
        assert!(embedding.len() >= 256, "must use full neural output, not acoustic fallback");
        assert!(embedding.iter().all(|x| x.is_finite()));
        assert!((speaker_embedding::l2_norm(&embedding) - 1.0).abs() < 0.001);
        let quiet: Vec<f32> = pcm.iter().map(|s| s * 0.25).collect();
        let attenuated = engine.compute_embedding(&quiet, rate).unwrap();
        let stability = speaker_embedding::cosine_similarity(&embedding, &attenuated);
        assert!(stability >= 0.90, "reader {}: volume stability {stability}", fixture.speaker_id);
        eprintln!("reader={} chapter={} dimensions={} volume-stability={stability:.3}", fixture.speaker_id, fixture.chapter, embedding.len());
        embeddings.push(embedding);
    }
    let threshold = speaker_embedding::DEFAULT_MATCH_THRESHOLD;
    let same = speaker_embedding::cosine_similarity(&embeddings[0], &embeddings[1]);
    let different = speaker_embedding::cosine_similarity(&embeddings[0], &embeddings[2]);
    eprintln!("same reader across chapters={same:.3}; different reader={different:.3}; threshold={threshold:.3}");
    let vault = [VaultSpeakerEmbedding { speaker_id: fixtures[0].speaker_id.clone(), display_name: "Enrolled fixture reader".into(), embedding: embeddings[0].clone(), snippet_path: None }];
    assert_eq!(speaker_embedding::match_speaker_against_vault(&embeddings[1], &vault, threshold).expect("same reader must be recognized").0, fixtures[0].speaker_id);
    assert!(speaker_embedding::match_speaker_against_vault(&embeddings[2], &vault, threshold).is_none(), "different reader must not be merged");
}
