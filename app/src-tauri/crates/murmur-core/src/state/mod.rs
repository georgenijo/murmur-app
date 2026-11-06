//! Host-neutral source-state schemas. WebKit caches are never authoritative.
use crate::completion::{Error, Result};
use serde_json::{json, Value};
mod appearance;
mod settings_nested;

pub fn settings_defaults() -> Value {
    serde_json::from_str(include_str!("settings-defaults.json")).expect("checked-in schema")
}
fn allowed(value: &Value, choices: &[&str]) -> bool {
    value.as_str().is_some_and(|s| choices.contains(&s))
}
fn bounded(v: &Value, min: f64, max: f64, default: f64) -> Value {
    json!(v.as_f64().unwrap_or(default).clamp(min, max).trunc() as i64)
}
fn unique_strings(v: &Value, cap: usize) -> Value {
    let mut out = vec![];
    for s in v.as_array().into_iter().flatten().filter_map(Value::as_str) {
        let s = s.trim();
        if !s.is_empty()
            && !s.contains('\0')
            && s.encode_utf16().count() <= 4096
            && !out.contains(&json!(s))
            && out.len() < cap
        {
            out.push(json!(s));
        }
    }
    json!(out)
}
/// Upgrade legacy settings to v7; unknown future schemas fail closed, preserving
/// the source. Capture consents require actual booleans, never truthy values.
pub fn settings(mut v: Value) -> Result<Value> {
    let raw = v.clone();
    let p = v.as_object_mut().ok_or(Error::Invalid)?;
    let version = p
        .get("settingsVersion")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if version > 7 {
        return Err(Error::Invalid);
    }
    let defaults = settings_defaults();
    for (k, d) in defaults.as_object().unwrap() {
        let valid = p.get(k).is_some_and(|v| match d {
            Value::Bool(_) => v.is_boolean(),
            Value::String(_) => v.is_string(),
            Value::Number(_) => v.is_number(),
            Value::Array(_) => v.is_array(),
            Value::Null => p.contains_key(k),
            _ => true,
        });
        if !valid {
            p.insert(k.clone(), d.clone());
        }
    }
    if version < 1 && p["autoPasteDelayMs"] == 50 {
        p.insert("autoPasteDelayMs".into(), json!(0));
    }
    if version < 2 {
        p.insert("overlayVerticalOffset".into(), json!(0));
    }
    if version < 7 {
        p.insert("overlayLiveWords".into(), json!(true));
    }
    if p["overlayPlacement"] == "below_menu_bar" {
        p.insert("overlayPlacement".into(), json!("floating_pill"));
    }
    for (k, choices) in [
        ("recordingMode", &["hold_down", "double_tap", "both"][..]),
        ("overlayPlacement", &["notch", "floating_pill"][..]),
    ] {
        if !allowed(&p[k], choices) {
            p.insert(k.into(), defaults[k].clone());
        }
    }
    let catalog: Value = serde_json::from_str(include_str!("settings-catalog.json")).unwrap();
    for (key, list) in [
        ("model", "models"),
        ("language", "languages"),
        ("doubleTapKey", "keys"),
        ("autoStopSilenceMs", "autoStop"),
    ] {
        if !catalog[list].as_array().unwrap().contains(&p[key]) {
            p.insert(key.into(), defaults[key].clone());
        }
    }
    for (key, min, max) in [
        ("autoPasteDelayMs", 0., 500.),
        ("overlayVerticalOffset", -12., 12.),
        ("soundCueVolume", 0., 100.),
        ("meetingRetentionDays", 0., 3650.),
        ("meetingMaxSessions", 1., 10000.),
    ] {
        p.insert(
            key.into(),
            bounded(&p[key], min, max, defaults[key].as_f64().unwrap()),
        );
    }
    for (key, list) in [
        ("captureStandbySeconds", json!([60, 300, 900])),
        ("capturePreRollMs", json!([300, 450])),
    ] {
        if !list.as_array().unwrap().contains(&p[key]) {
            p.insert(key.into(), defaults[key].clone());
        }
    }
    if raw["microphone"] == "system_default" {
        p.insert("microphoneIdMigrationComplete".into(), json!(true));
    } else if !raw
        .get("microphoneIdMigrationComplete")
        .is_some_and(Value::is_boolean)
    {
        p.insert("microphoneIdMigrationComplete".into(), json!(false));
    }
    let approved = unique_strings(&p["smartAutoApprovedDeviceIds"], 32);
    let preferred = unique_strings(&p["smartAutoPreferredDeviceIds"], 32);
    p.insert(
        "smartAutoPreferredDeviceIds".into(),
        json!(preferred
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| approved.as_array().unwrap().contains(v))
            .collect::<Vec<_>>()),
    );
    p.insert("smartAutoApprovedDeviceIds".into(), approved);
    if !raw.as_object().unwrap().contains_key("pasteLastShortcut") {
        p.insert(
            "pasteLastShortcut".into(),
            if p.get("pasteLastShortcutEnabled") == Some(&json!(true)) {
                json!("command_shift_v")
            } else {
                Value::Null
            },
        );
    }
    if !allowed(
        &p["pasteLastShortcut"],
        &["command_shift_v", "command_option_v", "command_control_v"],
    ) {
        p.insert("pasteLastShortcut".into(), Value::Null);
    }
    for key in [
        "hotkey",
        "liveTranscriptPreview",
        "overlayProcessingStyle",
        "pasteLastShortcutEnabled",
        "queryHotkey",
        "queryProvider",
        "queryExecutable",
        "queryArguments",
        "queryTimeoutSeconds",
        "queryContextLevel",
        "queryAutomaticallyCopyAnswers",
        "retainQueryHistory",
        "assistantConnection",
    ] {
        p.remove(key);
    }
    p.insert(
        "soundCueVolume".into(),
        json!(raw["soundCueVolume"]
            .as_f64()
            .unwrap_or(45.)
            .clamp(0., 100.)
            .round() as i64),
    );
    settings_nested::normalize(p, &raw, &catalog);
    p.insert("settingsVersion".into(), json!(7));
    Ok(v)
}
/// Validate durable source state. Derived appearance caches are deliberately
/// excluded: the renderer regenerates them from the complete source theme.
pub fn normalize(key: &str, value: Value) -> Result<Value> {
    let valid = match key {
        "dictation-settings" => return settings(value),
        "murmur-appearance" => return appearance::document(value),
        "murmur-theme-library" => return appearance::library(value),
        "murmur_discovery_v1" => {
            value.is_null()
                || value["version"] == 1
                    && value["checklistDismissed"].is_boolean()
                    && ["completed", "pendingHints", "dismissedHints"]
                        .iter()
                        .all(|k| {
                            value[k]
                                .as_array()
                                .is_some_and(|a| a.iter().all(Value::is_string))
                        })
                    && ["transforms", "meetings", "correctionsTaught"]
                        .iter()
                        .all(|k| value["baseline"][k].as_u64().is_some())
                    && value["baseline"]["modeBindings"].is_array()
        }
        "murmur.settings-disclosure.v1" => {
            value["version"] == 1
                && value["panes"]
                    .as_object()
                    .is_some_and(|o| o.values().all(Value::is_boolean))
        }
        "usage-dashboard-collapsed" | "murmur_onboarding_complete" => value.is_boolean(),
        "murmur-microphone-startup-benchmark-reports" => value
            .as_array()
            .is_some_and(|a| a.len() <= 10 && a.iter().all(Value::is_object)),
        _ => false,
    };
    if valid {
        if key == "murmur_discovery_v1" && !value.is_null() {
            let mut value = value;
            let completed = unique_strings(&value["completed"], 5);
            value["completed"] = json!(completed
                .as_array()
                .unwrap()
                .iter()
                .filter(|v| allowed(
                    v,
                    &[
                        "command_palette",
                        "mode_binding",
                        "meeting",
                        "correction",
                        "shortcuts"
                    ]
                ))
                .collect::<Vec<_>>());
            for k in ["pendingHints", "dismissedHints"] {
                let entries = unique_strings(&value[k], 3);
                if entries.as_array().unwrap().iter().any(|v| {
                    !allowed(
                        v,
                        &["double_tap_timing", "correct_and_teach", "browser_modes"],
                    )
                }) {
                    return Err(Error::Invalid);
                }
                value[k] = entries;
            }
            let dismissed = value["dismissedHints"].clone();
            value["pendingHints"] = json!(value["pendingHints"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|v| !dismissed.as_array().unwrap().contains(v))
                .collect::<Vec<_>>());
            value["baseline"]["modeBindings"] =
                unique_strings(&value["baseline"]["modeBindings"], usize::MAX);
            return Ok(value);
        }
        Ok(value)
    } else {
        Err(Error::Invalid)
    }
}
pub(crate) fn file(key: &str) -> Option<&'static str> {
    match key {
        "dictation-settings" => Some("settings.json"),
        "murmur-theme-library" => Some("theme-library.json"),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_settings_contract_fixtures_now_owned_by_rust() {
        let fixtures: Vec<Value> =
            serde_json::from_str(include_str!("settings-fixtures.json")).unwrap();
        let mut failed = 0;
        for (i, case) in fixtures.iter().enumerate() {
            let mut input = if case["withDefaults"] == true {
                settings_defaults()
            } else {
                json!({})
            };
            for key in case["omit"].as_array().unwrap() {
                input.as_object_mut().unwrap().remove(key.as_str().unwrap());
            }
            input
                .as_object_mut()
                .unwrap()
                .extend(case["input"].as_object().unwrap().clone());
            let mut expected = settings_defaults();
            expected
                .as_object_mut()
                .unwrap()
                .extend(case["output"].as_object().unwrap().clone());
            expected["settingsVersion"] = json!(7);
            if let Some(gate) = input.get("launchOnboardingComplete") {
                expected["launchOnboardingComplete"] = gate.clone();
            }
            let result = settings(input).unwrap();
            if result != expected {
                let differences: Vec<_> = expected
                    .as_object()
                    .unwrap()
                    .iter()
                    .filter(|(k, v)| result[*k] != **v)
                    .map(|(k, _)| k.clone())
                    .collect();
                eprintln!("synthetic schema fixture {i}: differing fields {differences:?}");
                failed += 1;
            }
        }
        assert_eq!(failed, 0);
    }
}

