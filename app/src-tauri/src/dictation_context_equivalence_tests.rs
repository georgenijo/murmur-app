//! Resolver equivalence across the Modes retirement.
//!
//! `legacy-matrix.json` holds persisted settings that exercise every way a Mode
//! changed a recording. `before-resolver.json` is what the pre-removal resolver
//! produced for each probe; it was captured on the last revision that still had
//! Modes and is frozen evidence now. The frontend migration (`legacyModes.ts`)
//! rewrites the same settings into `migrated-matrix.json`; the resolver over
//! those migrated settings must reproduce every field that still has a home.

use crate::commands::recording::sanitize_app_profiles;
use crate::dictation_context::{resolve, DictationContextSnapshot, ResolverInputs};
use crate::state::DictationState;
use serde_json::{json, Map, Value};
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
        global: state,
        prompt: None,
        correction_matcher: None,
        ide_context_index: None,
        vocabulary_version: 1,
        voice_commands: None,
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

/// The resolver over migrated settings (no Modes) against the committed
/// pre-removal outputs, restricted to the fields each probe says have a home.
#[test]
fn migrated_settings_reproduce_before_outputs_for_fields_with_a_home() {
    let matrix = read_fixture("legacy-matrix.json");
    let before = read_fixture("before-resolver.json");
    let migrated = read_fixture("migrated-matrix.json");
    let after = resolve_all(&matrix, |case| {
        let id = case["id"].as_str().unwrap();
        let settings = migrated["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["id"] == id)
            .unwrap_or_else(|| panic!("no migrated settings for {id}"));
        let settings = &settings["settings"];
        let mut state = global_state(settings);
        state.app_profiles =
            sanitize_app_profiles(settings["appProfiles"].as_array().expect("appProfiles"));
        state
    });

    let mut compared = 0;
    for case in matrix["cases"].as_array().unwrap() {
        let case_id = case["id"].as_str().unwrap();
        for probe in case["probes"].as_array().unwrap() {
            let probe_id = string(probe, "id");
            let label = format!("{case_id}/{probe_id}");
            let want = probe_output(&before, case_id, probe_id);
            let got = probe_output(&after, case_id, probe_id);
            let fields = probe["compare"].as_array().expect("compare");
            let mut want_fields = Map::new();
            let mut got_fields = Map::new();
            for field in fields {
                let field = field.as_str().unwrap();
                want_fields.insert(field.to_string(), want[field].clone());
                got_fields.insert(field.to_string(), got[field].clone());
                compared += 1;
            }
            assert_eq!(got_fields, want_fields, "{label}: migrated != before");
            if let Some(expected) = probe.get("afterExpected") {
                assert_partial(got, expected, &format!("{label} after"));
            }
            if let Some(expected) = probe.get("expectIdeContext") {
                assert_eq!(&got["ideContextEnabled"], expected, "{label} ide context");
            }
        }
    }
    assert!(compared > 300, "matrix compared only {compared} fields");
}
