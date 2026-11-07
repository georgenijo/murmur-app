//! Synthetic Phase 2–3 downgrade contract. Both hosts use these exact loaders;
//! production TCC/Developer ID acceptance remains a separate signed rehearsal.
use crate::{
    completion::{lifecycle::Operation, save_source, Command, CompletionService, HistoryMutation},
    state,
};
use serde_json::{json, Value};

fn fixture() -> Value {
    serde_json::from_str(include_str!("gpui-downgrade-fixtures.json")).unwrap()
}
fn assert_future_keys(original: &Value, written: &Value) {
    match original {
        Value::Object(object) => {
            for (key, value) in object {
                if key.starts_with("future") {
                    assert_eq!(written.get(key), Some(value), "{key}");
                } else if let Some(written) = written.get(key) {
                    assert_future_keys(value, written);
                }
            }
        }
        Value::Array(array) => {
            for (i, value) in array.iter().enumerate() {
                assert_future_keys(value, &written[i]);
            }
        }
        _ => (),
    }
}

#[test]
fn same_version_unknown_keys_survive_normalization_and_settings_edits() {
    let fixture = fixture();
    for (name, key) in [
        ("settings", "dictation-settings"),
        ("appearance", "murmur-appearance"),
        ("library", "murmur-theme-library"),
    ] {
        let normalized = state::normalize(key, fixture[name].clone()).unwrap();
        assert_future_keys(&fixture[name], &normalized);
        assert_eq!(
            state::normalize(key, normalized.clone()).unwrap(),
            normalized
        );
        if name == "settings" {
            let edited = state::settings_edit(&normalized, "autoPaste", json!(true)).unwrap();
            assert_future_keys(&fixture[name], &edited);
            assert_eq!(edited["settingsVersion"], 7);
            assert_eq!(edited["autoPaste"], true);
        }
    }
}

#[test]
fn gpui_written_files_reopen_through_the_shared_tauri_loaders() {
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    {
        let gpui = CompletionService::start(root.into(), false).unwrap();
        gpui.lifecycle(Operation::Status).unwrap();
        for (key, name) in [
            ("dictation-settings", "settings"),
            ("murmur-theme-library", "library"),
        ] {
            let value = state::normalize(key, fixture[name].clone()).unwrap();
            save_source(root, key, &value.to_string()).unwrap();
        }
        gpui.lifecycle(Operation::Save {
            key: "murmur-appearance".into(),
            value: fixture["appearance"].clone(),
        })
        .unwrap();
        gpui.execute(Command::Complete {
            source: "synthetic-session:1".into(),
            entry: Some(fixture["history"].clone()),
            event: None,
        })
        .unwrap();
        futures_executor::block_on(gpui.seal_and_flush()).unwrap();
    }
    // Model same-schema additive metadata from a future writer. The next GPUI
    // mutation must retain it, without changing the envelope/schema versions.
    for file in ["history.json", "ui-state.json"] {
        let path = root.join(file);
        let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["futureEnvelope"] = json!({"opaque": [42, null]});
        std::fs::write(path, value.to_string()).unwrap();
    }
    {
        let gpui = CompletionService::start(root.into(), false).unwrap();
        let snapshot = gpui.execute(Command::Snapshot).unwrap();
        let pinned = gpui
            .execute(Command::History(HistoryMutation::Pin {
                target: snapshot.history[0].clone(),
            }))
            .unwrap();
        gpui.execute(Command::History(HistoryMutation::Edit {
            target: pinned.history[0].clone(),
            text: "Synthetic edited transcript.".into(),
        }))
        .unwrap();
        let current =
            gpui.lifecycle(Operation::Status).unwrap().values["murmur-appearance"].clone();
        let mut edited = current.clone();
        edited["mode"] = json!("dark");
        edited["revision"] = json!(5);
        gpui.compare_source("murmur-appearance".into(), current, edited)
            .unwrap();
        futures_executor::block_on(gpui.seal_and_flush()).unwrap();
    }
    // This is the same loading code used by Tauri's completion snapshot and
    // lifecycle commands, after all GPUI handles/locks have been dropped.
    let tauri = CompletionService::start(root.into(), false).unwrap();
    let snapshot = tauri.execute(Command::Snapshot).unwrap();
    assert_eq!(snapshot.history.len(), 1);
    assert_future_keys(&fixture["history"], &snapshot.history[0]);
    assert_eq!(snapshot.history[0]["pinned"], true);
    assert_eq!(snapshot.history[0]["text"], "Synthetic edited transcript.");
    let loaded = tauri.lifecycle(Operation::Status).unwrap();
    for (name, key) in [
        ("settings", "dictation-settings"),
        ("library", "murmur-theme-library"),
        ("appearance", "murmur-appearance"),
    ] {
        assert_future_keys(&fixture[name], &loaded.values[key]);
    }
    assert_eq!(loaded.values["murmur-appearance"]["mode"], "dark");
    for file in ["history.json", "ui-state.json"] {
        let value: Value =
            serde_json::from_slice(&std::fs::read(root.join(file)).unwrap()).unwrap();
        assert_eq!(value["schema"], 1);
        assert_eq!(value["futureEnvelope"], json!({"opaque": [42, null]}));
    }
}

#[test]
fn unsupported_schema_versions_fail_without_replacing_original_files() {
    let fixture = fixture();
    for (key, name, file, field, version) in [
        (
            "dictation-settings",
            "settings",
            "settings.json",
            "settingsVersion",
            8,
        ),
        (
            "murmur-theme-library",
            "library",
            "theme-library.json",
            "version",
            2,
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut future = fixture[name].clone();
        future[field] = json!(version);
        let bytes = future.to_string();
        let path = dir.path().join(file);
        std::fs::write(&path, &bytes).unwrap();
        assert!(save_source(dir.path(), key, &fixture[name].to_string()).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), bytes);
    }
    let mut future = fixture["appearance"].clone();
    future["version"] = json!(2);
    assert!(state::normalize("murmur-appearance", future).is_err());
    // Known invalid fields cannot sneak back in through opaque-field retention.
    let invalid = json!({"version":1,"theme":{"version":1,"contrast":"bad","accent":"bad"},"selection":{"light":"!","dark":"!"},"cache":{"obsolete":true},"future":true});
    let normalized = state::normalize("murmur-appearance", invalid).unwrap();
    assert!(normalized.get("cache").is_none());
    assert!(normalized.get("selection").is_none());
    assert!(normalized["theme"].get("accent").is_none());
    assert!(normalized["theme"].get("contrast").is_none());
    assert_eq!(normalized["future"], true);
}
