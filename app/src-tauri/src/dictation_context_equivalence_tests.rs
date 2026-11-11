//! Resolver equivalence across the Modes retirement.
//!
//! `legacy-matrix.json` holds persisted settings that exercise every way a Mode
//! changed a recording. `before-resolver.json` is what the pre-removal resolver
//! produced for each probe. The frontend migration (`legacyModes.ts`) rewrites
//! the same settings into `migrated-matrix.json`; the resolver over those
//! migrated settings must reproduce every field that still has a home.
//!
//! Set `MURMUR_UPDATE_MODES_FIXTURES=1` on the pre-removal revision to
//! regenerate `before-resolver.json`.

use crate::commands::recording::{sanitize_app_profiles, sanitize_modes};
use crate::dictation_context::{resolve, DictationContextSnapshot, ResolverInputs};
use crate::state::DictationState;
use serde_json::{json, Value};
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../src/lib/fixtures/modes-retirement")
        .join(name)
}

fn read_fixture(name: &str) -> Value {
    let path = fixture_path(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse {name}: {error}"))
}

fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing string `{key}`"))
}

fn global_state(settings: &Value) -> DictationState {
    DictationState {
        model_name: string(settings, "model").to_string(),
        language: string(settings, "language").to_string(),
        auto_paste: settings["autoPaste"].as_bool().expect("autoPaste"),
        cleanup_enabled: settings["cleanupEnabled"]
            .as_bool()
            .expect("cleanupEnabled"),
        smart_formatting_enabled: settings["smartFormattingEnabled"]
            .as_bool()
            .expect("smartFormattingEnabled"),
        voice_commands_enabled: settings["voiceCommandsEnabled"]
            .as_bool()
            .expect("voiceCommandsEnabled"),
        ..DictationState::default()
    }
}

/// Mirrors `configure_dictation` for the pre-removal settings document.
fn legacy_state(settings: &Value) -> DictationState {
    let mut state = global_state(settings);
    if let Some(modes) = settings.get("modes").and_then(Value::as_array) {
        state.modes = sanitize_modes(modes);
    }
    if let Some(id) = settings.get("activeModeId").and_then(Value::as_str) {
        let id = id.trim();
        let usable = crate::dictation_context::builtin_mode(id).is_some()
            || state.modes.iter().any(|mode| mode.id == id && mode.enabled);
        state.manual_mode_id = if usable { id } else { "builtin.everyday" }.to_string();
    }
    if let Some(profiles) = settings.get("appProfiles").and_then(Value::as_array) {
        state.app_profiles = sanitize_app_profiles(profiles);
    }
    state
}

fn observe(snapshot: &DictationContextSnapshot) -> Value {
    let transformations = &snapshot.transformations;
    json!({
        "writingStyle": snapshot.writing_style.as_str(),
        "autoPaste": snapshot.delivery.auto_paste,
        "cleanupEnabled": transformations.cleanup_enabled,
        "cleanupRemoveFiller": transformations.cleanup_remove_filler,
        "cleanupCapitalize": transformations.cleanup_capitalize,
        "correctionEnabled": transformations.correction_enabled,
        "smartFormattingEnabled": transformations.smart_formatting_enabled,
        "cliFormattingMode": format!("{:?}", transformations.cli_formatting_mode),
        "cliFormattingEnabled": transformations.cli_formatting_enabled,
        "ideContextEnabled": transformations.ide_context_enabled,
        "textRegister": serde_json::to_value(transformations.text_register).unwrap(),
        "spokenNumbersEnabled": transformations.spoken_numbers_enabled,
        "builtInVoiceCommands": snapshot.enabled_command_groups.built_in_voice_commands,
        "spokenStructurePolicy": format!("{:?}", transformations.spoken_structure_policy),
        "modelName": snapshot.transcription.model_name,
        "language": snapshot.transcription.language,
    })
}

fn resolve_probe(state: &DictationState, bundle_id: &str) -> Value {
    observe(&resolve(ResolverInputs {
        bundle_id: Some(bundle_id),
        site_mode_id: None,
        global: state,
        prompt: None,
        correction_matcher: None,
        ide_context_index: None,
        vocabulary_version: 1,
        voice_commands: None,
        session_overrides: Default::default(),
    }))
}

fn resolve_all(matrix: &Value, state_of: impl Fn(&Value) -> DictationState) -> Value {
    let cases = matrix["cases"].as_array().expect("cases");
    let outputs = cases
        .iter()
        .map(|case| {
            let state = state_of(case);
            let probes = case["probes"]
                .as_array()
                .expect("probes")
                .iter()
                .map(|probe| {
                    json!({
                        "id": probe["id"],
                        "output": resolve_probe(&state, string(probe, "bundleId")),
                    })
                })
                .collect::<Vec<_>>();
            json!({ "id": case["id"], "probes": probes })
        })
        .collect::<Vec<_>>();
    json!({ "version": 1, "cases": outputs })
}

fn assert_partial(actual: &Value, expected: &Value, label: &str) {
    for (key, want) in expected.as_object().expect("expectation object") {
        assert_eq!(&actual[key], want, "{label}: {key}");
    }
}

fn probe_output<'a>(before: &'a Value, case_id: &str, probe_id: &str) -> &'a Value {
    before["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["id"] == case_id)
        .and_then(|case| {
            case["probes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|probe| probe["id"] == probe_id)
        })
        .map(|probe| &probe["output"])
        .unwrap_or_else(|| panic!("no before output for {case_id}/{probe_id}"))
}

#[test]
fn legacy_resolver_matches_committed_before_outputs() {
    let matrix = read_fixture("legacy-matrix.json");
    let actual = resolve_all(&matrix, |case| legacy_state(&case["settings"]));

    if std::env::var_os("MURMUR_UPDATE_MODES_FIXTURES").is_some() {
        let mut text = serde_json::to_string_pretty(&actual).unwrap();
        text.push('\n');
        std::fs::write(fixture_path("before-resolver.json"), text).unwrap();
        return;
    }

    assert_eq!(
        actual,
        read_fixture("before-resolver.json"),
        "pre-removal resolver output drifted from the committed fixture"
    );

    for case in matrix["cases"].as_array().unwrap() {
        for probe in case["probes"].as_array().unwrap() {
            let label = format!("{}/{}", case["id"], probe["id"]);
            let output = probe_output(&actual, case["id"].as_str().unwrap(), string(probe, "id"));
            if let Some(expected) = probe.get("beforeExpected") {
                assert_partial(output, expected, &format!("{label} before"));
            }
            if let Some(expected) = probe.get("expectIdeContext") {
                assert_eq!(
                    &output["ideContextEnabled"], expected,
                    "{label} ide context"
                );
            }
        }
    }
}
