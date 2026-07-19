//! Opt-in FluidAudio model integration test.
//!
//! Run with an installed FluidAudio cache and a 16 kHz mono PCM fixture:
//! `MURMUR_COREML_TEST_WAV=/path/to/prompt.wav cargo test --test coreml_transcription_integration -- --ignored`

#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

use ui_lib::transcriber::{
    coreml, parse_wav_to_samples, CoreMlBackend, TranscriptionBackend, COREML_MODEL_NAME,
};

#[test]
#[ignore = "requires the optional FluidAudio model cache and MURMUR_COREML_TEST_WAV"]
fn transcribes_with_installed_coreml_model() {
    let wav_path = std::env::var("MURMUR_COREML_TEST_WAV")
        .expect("set MURMUR_COREML_TEST_WAV to a 16 kHz mono 16-bit PCM WAV");
    assert!(
        coreml::specific_model_exists(COREML_MODEL_NAME),
        "FluidAudio Core ML model cache is not installed or is incomplete"
    );

    let wav = std::fs::read(&wav_path).expect("failed to read MURMUR_COREML_TEST_WAV");
    let samples = parse_wav_to_samples(&wav).expect("fixture must be 16 kHz mono 16-bit PCM");
    let mut backend = CoreMlBackend::new();
    backend.load_model(COREML_MODEL_NAME).unwrap();
    let first = backend
        .transcribe(&samples, "auto", None, true)
        .expect("Core ML transcription failed");
    let second = backend
        .transcribe(&samples, "auto", None, true)
        .expect("second Core ML transcription failed");

    assert!(
        !first.trim().is_empty(),
        "first Core ML transcription returned empty"
    );
    assert_eq!(
        first, second,
        "consecutive one-shot Core ML calls must not leak decoder state"
    );
}
