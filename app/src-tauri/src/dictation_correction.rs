use serde::Deserialize;

const MAX_EDIT_BYTES: usize = 256;

#[derive(Clone, Default)]
pub(crate) enum ReviewPurpose {
    #[default]
    SelectedText,
    Correction {
        recording_id: u64,
        delivery: CorrectionDelivery,
        teaching_context: Option<crate::correct_and_teach::TeachingContext>,
    },
}

#[derive(Clone)]
pub(crate) enum CorrectionDelivery {
    Copy,
    Selection(crate::frontmost::DeliveryTargetSnapshot),
}

impl ReviewPurpose {
    pub(crate) fn is_correction(&self) -> bool {
        matches!(self, Self::Correction { .. })
    }

    pub(crate) fn target(&self) -> Option<crate::frontmost::DeliveryTargetSnapshot> {
        match self {
            Self::Correction {
                delivery: CorrectionDelivery::Selection(target),
                ..
            } => Some(target.clone()),
            _ => None,
        }
    }

    pub(crate) fn copy_only(&self) -> bool {
        matches!(
            self,
            Self::Correction {
                delivery: CorrectionDelivery::Copy,
                ..
            }
        )
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelEdit {
    heard: String,
    replacement: String,
}

/// Only an explicitly introduced trailing sequence of individual ASCII letters
/// is spelling. Ordinary prose such as "I am a developer" remains prose.
pub(crate) fn literal_spelling(instruction: &str) -> Option<String> {
    let words: Vec<_> = instruction.split_whitespace().collect();
    let start = words.iter().rposition(|word| {
        matches!(
            word.to_ascii_lowercase().as_str(),
            "spelled" | "spelt" | "spelling" | "with"
        )
    })? + 1;
    let letters: Vec<_> = words
        .get(start..)?
        .iter()
        .map(|word| {
            word.trim_matches(|c: char| matches!(c, '.' | ',' | ':' | ';' | '!' | '?' | '\"'))
        })
        .collect();
    if !(2..=64).contains(&letters.len())
        || letters
            .iter()
            .any(|letter| letter.len() != 1 || !letter.as_bytes()[0].is_ascii_alphabetic())
    {
        return None;
    }
    Some(letters.concat())
}

pub(crate) fn model_instruction(spoken: &str) -> String {
    format!("Use compact single-line JSON without line breaks. Correct one word or short phrase in the input according to the request below. Return ONLY a JSON object with keys \"heard\" (the exact original phrase to replace) and \"replacement\" (the corrected phrase). Do not return the full sentence. If unclear return {{}}. Request: {spoken}")
}

pub(crate) fn corrected_text(original: &str, spoken: &str, output: &str) -> Result<String, ()> {
    let edit: ModelEdit = serde_json::from_str(output).map_err(|_| ())?;
    let replacement = literal_spelling(spoken).unwrap_or(edit.replacement);
    if edit.heard.trim().is_empty()
        || replacement.trim().is_empty()
        || edit.heard.len() > MAX_EDIT_BYTES
        || replacement.len() > MAX_EDIT_BYTES
        || edit.heard.split_whitespace().count() > 8
        || replacement.split_whitespace().count() > 8
        || edit.heard.chars().any(char::is_control)
        || replacement.chars().any(char::is_control)
        || edit.heard == replacement
        || (literal_spelling(spoken).is_some() && edit.heard.split_whitespace().count() != 1)
    {
        return Err(());
    }
    let mut matches = original.match_indices(&edit.heard).filter(|(start, _)| {
        let end = start + edit.heard.len();
        let word = |c: char| c.is_alphanumeric() || c == '_';
        !original[..*start].chars().next_back().is_some_and(word)
            && !original[end..].chars().next().is_some_and(word)
    });
    let (start, _) = matches.next().ok_or(())?;
    if matches.next().is_some() {
        return Err(());
    }
    let mut corrected = original.to_string();
    corrected.replace_range(start..start + edit.heard.len(), &replacement);
    if corrected.len() > crate::selection::MAX_SELECTION_BYTES {
        return Err(());
    }
    crate::correct_and_teach::propose_rule(original, &corrected).map_err(|_| ())?;
    Ok(corrected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires the installed local model and built macOS sidecar"]
    async fn installed_model_proposes_real_corrections() {
        use crate::llm_sidecar::*;
        use std::time::Duration;
        let model = installed_model_path().expect("installed model path");
        assert!(model.is_file(), "install the transform model first");
        let helper = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("binaries/murmur-llm-sidecar-aarch64-apple-darwin");
        let audio_case = if let Some(directory) = std::env::var_os("MURMUR_CORRECTION_WAV_DIR") {
            Some(
                tokio::task::spawn_blocking(move || {
                    use crate::transcriber::{
                        parse_wav_to_samples, CoreMlBackend, TranscriptionBackend,
                        COREML_MODEL_NAME,
                    };
                    let directory = std::path::PathBuf::from(directory);
                    let mut backend = CoreMlBackend::new();
                    backend
                        .load_model(COREML_MODEL_NAME)
                        .expect("installed ASR model");
                    let mut recognize = |name| {
                        let wav = std::fs::read(directory.join(name)).expect("synthetic WAV");
                        backend
                            .transcribe(&parse_wav_to_samples(&wav).unwrap(), "auto", None, true)
                            .expect("real ASR")
                    };
                    let original = recognize("dictation.wav");
                    let spoken = recognize("instruction.wav");
                    assert!(
                        original.contains("Friday"),
                        "synthetic original: {original}"
                    );
                    let expected = original.replace("Friday", "Monday");
                    (original, spoken, expected)
                })
                .await
                .unwrap(),
            )
        } else {
            None
        };
        let sidecar = std::sync::Arc::new(LlmSidecar::for_test(TestSpawnConfig {
            helper_path: helper,
            model_path: model,
            model_size: TRANSFORM_MODEL_SIZE_BYTES,
            model_sha256: TRANSFORM_MODEL_SHA256.into(),
            scenario_env: vec![],
            request_slack: Duration::from_secs(2),
            cancel_grace: Duration::from_millis(250),
            handshake_timeout: Duration::from_secs(45),
            idle_after: Duration::from_secs(60),
        }));
        let mut cases: Vec<(String, String, String)> = [
            (
                "We should use Tori for the desktop app.",
                "The framework is spelled T A U R I.",
                "We should use TAURI for the desktop app.",
            ),
            (
                "Ship the release on Friday.",
                "Change Friday to Monday.",
                "Ship the release on Monday.",
            ),
            (
                "Send the report to John.",
                "The last word should be Jane.",
                "Send the report to Jane.",
            ),
        ]
        .into_iter()
        .map(|(original, spoken, expected)| (original.into(), spoken.into(), expected.into()))
        .collect();
        if let Some(case) = audio_case {
            cases.push(case);
        }
        for (original, spoken, expected) in cases {
            let output = sidecar
                .transform(
                    &model_instruction(&spoken),
                    &original,
                    Duration::from_secs(20),
                    CancelToken::new(),
                )
                .await
                .expect("local inference");
            assert_eq!(
                corrected_text(&original, &spoken, &output.output),
                Ok(expected),
                "synthetic correction output: {}",
                output.output
            );
        }
        sidecar.shutdown();
    }

    #[test]
    fn spelled_letters_override_the_real_models_incorrect_replacement() {
        assert_eq!(
            corrected_text(
                "Use Tori for the app.",
                "The framework is spelled T A U R I.",
                r#"{"heard":"Tori","replacement":"Tor"}"#
            ),
            Ok("Use TAURI for the app.".into())
        );
    }

    #[test]
    fn only_explicit_separated_letters_are_spelling() {
        assert_eq!(
            literal_spelling("replace the last word with T A U R I"),
            Some("TAURI".into())
        );
        for input in [
            "I am a developer",
            "replace Friday with Monday",
            "spelled T A U R I and remember it",
            "spelled B 8",
        ] {
            assert_eq!(literal_spelling(input), None);
        }
    }

    #[test]
    fn reconstructs_one_exact_edit_and_preserves_surrounding_text() {
        assert_eq!(
            corrected_text(
                "Ship Friday. Keep the version 2.0!",
                "Change Friday to Monday.",
                r#"{"heard":"Friday","replacement":"Monday"}"#
            ),
            Ok("Ship Monday. Keep the version 2.0!".into())
        );
    }

    #[test]
    fn rejects_ambiguous_absent_partial_or_malformed_edits() {
        for (original, output) in [
            ("Tori and Tori", r#"{"heard":"Tori","replacement":"Tauri"}"#),
            ("Tori", r#"{"heard":"Tor","replacement":"Tauri"}"#),
            ("Tori", r#"{"heard":"Jane","replacement":"Tauri"}"#),
            (
                "Tori",
                r#"{"heard":"Tori","replacement":"Tauri","action":"paste"}"#,
            ),
            ("Tori", "Here is your correction: Tauri"),
            ("Tori", "{}"),
        ] {
            assert!(corrected_text(original, "fix it", output).is_err());
        }
        assert!(corrected_text(
            "Use Tori today",
            "spelled T A U R I",
            r#"{"heard":"Use Tori today","replacement":"Tauri"}"#
        )
        .is_err());
    }
}
