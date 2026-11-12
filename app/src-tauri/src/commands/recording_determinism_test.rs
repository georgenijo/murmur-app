//! Opt-in, out-of-app evidence. Inputs are generated here, never user recordings.
use super::decode_final_transcript;
use crate::dictation_context::TranscriptionSettings;
use crate::transcriber::{self, CoreMlBackend, TranscriptionBackend};
use crate::vad;
use sha2::{Digest, Sha256};
use std::process::{Command, Stdio};

struct Clip {
    original: Vec<f32>,
    filtered: Vec<f32>,
}

fn decode_hash(
    backend: &mut CoreMlBackend,
    settings: &TranscriptionSettings,
    clip: &Clip,
) -> [u8; 32] {
    let attempt = decode_final_transcript(
        backend,
        settings,
        &clip.filtered,
        &clip.original,
        clip.filtered.len() != clip.original.len(),
        #[cfg(feature = "preview-decode-reuse")]
        None,
    )
    .unwrap_or_else(|_| panic!("final_decode_succeeded=false"));
    assert!(attempt.empty_error.is_none(), "final_decode_nonempty=false");
    assert!(
        !attempt.text.trim().is_empty(),
        "final_decode_nonempty=false"
    );
    Sha256::digest(attempt.text.as_bytes()).into()
}

