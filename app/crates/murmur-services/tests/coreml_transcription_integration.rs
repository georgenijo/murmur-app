//! Opt-in FluidAudio model integration test.
//!
//! Run with an installed FluidAudio cache and a 16 kHz mono PCM fixture:
//! `MURMUR_COREML_TEST_WAV=/path/to/prompt.wav cargo test --test coreml_transcription_integration -- --ignored`

#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

use murmur_services::transcriber::{
    coreml, parse_wav_to_samples, CoreMlBackend, TranscriptionBackend, COREML_MODEL_NAME,
};

#[test]
#[ignore = "requires installed shared FluidAudio cache; read-only loading, no capture or download"]
fn installed_coreml_cache_loads_in_a_preview_without_changing_model_files() {
    use sha2::{Digest, Sha256};
    use std::{collections::BTreeMap, io::Read, path::Path};
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    murmur_services::native_services::initialize_identity(
        "com.localdictation.onboarding28-coreml-review-test",
        &home,
    )
    .unwrap();
    let mut backend = CoreMlBackend::new();
    let model = backend.models_dir().unwrap().join("parakeet-tdt-0.6b-v3");
    fn artifacts(root: &Path) -> BTreeMap<std::path::PathBuf, [u8; 32]> {
        fn visit(root: &Path, path: &Path, result: &mut BTreeMap<std::path::PathBuf, [u8; 32]>) {
            for entry in std::fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(root, &path, result);
                } else {
                    let mut file = std::fs::File::open(&path).unwrap();
                    let mut hash = Sha256::new();
                    let mut bytes = [0u8; 65536];
                    loop {
                        let count = file.read(&mut bytes).unwrap();
                        if count == 0 {
                            break;
                        }
                        hash.update(&bytes[..count]);
                    }
                    result.insert(
                        path.strip_prefix(root).unwrap().to_owned(),
                        hash.finalize().into(),
                    );
                }
            }
        }
        let mut result = BTreeMap::new();
        visit(root, root, &mut result);
        result
    }
    assert!(coreml::specific_model_exists(COREML_MODEL_NAME));
    let before = artifacts(&model);
    backend.load_model(COREML_MODEL_NAME).unwrap();
    assert!(backend.is_model_loaded(COREML_MODEL_NAME));
    assert_eq!(
        artifacts(&model),
        before,
        "runtime load must not change the shared model cache"
    );
}

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