#[cfg(test)]
mod source_tests {
    use super::*;
    #[test]
    fn source_theme_library_normalizes_reserved_ids_and_revisions_without_react() {
        let input = json!({"version":1,"revision":2147483647u64,"themes":[{"version":1,"id":"opencode","label":"OpenCode","modes":["dark"],"theme":{"version":1,"presetId":"custom","accent":"#ABC"},"source":{"kind":"local"}}]});
        let library = normalize("murmur-theme-library", input).unwrap();
        assert_eq!(library["themes"][0]["id"], "opencode-2");
        assert_eq!(library["legacyRenamedIds"]["opencode"], "opencode-2");
        assert_eq!(library["revision"], 0);
        assert_eq!(library["themes"][0]["theme"]["accent"], "#aabbcc");
        assert_eq!(
            normalize("murmur-theme-library", library.clone()).unwrap(),
            library
        );
    }
    #[test]
    fn future_source_schema_stays_in_place_across_multiple_older_launches() {
        use crate::completion::{lifecycle::Operation, CompletionService};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        let future = br#"{"settingsVersion":8,"autoPaste":true}"#;
        std::fs::write(&path, future).unwrap();
        for _ in 0..2 {
            let service = CompletionService::start(root.path().into(), true).unwrap();
            assert!(service.lifecycle(Operation::Status).is_err());
            assert!(
                crate::completion::save_source(root.path(), "dictation-settings", "{}").is_err()
            );
            assert_eq!(std::fs::read(&path).unwrap(), future);
        }
    }
}

