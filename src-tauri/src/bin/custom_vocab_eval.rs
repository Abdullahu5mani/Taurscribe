//! Benchmark evaluation binary for Custom Vocabulary & Context Jargon Injection.
//!
//! Evaluates Whisper ASR accuracy on LibriSpeech utterances containing challenging
//! proper nouns, names, and archaic/complex words, comparing:
//!   1. Baseline (unprompted) recognition
//!   2. Custom Vocabulary & Jargon injected recognition (dynamic prompt biasing + casing)
//!
//! Usage:
//!   cargo run --release --bin custom_vocab_eval
//!   cargo run --release --bin custom_vocab_eval -- --manifest <path> --audio-root <path>

use std::path::{Path, PathBuf};
use taurscribe_lib::audio_decode;
use taurscribe_lib::audio_preprocess;
use taurscribe_lib::context::{apply_custom_vocabulary_casing, build_dynamic_prompt};
use taurscribe_lib::librispeech_wer;
use taurscribe_lib::utils::clean_transcript;
use taurscribe_lib::whisper::WhisperManager;

struct TestCase {
    utt_id: &'static str,
    flac_rel: &'static str,
    ref_text: &'static str,
    target_terms: Vec<&'static str>,
}

fn pcm_for_eval(flac: &Path) -> Result<Vec<f32>, String> {
    let (pcm, sr) = audio_decode::decode_audio_mono_f32(flac)?;
    let mut pcm16 = audio_preprocess::resample_mono_to_16k(&pcm, sr)?;
    audio_preprocess::trim_file_buffer_edges_16k(&mut pcm16);
    audio_preprocess::preprocess_assembled_speech_16k(&mut pcm16);
    Ok(pcm16)
}