#[test]
#[ignore = "requires macOS say and installed Parakeet Core ML/Silero models; no app runtime"]
fn synthetic_final_decode_is_deterministic() {
    // Eight distinct short utterances and eight longer passages, each at two
    // speaking rates. The latter remain >15s even after the app's VAD recipe.
    const SHORT: [&str; 8] = [
        "Hello world. This is a simple dictation test.",
        "Please move the blue folder beside the green notebook.",
        "Our next appointment is on Tuesday at eleven in the morning.",
        "The small wooden bridge crosses a quiet river.",
        "Add apples, oranges, bread, and milk to the shopping list.",
        "Can you hear the difference between these two sentences?",
        "Set a timer for twenty minutes, then check the oven.",
        "First save the document. Then close the window.",
    ];
    const LONG: [&str; 8] = [
        "On a clear morning the walking group followed the river through the valley. They stopped near a wooden bridge to study a map and decide which trail to take next. The longer route passed through an orchard before reaching the village. Everyone agreed that there was enough time to visit the market, buy some bread, and return along the shaded path before the afternoon rain arrived.",
        "The library opened its new reading room on Saturday morning. Volunteers arranged the chairs near the windows and placed a small table beside the entrance. Visitors could borrow books, listen to a short talk, or ask questions about the collection. A local teacher explained how the old maps had been restored. At the end of the day the staff carefully packed the display and prepared the room for another busy week.",
        "Before preparing dinner, the cook checked the ingredients and read the recipe twice. The vegetables needed to be washed and cut into small pieces. While the soup was warming on the stove, a loaf of bread rested on the counter. The guests would arrive after sunset, so there was still time to set the table, fill the water glasses, and choose a quiet piece of music for the evening.",
        "The engineering team met to discuss a small improvement to the workshop. They wanted a clearer place to store tools and a safer path between the benches. Each person described a problem they had noticed during the previous week. After considering several ideas, the group chose a simple plan that could be completed in one afternoon. They measured the shelves, wrote down the dimensions, and ordered the materials for delivery the following Monday.",
        "During the train journey the students watched the landscape change from crowded streets to open fields. Their guide explained the history of several towns along the route. At the next station they would leave the train and walk to a museum near the central square. The schedule allowed enough time for lunch before the guided tour began. Everyone checked their bags and made sure they had the tickets needed for the return trip.",
        "A new gardener can begin with a few herbs in pots near a sunny window. The soil should drain well, and the plants need regular attention without receiving too much water. It helps to check the leaves each morning and turn the pots occasionally so that the stems grow evenly. After a few weeks the first small harvest can be used in a meal, making the daily work feel both practical and rewarding.",
        "The weather report predicted a calm start to the weekend followed by stronger winds on Sunday. The sailing club decided to hold its practice session early on Saturday. Members checked the equipment, reviewed the safety instructions, and divided into small groups. A coach stayed near the harbor to answer questions. When the session ended, everyone helped clean the boats and store the ropes before gathering for a short discussion about what they had learned.",
        "To organize the neighborhood exhibition, the committee asked residents to contribute photographs of familiar streets and buildings. Each picture received a short description explaining when and where it had been taken. The display showed how the area had changed over several generations while keeping many of its favorite places. Visitors spent time comparing the older images with recent views, sharing memories, and suggesting more stories that could be included in a future exhibition.",
    ];
    let temp = tempfile::tempdir().expect("synthetic_temp_created=false");
    let path = vad::vad_model_path().expect("vad_model_available=false");
    assert!(path.is_file(), "vad_model_available=false");
    let settings = TranscriptionSettings {
        model_name: transcriber::COREML_MODEL_NAME.to_string(),
        language: "en".to_string(),
        vad_sensitivity: 50,
        prompt: None,
        smart_punctuation: true,
        streaming_pipeline_enabled: false,
    };
    let mut clips = Vec::new();
    let mut short_clips = 0;
    let mut long_clips = 0;
    for rate in [150, 180] {
        for (text, long) in SHORT
            .iter()
            .map(|s| (*s, false))
            .chain(LONG.iter().map(|s| (*s, true)))
        {
            let wav = temp.path().join(format!("clip-{}.wav", clips.len()));
            let generated = Command::new("/usr/bin/say")
                .args([
                    "-v",
                    "Samantha",
                    "-r",
                    &rate.to_string(),
                    "--file-format=WAVE",
                    "--data-format=LEI16@16000",
                    "-o",
                ])
                .arg(&wav)
                .arg(text)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("synthetic_generation_started=false");
            assert!(generated.success(), "synthetic_generation_succeeded=false");
            let bytes = std::fs::read(&wav).expect("synthetic_wav_read=false");
            let original = transcriber::parse_wav_to_samples(&bytes)
                .unwrap_or_else(|_| panic!("synthetic_wav_valid=false"));
            let filtered = match vad::filter_speech(
                &path.to_string_lossy(),
                &original,
                vad::threshold_for_sensitivity(settings.vad_sensitivity),
            ) {
                Ok(vad::VadResult::Speech(samples)) => samples,
                _ => panic!("synthetic_speech_detected=false"),
            };
            let (with_bounds, _) = vad::filter_speech_with_segments(
                &path.to_string_lossy(),
                &original,
                vad::threshold_for_sensitivity(settings.vad_sensitivity),
            )
            .unwrap_or_else(|_| panic!("shadow_vad_succeeded=false"));
            assert!(
                matches!(with_bounds, vad::VadResult::Speech(ref samples)
                if crate::shadow_measurement::pcm_equal(samples, &filtered)),
                "final_vad_pcm_unchanged=false"
            );
            if long {
                assert!(filtered.len() > 15 * 16_000, "long_filtered_clip=false");
                long_clips += 1;
            } else {
                assert!(filtered.len() < 15 * 16_000, "short_filtered_clip=false");
                short_clips += 1;
            }
            clips.push(Clip { original, filtered });
        }
    }
    let mut backend = CoreMlBackend::new();
    let mut compared = 0;
    let mut mismatches = 0;
    let mut first_after_load_compared = 0;
    let mut after_reload_compared = 0;
    for generation in 0..2 {
        if generation > 0 {
            backend.reset();
            assert!(
                !backend.is_model_loaded(&settings.model_name),
                "model_unloaded=false"
            );
        }
        backend
            .load_model(&settings.model_name)
            .unwrap_or_else(|_| panic!("model_loaded=false"));
        // No warm-up decode: the first member of this pair is the very first
        // final decode after load (and, on generation 1, after unload/reload).
        for (index, clip) in clips.iter().enumerate() {
            let first = decode_hash(&mut backend, &settings, clip);
            let second = decode_hash(&mut backend, &settings, clip);
            mismatches += usize::from(first != second);
            compared += 1;
            first_after_load_compared += usize::from(index == 0);
            after_reload_compared += usize::from(generation > 0);
        }
    }
    backend.reset();
    println!("{{\"clips\":{},\"shortClips\":{short_clips},\"longClips\":{long_clips},\"compared\":{compared},\"mismatches\":{mismatches},\"firstAfterLoadCompared\":{first_after_load_compared},\"afterReloadCompared\":{after_reload_compared}}}", clips.len());
    assert_eq!(mismatches, 0, "deterministic=false");
}