#[cfg(test)]
mod review_regressions {
    use super::*;
    #[test]
    fn literal_defaults_consents_smart_destination_and_correction_fuzzy() {
        let v = settings(json!({})).unwrap();
        assert_eq!(v["model"], "parakeet-tdt-0.6b-v3-coreml");
        assert_eq!(v["idleTimeoutMinutes"], 5);
        for key in [
            "autoPaste",
            "smartDestinationEnabled",
            "meetingRetainAudio",
            "meetingSuggestionsEnabled",
            "smartAutoProbeEnabled",
            "smartAutoAllowContinuity",
            "saveAudio",
            "saveTranscript",
        ] {
            assert_eq!(v[key], false, "{key}");
        }
        assert_eq!(v["retainHistory"], true);
        assert_eq!(v["overlayLiveWords"], true);
        assert_eq!(v["correctionFuzzy"], true);
        assert_eq!(
            settings(json!({"smartDestinationEnabled":true})).unwrap()["smartDestinationEnabled"],
            true
        );
        for bad in [json!("true"), json!(1), json!({}), Value::Null] {
            let v = settings(json!({"smartDestinationEnabled":bad,"correctionFuzzy":bad})).unwrap();
            assert_eq!(v["smartDestinationEnabled"], false);
            assert_eq!(v["correctionFuzzy"], true);
        }
        assert_eq!(
            settings(json!({"idleTimeoutMinutes":-1})).unwrap()["idleTimeoutMinutes"],
            -1
        );
    }
    #[test]
    fn complete_model_catalog_preserves_every_supported_model() {
        let models = [
            "parakeet-tdt-0.6b-v3-coreml",
            "tiny.en",
            "base.en",
            "small.en",
            "medium.en",
            "large-v3-turbo",
            "parakeet-tdt-0.6b-v2-fp16",
        ];
        let catalog: Value = serde_json::from_str(include_str!("settings-catalog.json")).unwrap();
        assert_eq!(catalog["models"], json!(models));
        for model in models {
            assert_eq!(settings(json!({"model":model})).unwrap()["model"], model);
        }
    }
    #[test]
    fn every_supported_browser_and_exact_legacy_host_contract() {
        let browsers = [
            "com.apple.Safari",
            "com.google.Chrome",
            "com.microsoft.edgemac",
            "com.brave.Browser",
            "company.thebrowser.Browser",
            "org.chromium.Chromium",
        ];
        for browser in browsers {
            for (host, expected) in [
                (" Foo.COM. ", Some("foo.com")),
                ("localhost", Some("localhost")),
                ("a-b.example", Some("a-b.example")),
                ("-foo.com", None),
                ("foo-.com", None),
                ("foo..com", None),
                ("foo.com..", None),
                ("foo", None),
                ("https://foo.com", None),
                ("f_o.com", None),
                ("foo.com:80", None),
                ("*.com", None),
                ("foo bar.com", None),
                ("日本.com", None),
            ] {
                let v = settings(json!({"browserSiteRules":[{"id":"rule","browserBundleId":browser,"host":host,"modeId":"builtin.everyday"}]})).unwrap();
                assert_eq!(
                    v["browserSiteRules"].as_array().unwrap().len(),
                    usize::from(expected.is_some()),
                    "{browser}/{host}"
                );
                if let Some(host) = expected {
                    assert_eq!(v["browserSiteRules"][0]["host"], host);
                }
            }
        }
        for host in [
            format!("{}.com", "a".repeat(64)),
            format!(
                "{}.{}.{}.com",
                "a".repeat(63),
                "b".repeat(63),
                "c".repeat(63)
            ) + &"d".repeat(64),
        ] {
            assert!(settings(json!({"browserSiteRules":[{"id":"r","browserBundleId":"com.apple.Safari","host":host,"modeId":"builtin.everyday"}]})).unwrap()["browserSiteRules"].as_array().unwrap().is_empty());
        }
    }
    #[test]
    fn unicode_bounds_count_utf16_for_modes_roots_vocabulary_and_theme_labels() {
        for (name, accepted) in [
            ("日".repeat(60), true),
            ("日".repeat(128), true),
            ("日".repeat(129), false),
            ("😀".repeat(64), true),
            ("😀".repeat(65), false),
        ] {
            let v = settings(json!({"modes":[{"id":"mode","name":name}],"activeModeId":"mode"}))
                .unwrap();
            assert_eq!(v["modes"].as_array().unwrap().len(), usize::from(accepted));
            assert_eq!(
                v["activeModeId"],
                if accepted { "mode" } else { "builtin.everyday" }
            );
        }
        let v=settings(json!({"appProfiles":[{"bundleId":"synthetic","ideProjectRoots":["日".repeat(4096), "😀".repeat(2049)]}], "vocabularyEntries":[{"written":"😀".repeat(129),"aliases":["日".repeat(300)]}]})).unwrap();
        assert_eq!(
            v["appProfiles"][0]["ideProjectRoots"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            v["vocabularyEntries"][0]["written"]
                .as_str()
                .unwrap()
                .encode_utf16()
                .count(),
            256
        );
        assert_eq!(
            v["vocabularyEntries"][0]["aliases"][0]
                .as_str()
                .unwrap()
                .encode_utf16()
                .count(),
            256
        );
        for (label, valid) in [
            ("日".repeat(64), true),
            ("😀".repeat(32), true),
            ("😀".repeat(33), false),
        ] {
            assert_eq!(normalize("murmur-theme-library", json!({"version":1,"themes":[{"version":1,"id":"fixture","label":label,"modes":["dark"],"theme":{"version":1},"source":{"kind":"local"}}]})).is_ok(), valid);
        }
    }
}