fn check_terms_present(text: &str, terms: &[&str]) -> (usize, usize) {
    let lower_text = text.to_lowercase();
    let mut hit = 0;
    for t in terms {
        if lower_text.contains(&t.to_lowercase()) {
            hit += 1;
        }
    }
    (hit, terms.len())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("===============================================================================");
    println!("  TAURSCRIBE — CUSTOM VOCABULARY & CONTEXT JARGON EVALUATION SUITE");
    println!("===============================================================================\n");

    let test_cases = vec![
        TestCase {
            utt_id: "7127-75947-0028",
            flac_rel: "7127/75947/7127-75947-0028.flac",
            ref_text: "QUICK QUICK THEN AMONG THE HIGH REED GRASS SAID MONTALAIS STOOP ATHENAIS YOU ARE SO TALL",
            target_terms: vec!["Montalais", "Athenaïs"],
        },
        TestCase {
            utt_id: "61-70970-0019",
            flac_rel: "61/70970/61-70970-0019.flac",
            ref_text: "AT LAST ALL WAS QUIET AND BLACK IN THE COURTYARD OF GAMEWELL",
            target_terms: vec!["Gamewell"],
        },
        TestCase {
            utt_id: "672-122797-0058",
            flac_rel: "672/122797/672-122797-0058.flac",
            ref_text: "WHO IS HUMPY DUMPY ASKED THE MICE",
            target_terms: vec!["Humpy Dumpy"],
        },
        TestCase {
            utt_id: "5142-36377-0003",
            flac_rel: "5142/36377/5142-36377-0003.flac",
            ref_text: "AMBROSE MET ME AT THE BOTTOM OF THE STAIRS AND SHOWED ME THE WAY TO THE SUPPER ROOM",
            target_terms: vec!["Ambrose"],
        },
        TestCase {
            utt_id: "61-70970-0004",
            flac_rel: "61/70970/61-70970-0004.flac",
            ref_text: "BUT TAKE IT WHILST I LIVE AND WEAR MONTFICHET'S SHIELD IN THE DAYS WHEN MY EYES CAN BE REJOICED BY SO BRAVE A SIGHT FOR YOU WILL NE'ER DISGRACE OUR SCUTCHEON I WARRANT ME",
            target_terms: vec!["Montfichet's", "scutcheon"],
        },
        TestCase {
            utt_id: "4507-16021-0017",
            flac_rel: "4507/16021/4507-16021-0017.flac",
            ref_text: "HE WOULD BE LIKE A PHILOLOGIST REFUSING TO EXAMINE A FACT IN LANGUAGE A PHILOSOPHER HESITATING TO SCRUTINIZE A FACT IN HUMANITY",
            target_terms: vec!["philologist", "philosopher", "scrutinize"],
        },
        TestCase {
            utt_id: "2300-131720-0010",
            flac_rel: "2300/131720/2300-131720-0010.flac",
            ref_text: "IT COULD NOT BE USED FOR ELECTROPLATING OR DEPOSITION NOR COULD IT CHARGE STORAGE BATTERIES ALL OF WHICH ARE EASILY WITHIN THE ABILITY OF THE DIRECT CURRENT",
            target_terms: vec!["electroplating", "deposition"],
        },
        TestCase {
            utt_id: "2300-131720-0003",
            flac_rel: "2300/131720/2300-131720-0003.flac",
            ref_text: "THE DYNAMO ELECTRIC MACHINE THOUGH SMALL WAS ROBUST FOR UNDER ALL THE VARYING SPEEDS OF WATER POWER AND THE VICISSITUDES OF THE PLANT TO WHICH IT BELONGED IT CONTINUED IN ACTIVE USE UNTIL EIGHTEEN NINETY NINE SEVENTEEN YEARS",
            target_terms: vec!["dynamo", "vicissitudes"],
        },
        TestCase {
            utt_id: "2300-131720-0029",
            flac_rel: "2300/131720/2300-131720-0029.flac",
            ref_text: "HENCE THE EDISON ELECTROLYTIC METER IS NO LONGER USED DESPITE ITS EXCELLENT QUALITIES",
            target_terms: vec!["Edison", "electrolytic"],
        },
        TestCase {
            utt_id: "260-123288-0002",
            flac_rel: "260/123288/260-123288-0002.flac",
            ref_text: "THE ATMOSPHERE IS CHARGED WITH VAPOURS PERVADED WITH THE ELECTRICITY GENERATED BY THE EVAPORATION OF SALINE WATERS",
            target_terms: vec!["pervaded", "saline"],
        },
        TestCase {
            utt_id: "1284-134647-0004",
            flac_rel: "1284/134647/1284-134647-0004.flac",
            ref_text: "SOME OF THE PENAL REGULATIONS WERE COPIED FROM THE EDICTS OF DIOCLETIAN AND THIS METHOD OF CONVERSION WAS APPLAUDED BY THE SAME BISHOPS WHO HAD FELT THE HAND OF OPPRESSION AND PLEADED FOR THE RIGHTS OF HUMANITY",
            target_terms: vec!["Diocletian", "edicts"],
        },
        TestCase {
            utt_id: "1284-134647-0007",
            flac_rel: "1284/134647/1284-134647-0007.flac",
            ref_text: "PROSCRIBED BY THE CIVIL AND ECCLESIASTICAL POWERS OF THE EMPIRE THE DONATISTS STILL MAINTAINED IN SOME PROVINCES PARTICULARLY IN NUMIDIA THEIR SUPERIOR NUMBERS AND FOUR HUNDRED BISHOPS ACKNOWLEDGED THE JURISDICTION OF THEIR PRIMATE",
            target_terms: vec!["proscribed", "ecclesiastical", "Donatists", "Numidia"],
        },
        TestCase {
            utt_id: "1188-133604-0040",
            flac_rel: "1188/133604/1188-133604-0040.flac",
            ref_text: "THE CRAMPNESS AND THE POVERTY ARE ALL INTENDED",
            target_terms: vec!["crampness"],
        },
    ];

    // Locate LibriSpeech corpus audio root
    let default_root = PathBuf::from("taurscribe-runtime/librispeech/LibriSpeech/test-clean");
    let alt_root = PathBuf::from("../taurscribe-runtime/librispeech/LibriSpeech/test-clean");
    let audio_root = if default_root.is_dir() {
        default_root
    } else if alt_root.is_dir() {
        alt_root
    } else {
        eprintln!("[ERROR] Could not find LibriSpeech test-clean directory at either:");
        eprintln!("  - {}", default_root.display());
        eprintln!("  - {}", alt_root.display());
        return Err("LibriSpeech test-clean root missing".into());
    };

    println!("[INFO] Initializing Whisper model...");
    let mut whisper = WhisperManager::new();
    whisper.initialize(None, false)?;

    let mut baseline_hits = 0;
    let mut injected_hits = 0;
    let mut total_target_terms = 0;
    let mut baseline_wers = Vec::new();
    let mut injected_wers = Vec::new();

    println!("\n-------------------------------------------------------------------------------");
    println!("RUNNING EVALUATION ON TEST UTTERANCES");
    println!("-------------------------------------------------------------------------------");

    for (idx, tc) in test_cases.iter().enumerate() {
        let flac_path = audio_root.join(tc.flac_rel);
        if !flac_path.is_file() {
            eprintln!("[WARN] Audio file not found: {}", flac_path.display());
            continue;
        }

        let pcm = match pcm_for_eval(&flac_path) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[ERROR] Failed to decode {}: {}", tc.utt_id, e);
                continue;
            }
        };

        // 1. Baseline: Whisper without custom vocabulary prompt
        let raw_baseline = whisper.transcribe_audio_data(&pcm, None)?;
        let hyp_baseline = clean_transcript(&raw_baseline);
        let norm_ref = librispeech_wer::normalize_for_wer(tc.ref_text);
        let norm_baseline = librispeech_wer::normalize_for_wer(&hyp_baseline);
        let wer_baseline = librispeech_wer::word_error_rate(&norm_ref, &norm_baseline);
        let (base_hit, term_count) = check_terms_present(&hyp_baseline, &tc.target_terms);

        // 2. Custom Vocabulary Injected: Build dynamic prompt & apply casing
        let vocab_strings: Vec<String> = tc.target_terms.iter().map(|s| s.to_string()).collect();
        let dynamic_prompt = build_dynamic_prompt(&vocab_strings, false);
        let raw_injected = whisper.transcribe_audio_data(&pcm, dynamic_prompt.as_deref())?;
        let cleaned_injected = clean_transcript(&raw_injected);
        let hyp_injected = apply_custom_vocabulary_casing(&cleaned_injected, &vocab_strings);
        let norm_injected = librispeech_wer::normalize_for_wer(&hyp_injected);
        let wer_injected = librispeech_wer::word_error_rate(&norm_ref, &norm_injected);
        let (inj_hit, _) = check_terms_present(&hyp_injected, &tc.target_terms);

        baseline_hits += base_hit;
        injected_hits += inj_hit;
        total_target_terms += term_count;
        baseline_wers.push(wer_baseline);
        injected_wers.push(wer_injected);

        println!("\nUtterance [{}/{}]: {}", idx + 1, test_cases.len(), tc.utt_id);
        println!("  Target Terms: {:?}", tc.target_terms);
        println!("  Reference:    {}", tc.ref_text);
        println!("  Prompt Sent:  {:?}", dynamic_prompt);
        println!("  Baseline:     \"{}\" (WER: {:.2}%, Terms Found: {}/{})", hyp_baseline, wer_baseline * 100.0, base_hit, term_count);
        println!("  Injected:     \"{}\" (WER: {:.2}%, Terms Found: {}/{})", hyp_injected, wer_injected * 100.0, inj_hit, term_count);
        
        if inj_hit > base_hit {
            println!("  Result:       >>> ACCURACY BOOST! (Term accuracy increased from {}/{} to {}/{})", base_hit, term_count, inj_hit, term_count);
        } else if wer_injected < wer_baseline {
            println!("  Result:       >>> WER IMPROVED! ({:.2}% -> {:.2}%)", wer_baseline * 100.0, wer_injected * 100.0);
        } else {
            println!("  Result:       >>> Maintained baseline parity");
        }
    }

    println!("\n===============================================================================");
    println!("  EVALUATION SUMMARY & EMPIRICAL BENCHMARK");
    println!("===============================================================================");

    let baseline_term_acc = (baseline_hits as f64 / total_target_terms as f64) * 100.0;
    let injected_term_acc = (injected_hits as f64 / total_target_terms as f64) * 100.0;
    let avg_wer_baseline = (baseline_wers.iter().sum::<f64>() / baseline_wers.len() as f64) * 100.0;
    let avg_wer_injected = (injected_wers.iter().sum::<f64>() / injected_wers.len() as f64) * 100.0;

    println!("Target Terms Evaluated:        {}", total_target_terms);
    println!("Baseline Term Accuracy:        {:.1}% ({}/{} detected)", baseline_term_acc, baseline_hits, total_target_terms);
    println!("Custom Vocab Term Accuracy:    {:.1}% ({}/{} detected)", injected_term_acc, injected_hits, total_target_terms);
    println!("Term Accuracy Delta:           +{:.1}%", injected_term_acc - baseline_term_acc);
    println!("-------------------------------------------------------------------------------");
    println!("Mean Baseline WER:             {:.2}%", avg_wer_baseline);
    println!("Mean Custom Vocab WER:         {:.2}%", avg_wer_injected);
    println!("WER Relative Reduction:        {:.1}%", (avg_wer_baseline - avg_wer_injected) / avg_wer_baseline * 100.0);
    println!("===============================================================================\n");

    whisper.unload();
    Ok(())
}
