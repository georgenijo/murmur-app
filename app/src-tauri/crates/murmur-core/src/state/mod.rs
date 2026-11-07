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
    unique_strings_bounded(v, cap, false)
}
fn unique_strings_bounded(v: &Value, cap: usize, utf8_bytes: bool) -> Value {
    let mut out = vec![];
    for s in v.as_array().into_iter().flatten().filter_map(Value::as_str) {
        let s = s.trim();
        if !s.is_empty()
            && !s.contains('\0')
            && (if utf8_bytes {
                s.len()
            } else {
                s.encode_utf16().count()
            }) <= 4096
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
    let approved = unique_strings_bounded(&p["smartAutoApprovedDeviceIds"], 32, true);
    let preferred = unique_strings_bounded(&p["smartAutoPreferredDeviceIds"], 32, true);
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
/// Validate a single explicit Settings edit without accepting the normalizer's
/// fallback as the user's requested value. Callers serialize current-read,
/// expected-revision comparison and publication under SOURCE_WRITE_LOCK.
/// Unknown/metadata fields are not editable preferences. Other settings,
/// including opaque future fields in the current version, remain preserved.
pub fn settings_edit(current: &Value, key: &str, value: Value) -> Result<Value> {
    if !settings_defaults().as_object().unwrap().contains_key(key)
        || matches!(
            key,
            "microphoneIdMigrationComplete"
                | "codeVocabLastScan"
                | "customVocabulary"
                | "voiceCommands"
        )
    {
        return Err(Error::Invalid);
    }
    if settings(current.clone())? != *current {
        return Err(Error::Invalid);
    }
    let mut candidate = current.clone();
    let object = candidate.as_object_mut().ok_or(Error::Invalid)?;
    object.insert(key.into(), value.clone());
    let normalized = settings(candidate)?;
    if normalized[key] != value {
        return Err(Error::Invalid);
    }
    Ok(normalized)
}

/// Validate durable source state. Derived appearance caches are deliberately
/// excluded: the renderer regenerates them from the complete source theme.
pub fn source_size_limit(key: &str) -> usize {
    match key {
        "murmur-appearance" => 65536,
        "murmur-theme-library" => 1048576,
        _ => 32 * 1024 * 1024,
    }
}
pub fn normalize(key: &str, value: Value) -> Result<Value> {
    let limit = source_size_limit(key);
    if serde_json::to_vec(&value)
        .map_err(|_| Error::Invalid)?
        .len()
        > limit
    {
        return Err(Error::Invalid);
    }
    let normalized = normalize_inner(key, value)?;
    if serde_json::to_vec(&normalized)
        .map_err(|_| Error::Invalid)?
        .len()
        > limit
    {
        return Err(Error::Invalid);
    }
    Ok(normalized)
}
fn normalize_inner(key: &str, value: Value) -> Result<Value> {
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
        "updater-last-check" => value.as_u64().is_some(),
        "pending-update-release-notes" => return crate::updater::normalize_pending(value),
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
    fn explicit_settings_edits_reject_fallbacks_and_preserve_other_fields() {
        let mut current = settings(settings_defaults()).unwrap();
        current["opaqueFutureValue"] = json!({"nested":"synthetic"});
        for (key, value) in [
            ("autoPasteDelayMs", json!(501)),
            ("soundCueVolume", json!("loud")),
            ("model", json!("unknown")),
            ("pasteLastShortcut", json!("command_q")),
            ("capturePreRollMs", json!(301)),
            ("meetingMaxSessions", json!(0)),
            ("settingsVersion", json!(8)),
            ("microphoneIdMigrationComplete", json!(false)),
        ] {
            assert!(settings_edit(&current, key, value).is_err(), "{key}");
        }
        let edited = settings_edit(&current, "autoPaste", json!(true)).unwrap();
        let mut expected = current.clone();
        expected["autoPaste"] = json!(true);
        assert_eq!(edited, expected);
        assert_eq!(settings(edited.clone()).unwrap(), edited);
        let mut future = current.clone();
        future["settingsVersion"] = json!(8);
        assert!(settings_edit(&future, "autoPaste", json!(true)).is_err());
    }

    #[test]
    fn legacy_appearance_parity_table() {
        // Literal oracles independently transcribed from fa2be313 themeLibrary.ts.
        let cases: Vec<Value> =
            serde_json::from_str(include_str!("appearance-parity-fixtures.json")).unwrap();
        for case in cases {
            let result = normalize(
                "murmur-theme-library",
                json!({"version":1,"themes":[case["entry"]]}),
            );
            assert_eq!(
                result.is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
    }
    #[test]
    fn appearance_raw_limits_precede_normalization() {
        for (key, limit, base) in [
            ("murmur-appearance", 65536, json!({"version":1})),
            (
                "murmur-theme-library",
                1048576,
                json!({"version":1,"themes":[]}),
            ),
        ] {
            let mut value = base;
            value["ignored"] = json!("a".repeat(limit));
            assert!(normalize(key, value).is_err(), "{key}");
        }
    }
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
            let mut expected: Value =
                serde_json::from_str(include_str!("settings-expected-defaults.json")).unwrap();
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
    fn unicode_bounds_preserve_each_fields_legacy_units() {
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
            258
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

#[cfg(test)]
mod round_two_contract_tests {
    use super::*;
    #[test]
    fn literal_fallbacks_do_not_depend_on_production_defaults() {
        for input in [
            json!({}),
            json!({"captureStandbyEnabled":"bad","captureStandbySeconds":301,"capturePreRollMs":451,"meetingDiarization":null,"smartFormattingEnabled":1,"hotkeyMissFeedback":"true","smartAutoStopEnabled":1,"siteModeLookupEnabled":"true","autoStopSilenceMs":13}),
        ] {
            let v = settings(input).unwrap();
            for (key, expected) in [
                ("captureStandbyEnabled", json!(false)),
                ("captureStandbySeconds", json!(300)),
                ("capturePreRollMs", json!(450)),
                ("meetingDiarization", json!(false)),
                ("smartFormattingEnabled", json!(false)),
                ("hotkeyMissFeedback", json!(false)),
                ("smartAutoStopEnabled", json!(false)),
                ("siteModeLookupEnabled", json!(false)),
                ("autoStopSilenceMs", json!(0)),
                ("language", json!("auto")),
                ("correctionEnabled", json!(true)),
                ("codeVocabLastScan", Value::Null),
            ] {
                assert_eq!(v[key], expected, "{key}");
            }
        }
    }
    #[test]
    fn supplementary_boundaries_for_every_string_limit() {
        // Vocabulary slices code points, including an odd UTF-16 boundary.
        for text in [
            "😀".repeat(129),
            "😀".repeat(256),
            format!("{}a😀", "😀".repeat(255)),
        ] {
            let expected: String = text.chars().take(256).collect();
            let v =
                settings(json!({"vocabularyEntries":[{"written":text,"aliases":[text]}]})).unwrap();
            assert_eq!(v["vocabularyEntries"][0]["written"], expected);
            assert_eq!(v["vocabularyEntries"][0]["aliases"][0], expected);
            assert!(!expected.contains('�'));
        }
        for (text, valid) in [
            ("😀".repeat(64), true),
            (format!("{}a", "😀".repeat(64)), false),
        ] {
            let v=settings(json!({"modes":[{"id":text,"name":"fixture"}],"browserSiteRules":[{"id":text,"host":"example.com","browserBundleId":"com.apple.Safari","modeId":"builtin.everyday"}]})).unwrap();
            assert_eq!(v["modes"].as_array().unwrap().len(), usize::from(valid));
            assert_eq!(
                v["browserSiteRules"].as_array().unwrap().len(),
                usize::from(valid)
            );
        }
        for (text, valid) in [
            ("😀".repeat(2048), true),
            (format!("{}a", "😀".repeat(2048)), false),
        ] {
            let v =
                settings(json!({"appProfiles":[{"bundleId":"fixture","ideProjectRoots":[text]}]}))
                    .unwrap();
            assert_eq!(
                v["appProfiles"][0]["ideProjectRoots"]
                    .as_array()
                    .unwrap()
                    .len(),
                usize::from(valid)
            );
        }
        for (text, valid) in [
            ("😀".repeat(1024), true),
            (format!("{}a", "😀".repeat(1024)), false),
            ("日".repeat(1365), true),
            ("日".repeat(1366), false),
        ] {
            let v = settings(
                json!({"smartAutoApprovedDeviceIds":[text],"smartAutoPreferredDeviceIds":[text]}),
            )
            .unwrap();
            for key in ["smartAutoApprovedDeviceIds", "smartAutoPreferredDeviceIds"] {
                assert_eq!(v[key].as_array().unwrap().len(), usize::from(valid));
            }
        }
        for (text, valid) in [
            ("😀".repeat(64), false),
            (format!("{}a", "😀".repeat(64)), false),
        ] {
            assert_eq!(normalize("murmur-theme-library",json!({"version":1,"themes":[{"version":1,"id":"fixture","label":"Fixture","modes":["dark"],"theme":{"version":1},"source":{"kind":"open-vsx","extensionId":text,"version":"1","license":"MIT"}}]})).is_ok(), valid);
        }
        for (url, valid) in [
            (format!("https://example.com/{}", "😀".repeat(1014)), true),
            (format!("https://example.com/{}a", "😀".repeat(1014)), false),
        ] {
            assert_eq!(normalize("murmur-theme-library",json!({"version":1,"themes":[{"version":1,"id":"fixture","label":"Fixture","modes":["dark"],"theme":{"version":1},"source":{"kind":"open-vsx","extensionId":"fixture","version":"1","license":"MIT","sourceUrl":url}}]})).is_ok(), valid);
        }
    }
}
