//! Versioned WebKit lifecycle, serialized by the existing completion owner.
//! The transport supplies raw strings, so malformed JSON can never masquerade
//! as an empty store. No code here depends on WebKit's private disk layout.
use super::*;
use std::collections::BTreeMap;

pub const V2_KEYS: &[&str] = &[
    "dictation-settings",
    "murmur-theme-library",
    "murmur-appearance",
    "murmur_discovery_v1",
    "murmur.settings-disclosure.v1",
    "usage-dashboard-collapsed",
    "murmur-microphone-startup-benchmark-reports",
    "murmur_onboarding_complete",
];
pub const UPDATER_V3_KEYS: &[&str] = &["updater-last-check", "pending-update-release-notes"];
pub const TRANSCRIPT_KEYS: &[&str] = &[
    "dictation-history",
    "dictation-stats",
    "murmur-benchmark-report",
    "murmur-benchmark-reports",
];
pub const DROP_KEYS: &[&str] = &["murmur-overlay-vertical-offset", "murmur-ui-latency-v1"];
const UI_STATE: &str = "ui-state.json";
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Progress {
    pub v2: bool,
    /// Reserved for #990. This release never commits v3.
    pub v3: bool,
    pub retirement_started: bool,
    pub retired: bool,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct UiState {
    schema: u32,
    v2_imported: bool,
    values: BTreeMap<String, Value>,
    destinations: Vec<String>,
}
impl Default for UiState {
    fn default() -> Self {
        Self {
            schema: 1,
            v2_imported: false,
            values: BTreeMap::new(),
            destinations: vec![],
        }
    }
}
#[derive(Clone)]
pub enum Operation {
    Status,
    /// Complete observed key set, not a partial snapshot. Missing values must
    /// be represented by null. Storage read errors must abort the transport.
    Import(BTreeMap<String, Option<String>>),
    Save {
        key: String,
        value: Value,
    },
    /// Host must possess the no-React startup barrier. v3 gates this operation.
    BeginRetirement,
    /// Only after WebKit purge, empty-record verification, release of all
    /// webviews and durable directory removal. Repeating the operation is safe.
    Retired,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub v1: bool,
    pub v2: bool,
    pub v3: bool,
    pub transcript_keys_removed: bool,
    pub retirement_started: bool,
    pub retired: bool,
    pub values: BTreeMap<String, Value>,
}
fn decode(raw: &BTreeMap<String, Option<String>>, key: &str) -> Result<Option<Value>> {
    match raw.get(key).ok_or(Error::Invalid)? {
        None => Ok(None),
        Some(s) if s.len() <= crate::state::source_size_limit(key) => serde_json::from_str(s)
            .map(Some)
            .map_err(|_| Error::Invalid),
        _ => Err(Error::Invalid),
    }
}
fn valid_ui(v: &UiState) -> bool {
    v.schema == 1
        && v.destinations
            .iter()
            .enumerate()
            .all(|(i, k)| crate::state::file(k).is_some() && !v.destinations[..i].contains(k))
        && v.values
            .iter()
            .all(|(k, v)| crate::state::normalize(k, v.clone()).is_ok())
}
pub(super) fn valid_snapshot(value: &Value) -> bool {
    serde_json::from_value::<UiState>(value.clone()).is_ok_and(|v| valid_ui(&v))
}
impl Engine {
    fn ui_state(&self) -> Result<UiState> {
        self.storage.reject_future(UI_STATE, "schema", 1)?;
        let existed = self.storage.root.join(UI_STATE).exists();
        let state = self.storage.read(UI_STATE, valid_ui)?;
        if state.is_none()
            && (existed || self.migration()?.lifecycle.v2 || self.storage.quarantined(UI_STATE)?)
        {
            return Err(Error::Invalid);
        }
        Ok(state.unwrap_or_default())
    }
    fn state_values(&self) -> Result<BTreeMap<String, Value>> {
        let mut values = self.ui_state()?.values;
        for key in ["dictation-settings", "murmur-theme-library"] {
            if let Some(v) = self.state_file(key)? {
                values.insert(key.into(), v);
            }
        }
        Ok(values)
    }
    fn state_file(&self, key: &str) -> Result<Option<Value>> {
        recover_source(&self.storage.root, key)?;
        let file = crate::state::file(key).ok_or(Error::Invalid)?;
        self.storage.reject_future(
            file,
            if key == "dictation-settings" {
                "settingsVersion"
            } else {
                "version"
            },
            if key == "dictation-settings" { 7 } else { 1 },
        )?;
        let existed = self.storage.root.join(file).exists();
        let value: Option<Value> = self.storage.read(file, |v: &Value| {
            crate::state::normalize(key, v.clone()).is_ok()
        })?;
        if value.is_none()
            && (existed
                || self.ui_state()?.destinations.iter().any(|k| k == key)
                || source_required(&self.storage.root, key)?)
        {
            return Err(Error::Invalid);
        }
        value
            .map(|v: Value| crate::state::normalize(key, v))
            .transpose()
    }
    pub(super) fn lifecycle(&mut self, op: Operation) -> Result<Receipt> {
        if let Some(previous) = self.failed_source.take() {
            if let Err(error) = self.lifecycle_inner(previous.clone()) {
                self.failed_source = Some(previous);
                return Err(error);
            }
        }
        let result = self.lifecycle_inner(op.clone());
        if result.is_err() && matches!(op, Operation::Save { .. }) {
            self.failed_source = Some(op);
        }
        result
    }
    fn lifecycle_inner(&mut self, op: Operation) -> Result<Receipt> {
        self.storage.retry_intents()?;
        self.storage.recover_snapshot(UI_STATE, valid_snapshot)?;
        self.storage.sync_directory()?;
        let mut m = self.migration()?;
        let mut state = self.ui_state()?;
        match op {
            Operation::Status => {
                if !self.webkit_exists && !m.lifecycle.v2 {
                    let source = TRANSCRIPT_KEYS
                        .iter()
                        .chain(V2_KEYS)
                        .map(|k| ((*k).into(), None))
                        .collect();
                    self.lifecycle(Operation::Import(source))?;
                    m = self.migration()?;
                    m.keys_removed = vec![
                        Destination::History,
                        Destination::Statistics,
                        Destination::Reports,
                    ];
                    self.storage.write(MIGRATION, &m)?;
                }
            }
            Operation::Save { key, value } => {
                let value = crate::state::normalize(&key, value)?;
                if key == "murmur_onboarding_complete" {
                    // Patch the authoritative settings, never a cache snapshot.
                    // No settings yet: keep the gate in UI state until creation.
                    if let Some(mut settings) = self.state_file("dictation-settings")? {
                        settings["launchOnboardingComplete"] = value.clone();
                        publish_source(&mut self.storage, "dictation-settings", &settings)?;
                    }
                }
                if crate::state::file(&key).is_some() {
                    recover_source(&self.storage.root, &key)?;
                    publish_source(&mut self.storage, &key, &value)?;
                } else {
                    state.values.insert(key, value);
                    self.storage.publish_snapshot(UI_STATE, &state)?;
                }
            }
            Operation::Import(raw) => {
                if !m.v1 {
                    // Valid disk destinations (including a 1.3 Clear) take
                    // priority even over malformed obsolete source data.
                    let mut values = vec![];
                    for (d, key) in [
                        (Destination::History, "dictation-history"),
                        (Destination::Statistics, "dictation-stats"),
                        (Destination::Reports, "murmur-benchmark-reports"),
                    ] {
                        let doc = self.document(d)?;
                        let v = if doc.is_some() {
                            None
                        } else {
                            let v = decode(&raw, key)?;
                            if d == Destination::Reports && v.is_none() {
                                decode(&raw, "murmur-benchmark-report")?.map(|v| json!([v]))
                            } else {
                                v
                            }
                        };
                        values.push(v);
                    }
                    self.import(LegacyImport {
                        history: values[0].clone(),
                        statistics: values[1].clone(),
                        reports: values[2].clone(),
                    })?;
                    m = self.migration()?;
                }
                if !m.lifecycle.v2 {
                    if !state.v2_imported {
                        let mut all = self.state_values()?;
                        // Validate all source values before publishing any v2 data.
                        for key in V2_KEYS {
                            if all.contains_key(*key) {
                                continue;
                            }
                            if let Some(v) = decode(&raw, key)? {
                                all.insert((*key).into(), crate::state::normalize(key, v)?);
                            }
                        }
                        // Keep reserved-ID renames and selected halves consistent,
                        // including a crash after library publication before UI state.
                        let renames = all
                            .get("murmur-theme-library")
                            .and_then(|v| v.get("legacyRenamedIds"))
                            .cloned();
                        if let (Some(renames), Some(appearance)) =
                            (renames, all.get_mut("murmur-appearance"))
                        {
                            for mode in ["light", "dark"] {
                                if let Some(old) = appearance["selection"][mode].as_str() {
                                    if let Some(new) = renames.get(old) {
                                        appearance["selection"][mode] = new.clone();
                                    }
                                }
                            }
                        }
                        // Legacy onboarding fills only an absent settings field.
                        if let Some(gate) = all.get("murmur_onboarding_complete").cloned() {
                            if let Some(settings) = all
                                .get_mut("dictation-settings")
                                .and_then(Value::as_object_mut)
                            {
                                settings.entry("launchOnboardingComplete").or_insert(gate);
                            }
                        }
                        for key in ["dictation-settings", "murmur-theme-library"] {
                            if let Some(value) = all.remove(key) {
                                self.storage
                                    .write(crate::state::file(key).unwrap(), &value)?;
                                state.destinations.push(key.into());
                            }
                        }
                        state.values = all;
                        state.v2_imported = true;
                        self.storage.publish_snapshot(UI_STATE, &state)?;
                    }
                    m.lifecycle.v2 = true;
                    m.schema = 1;
                    self.storage.write(MIGRATION, &m)?;
                }
            }
            Operation::BeginRetirement | Operation::Retired => {
                // Flags never replace destination evidence, including explicit empties.
                self.state_values()?;
                for d in [
                    Destination::History,
                    Destination::Statistics,
                    Destination::Reports,
                ] {
                    if !self.document(d)?.is_some_and(|d| d.v1_imported) {
                        return Err(Error::Missing);
                    }
                }
                if !(m.v1 && m.lifecycle.v2 && m.lifecycle.v3 && m.keys_removed.len() == 3) {
                    return Err(Error::Invalid);
                }
                if matches!(op, Operation::Retired) {
                    if !m.lifecycle.retirement_started {
                        return Err(Error::Invalid);
                    }
                    m.lifecycle.retired = true;
                } else {
                    m.lifecycle.retirement_started = true;
                }
                self.storage.write(MIGRATION, &m)?;
            }
        }
        m = self.migration()?;
        Ok(Receipt {
            v1: m.v1,
            v2: m.lifecycle.v2,
            v3: m.lifecycle.v3,
            transcript_keys_removed: m.keys_removed.len() == 3,
            retirement_started: m.lifecycle.retirement_started,
            retired: m.lifecycle.retired,
            values: self.state_values()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{engine, reopen};
    use super::*;
    pub(super) fn source() -> BTreeMap<String, Option<String>> {
        TRANSCRIPT_KEYS
            .iter()
            .chain(V2_KEYS)
            .chain(DROP_KEYS)
            .chain(UPDATER_V3_KEYS)
            .map(|k| ((*k).into(), None))
            .collect()
    }
    fn appearance() -> Value {
        json!({"version":1,"mode":"system","revision":9,"theme":{"version":1,"presetId":"custom","accent":"#abcdef","dark":{"background":"#123456"}},"selection":{"light":"sonic","dark":"custom"},"cache":{"obsolete":true}})
    }
    #[test]
    fn unmigrated_direct_062_import_preserves_custom_source_without_library() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = engine(dir.path(), true);
        let mut raw = source();
        raw.insert("murmur-appearance".into(), Some(appearance().to_string()));
        raw.insert(
            "dictation-settings".into(),
            Some(
                json!({"settingsVersion":0,"autoPasteDelayMs":50,"overlayVerticalOffset":48})
                    .to_string(),
            ),
        );
        let r = e.lifecycle(Operation::Import(raw)).unwrap();
        assert!(r.v1 && r.v2 && !r.v3);
        assert_eq!(
            r.values["murmur-appearance"]["theme"],
            appearance()["theme"]
        );
        assert!(r.values["murmur-appearance"].get("cache").is_none());
        assert_eq!(r.values["dictation-settings"]["overlayVerticalOffset"], 0);
        assert!(e.lifecycle(Operation::BeginRetirement).is_err());
    }
    #[test]
    fn valid_pre_v1_disk_wins_over_corrupt_obsolete_webkit() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = engine(dir.path(), true);
        std::fs::write(dir.path().join("history.json"), b"[]").unwrap();
        let mut raw = source();
        raw.insert("dictation-history".into(), Some("{bad".into()));
        let result = e.lifecycle(Operation::Import(raw)).unwrap();
        assert!(result.v1 && result.v2);
        assert!(e.snapshot().unwrap().history.is_empty());
    }
    #[test]
    fn interrupted_import_and_already_imported_fill_only_and_marker_loss() {
        for boundary in [
            storage::Boundary::BeforeRename,
            storage::Boundary::AfterRename,
        ] {
            for file in [UI_STATE, MIGRATION, "settings.json"] {
                let dir = tempfile::tempdir().unwrap();
                let mut e = engine(dir.path(), true);
                e.import(LegacyImport::default()).unwrap();
                let mut raw = source();
                raw.insert("murmur-appearance".into(), Some(appearance().to_string()));
                raw.insert(
                    "dictation-settings".into(),
                    Some(json!({"autoPaste":true}).to_string()),
                );
                e.storage.fail = Some((file.into(), boundary));
                assert!(e.lifecycle(Operation::Import(raw.clone())).is_err());
                e = reopen(&e);
                let r = e.lifecycle(Operation::Import(raw.clone())).unwrap();
                assert!(r.v2);
                assert_eq!(r.values["dictation-settings"]["autoPaste"], true);
                e.lifecycle(Operation::Save {
                    key: "usage-dashboard-collapsed".into(),
                    value: json!(false),
                })
                .unwrap();
                std::fs::remove_file(dir.path().join(MIGRATION)).unwrap();
                let r = e.lifecycle(Operation::Import(raw)).unwrap();
                assert_eq!(r.values["usage-dashboard-collapsed"], false);
            }
        }
    }
    #[test]
    fn corrupt_source_and_future_disk_preserve_data_and_forbid_retirement() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = engine(dir.path(), true);
        let mut raw = source();
        raw.insert("murmur-appearance".into(), Some("{bad".into()));
        assert!(e.lifecycle(Operation::Import(raw.clone())).is_err());
        assert!(!e.migration().unwrap().lifecycle.v2);
        std::fs::write(dir.path().join(UI_STATE), b"{bad").unwrap();
        assert!(e.lifecycle(Operation::Import(raw)).is_err());
        assert!(std::fs::read_dir(dir.path()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("quarantine")));
        assert!(e.lifecycle(Operation::BeginRetirement).is_err());
    }
    #[test]
    fn clear_on_13_then_update_does_not_resurrect_even_corrupt_legacy() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = engine(dir.path(), true);
        e.clear(Destination::History, 1).unwrap();
        let mut raw = source();
        raw.insert("dictation-history".into(), Some("bad".into()));
        assert!(e.lifecycle(Operation::Import(raw)).unwrap().v2);
        assert!(e.snapshot().unwrap().history.is_empty());
    }
    #[test]
    fn sequential_versions_and_jump_to_future_v3_contract_retirement_recovery() {
        for sequential in [false, true] {
            for boundary in [
                storage::Boundary::BeforeRename,
                storage::Boundary::AfterRename,
            ] {
                let dir = tempfile::tempdir().unwrap();
                let mut e = engine(dir.path(), true);
                if sequential {
                    e.import(LegacyImport::default()).unwrap();
                    e = reopen(&e);
                }
                e.lifecycle(Operation::Import(source())).unwrap();
                assert!(e.lifecycle(Operation::BeginRetirement).is_err());
                // Model #990's durable updater receipt; production has no v3 writer.
                let mut m = e.migration().unwrap();
                m.lifecycle.v3 = true;
                e.storage.write(MIGRATION, &m).unwrap();
                e.execute(Command::KeysRemoved(vec![
                    Destination::History,
                    Destination::Statistics,
                    Destination::Reports,
                ]))
                .unwrap();
                e.storage.fail = Some((MIGRATION.into(), boundary));
                assert!(e.lifecycle(Operation::BeginRetirement).is_err());
                e = reopen(&e);
                e.lifecycle(Operation::BeginRetirement).unwrap();
                e.storage.fail = Some((MIGRATION.into(), boundary));
                assert!(e.lifecycle(Operation::Retired).is_err());
                e = reopen(&e);
                assert!(e.lifecycle(Operation::Retired).unwrap().retired);
            }
        }
    }
}

#[cfg(test)]
mod removal_tests {
    use super::*;
    #[test]
    fn interrupted_removal_ack_retries_without_importing_cleared_content() {
        for at in [
            storage::Boundary::BeforeRename,
            storage::Boundary::AfterRename,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut e = super::super::tests::engine(dir.path(), true);
            e.import(LegacyImport::default()).unwrap();
            e.storage.fail = Some((MIGRATION.into(), at));
            let destinations = vec![
                Destination::History,
                Destination::Statistics,
                Destination::Reports,
            ];
            assert!(e
                .execute(Command::KeysRemoved(destinations.clone()))
                .is_err());
            e = super::super::tests::reopen(&e);
            e.execute(Command::KeysRemoved(destinations)).unwrap();
            assert!(
                e.lifecycle(Operation::Status)
                    .unwrap()
                    .transcript_keys_removed
            );
            assert!(e.snapshot().unwrap().history.is_empty());
        }
    }
    #[test]
    fn appearance_pre_library_and_empty_library_keep_complete_theme_source() {
        for selection in [None, Some(json!({"light":"sonic","dark":"custom"}))] {
            let dir = tempfile::tempdir().unwrap();
            let mut e = super::super::tests::engine(dir.path(), true);
            let mut a = json!({"version":1,"mode":"system","theme":{"version":1,"presetId":"custom","light":{"primary":"#123456"},"dark":{"primary":"#abcdef"}}});
            if let Some(s) = selection {
                a["selection"] = s;
            }
            let mut raw: BTreeMap<String, Option<String>> = TRANSCRIPT_KEYS
                .iter()
                .chain(V2_KEYS)
                .map(|k| ((*k).into(), None))
                .collect();
            raw.insert("murmur-appearance".into(), Some(a.to_string()));
            raw.insert(
                "murmur-theme-library".into(),
                Some(json!({"version":1,"revision":0,"themes":[]}).to_string()),
            );
            let r = e.lifecycle(Operation::Import(raw)).unwrap();
            assert_eq!(
                r.values["murmur-appearance"],
                crate::state::normalize("murmur-appearance", a).unwrap()
            );
            assert!(r.values["murmur-theme-library"]["themes"]
                .as_array()
                .unwrap()
                .is_empty());
        }
    }
}

#[cfg(test)]
mod review_safety {
    use super::super::tests::{engine, reopen};
    use super::tests::source;
    use super::*;
    use crate::completion::storage::Boundary;
    #[test]
    fn missing_or_quarantined_v1_destinations_never_authorize_removal_or_retirement() {
        for file in ["history.json", "stats.json", "benchmark-reports.json"] {
            for loss in 0..3 {
                let dir = tempfile::tempdir().unwrap();
                let service = CompletionService::start(dir.path().into(), true).unwrap();
                service.lifecycle(Operation::Import(source())).unwrap();
                service
                    .execute(Command::KeysRemoved(vec![
                        Destination::History,
                        Destination::Statistics,
                        Destination::Reports,
                    ]))
                    .unwrap();
                let migration = dir.path().join(MIGRATION);
                let mut m: Value =
                    serde_json::from_slice(&std::fs::read(&migration).unwrap()).unwrap();
                m["lifecycle"]["v3"] = json!(true);
                std::fs::write(&migration, serde_json::to_vec(&m).unwrap()).unwrap();
                assert!(service.lifecycle(Operation::BeginRetirement).is_ok());
                let path = dir.path().join(file);
                if loss == 2 {
                    let mut doc: Value =
                        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                    doc["v1_imported"] = json!(false);
                    std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
                } else if loss == 1 {
                    std::fs::write(&path, b"{bad").unwrap();
                } else {
                    std::fs::remove_file(&path).unwrap();
                }
                for _ in 0..3 {
                    assert!(service.execute(Command::Snapshot).is_err());
                    assert!(service.lifecycle(Operation::Status).is_err());
                    assert!(service.lifecycle(Operation::Import(source())).is_err());
                    assert!(service.lifecycle(Operation::BeginRetirement).is_err());
                    assert!(service.lifecycle(Operation::Retired).is_err());
                }
                drop(service);
                let service = CompletionService::start(dir.path().into(), true).unwrap();
                assert!(service.execute(Command::Snapshot).is_err());
            }
        }
    }
    #[test]
    fn imported_and_created_separate_destinations_remain_required_after_loss_or_quarantine() {
        for key in ["dictation-settings", "murmur-theme-library"] {
            for created in 0..3 {
                for corrupt in [false, true] {
                    let dir = tempfile::tempdir().unwrap();
                    let mut e = engine(dir.path(), true);
                    let value = if key == "dictation-settings" {
                        json!({})
                    } else {
                        json!({"version":1,"themes":[]})
                    };
                    let mut raw = source();
                    if created == 0 {
                        raw.insert(key.into(), Some(value.to_string()));
                    }
                    e.lifecycle(Operation::Import(raw)).unwrap();
                    if created == 1 {
                        e.lifecycle(Operation::Save {
                            key: key.into(),
                            value: value.clone(),
                        })
                        .unwrap();
                    }
                    if created == 2 {
                        super::super::save_source(dir.path(), key, &value.to_string()).unwrap();
                    }
                    let mut m = e.migration().unwrap();
                    m.lifecycle.v3 = true;
                    m.keys_removed = vec![
                        Destination::History,
                        Destination::Statistics,
                        Destination::Reports,
                    ];
                    e.storage.write(MIGRATION, &m).unwrap();
                    assert!(e.lifecycle(Operation::BeginRetirement).is_ok());
                    let path = dir.path().join(crate::state::file(key).unwrap());
                    if corrupt {
                        std::fs::write(&path, b"{bad").unwrap();
                    } else {
                        std::fs::remove_file(&path).unwrap();
                    }
                    for _ in 0..3 {
                        e = reopen(&e);
                        assert!(e.lifecycle(Operation::Status).is_err());
                        assert!(e.lifecycle(Operation::Import(source())).is_err());
                        assert!(e.lifecycle(Operation::BeginRetirement).is_err());
                    }
                }
            }
        }
    }
    #[test]
    fn repeated_corrupt_ui_recovery_with_committed_v2_is_never_empty_success() {
        for missing_destination_receipt in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut e = engine(dir.path(), true);
            e.lifecycle(Operation::Import(source())).unwrap();
            if missing_destination_receipt {
                let path = dir.path().join(UI_STATE);
                let mut v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                v.as_object_mut().unwrap().remove("destinations");
                std::fs::write(path, serde_json::to_vec(&v).unwrap()).unwrap();
            } else {
                std::fs::write(dir.path().join(UI_STATE), b"{bad").unwrap();
            }
            for _ in 0..3 {
                e = reopen(&e);
                assert!(e.lifecycle(Operation::Status).is_err());
                assert!(e.lifecycle(Operation::Import(source())).is_err());
            }
        }
    }
    #[test]
    fn library_rename_and_appearance_publication_failure_matrix() {
        for file in ["theme-library.json", UI_STATE] {
            for boundary in [
                storage::Boundary::BeforeRename,
                storage::Boundary::AfterRename,
                storage::Boundary::DirectorySync,
            ] {
                let dir = tempfile::tempdir().unwrap();
                let mut e = engine(dir.path(), true);
                e.import(LegacyImport::default()).unwrap();
                let mut raw = source();
                raw.insert("murmur-theme-library".into(),Some(json!({"version":1,"themes":[{"version":1,"id":"opencode","label":"Synthetic","modes":["dark"],"theme":{"version":1},"source":{"kind":"local"}}]}).to_string()));
                raw.insert("murmur-appearance".into(),Some(json!({"version":1,"theme":{"version":1},"selection":{"light":"sonic","dark":"opencode"}}).to_string()));
                e.storage.fail = Some((file.into(), boundary));
                assert!(e.lifecycle(Operation::Import(raw.clone())).is_err());
                assert!(e.storage.fail.is_none());
                e = reopen(&e);
                let receipt = e.lifecycle(Operation::Import(raw)).unwrap();
                let id = &receipt.values["murmur-theme-library"]["themes"][0]["id"];
                assert_eq!(
                    receipt.values["murmur-appearance"]["selection"]["dark"],
                    *id
                );
            }
        }
    }
    #[test]
    fn corrupt_source_matrix_rejects_every_source_form_without_v2_receipt() {
        for key in V2_KEYS {
            for bad in [
                "{truncated".to_string(),
                "42".into(),
                "[null]".into(),
                "x".repeat(32 * 1024 * 1024 + 1),
                json!({"version":999,"settingsVersion":999}).to_string(),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let mut e = engine(dir.path(), true);
                let mut raw = source();
                raw.insert((*key).into(), Some(bad));
                assert!(e.lifecycle(Operation::Import(raw)).is_err(), "{key}");
                assert!(!e.migration().unwrap().lifecycle.v2);
                assert!(e.lifecycle(Operation::BeginRetirement).is_err());
            }
        }
    }
    #[test]
    fn corrupt_transcript_source_matrix_preserves_all_four_keys() {
        for key in TRANSCRIPT_KEYS {
            for bad in [
                "{truncated".to_string(),
                "42".into(),
                "[null]".into(),
                "x".repeat(32 * 1024 * 1024 + 1),
                json!({"schema":999}).to_string(),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let mut e = engine(dir.path(), true);
                let mut raw = source();
                raw.insert((*key).into(), Some(bad));
                // The plural reports key must be absent when testing its old single form.
                assert!(e.lifecycle(Operation::Import(raw)).is_err(), "{key}");
                assert!(!e.snapshot().unwrap().remove_keys.iter().any(|k| k == key));
                assert!(!e.migration().unwrap().v1);
            }
        }
    }
    #[test]
    fn settings_migrations_publish_version_and_alias_scope_round_trip_on_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = engine(dir.path(), true);
        let mut raw = source();
        raw.insert("dictation-settings".into(),Some(json!({"autoPasteDelayMs":50,"overlayVerticalOffset":48,"idleTimeoutMinutes":-1,"vocabularyEntries":[{"id":"v","written":"Synthetic","aliases":["sin thetic"],"scope":{"kind":"project","bundleId":"synthetic.app","root":"/synthetic"}}]}).to_string()));
        e.lifecycle(Operation::Import(raw)).unwrap();
        let disk: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(disk["autoPasteDelayMs"], 0);
        assert_eq!(disk["settingsVersion"], 7);
        assert_eq!(disk["overlayVerticalOffset"], 0);
        assert_eq!(disk["idleTimeoutMinutes"], -1);
        e = reopen(&e);
        let r = e.lifecycle(Operation::Status).unwrap();
        assert_eq!(
            r.values["dictation-settings"]["vocabularyEntries"],
            disk["vocabularyEntries"]
        );
        assert_eq!(
            disk["vocabularyEntries"][0]["aliases"],
            json!(["sin thetic"])
        );
        assert_eq!(disk["vocabularyEntries"][0]["scope"]["root"], "/synthetic");
    }
    #[test]
    fn interrupted_first_source_publication_replays_exact_intent_before_merge() {
        for key in ["dictation-settings", "murmur-theme-library"] {
            for boundary in [
                Boundary::BeforeRename,
                Boundary::AfterRename,
                Boundary::DirectorySync,
            ] {
                let dir = tempfile::tempdir().unwrap();
                let mut e = engine(dir.path(), true);
                e.lifecycle(Operation::Import(source())).unwrap();
                let value = if key == "dictation-settings" {
                    json!({"language":"es","autoPasteDelayMs":23,"microphone":"system_default"})
                } else {
                    json!({"version":1,"revision":17,"themes":[]})
                };
                let expected = crate::state::normalize(key, value.clone()).unwrap();
                let file = crate::state::file(key).unwrap();
                e.storage.fail = Some((file.into(), boundary));
                assert!(e
                    .lifecycle(Operation::Save {
                        key: key.into(),
                        value
                    })
                    .is_err());
                assert!(dir.path().join(format!("{file}.required")).exists());
                if boundary == Boundary::BeforeRename {
                    assert!(!dir.path().join(file).exists());
                }
                // Relaunch through public service, exercising status and save_source.
                drop(e);
                let service = CompletionService::start(dir.path().into(), true).unwrap();
                let state = service.lifecycle(Operation::Status).unwrap();
                assert_eq!(state.values[key], expected);
                assert!(!dir.path().join(format!("{file}.pending")).exists());
                save_source(dir.path(), key, &expected.to_string()).unwrap();
                assert_eq!(
                    service.lifecycle(Operation::Status).unwrap().values[key],
                    expected
                );
            }
        }
    }
    #[test]
    fn onboarding_patch_preserves_authoritative_settings_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = engine(dir.path(), true);
        e.lifecycle(Operation::Import(source())).unwrap();
        save_source(
            dir.path(),
            "dictation-settings",
            r#"{"language":"ja","autoPasteDelayMs":23}"#,
        )
        .unwrap();
        for complete in [true, false] {
            e.lifecycle(Operation::Save {
                key: "murmur_onboarding_complete".into(),
                value: json!(complete),
            })
            .unwrap();
            e = reopen(&e);
            let r = e.lifecycle(Operation::Status).unwrap();
            assert_eq!(
                r.values["dictation-settings"]["launchOnboardingComplete"],
                complete
            );
            assert_eq!(r.values["dictation-settings"]["language"], "ja");
            assert_eq!(r.values["dictation-settings"]["autoPasteDelayMs"], 23);
        }
    }
    #[test]
    fn settings_migrations_publish_literal_unchanged_delay_smart_destination_and_shortcut_removal()
    {
        for (input, delay, smart) in [
            (
                json!({"settingsVersion":6,"autoPasteDelayMs":23}),
                23,
                false,
            ),
            (
                json!({"settingsVersion":6,"autoPasteDelayMs":23,"smartDestinationEnabled":"true"}),
                23,
                false,
            ),
            (
                json!({"settingsVersion":7,"autoPasteDelayMs":23,"pasteLastShortcutEnabled":true}),
                23,
                false,
            ),
            (
                json!({"settingsVersion":6,"autoPasteDelayMs":23,"smartDestinationEnabled":true,"pasteLastShortcutEnabled":false}),
                23,
                true,
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut e = engine(dir.path(), true);
            let mut raw = source();
            raw.insert("dictation-settings".into(), Some(input.to_string()));
            e.lifecycle(Operation::Import(raw)).unwrap();
            let disk: Value =
                serde_json::from_slice(&std::fs::read(dir.path().join("settings.json")).unwrap())
                    .unwrap();
            assert_eq!(disk["autoPasteDelayMs"], delay);
            assert_eq!(disk["smartDestinationEnabled"], smart);
            assert!(disk.get("pasteLastShortcutEnabled").is_none());
            e = reopen(&e);
            let restarted = e.lifecycle(Operation::Status).unwrap();
            assert_eq!(
                restarted.values["dictation-settings"]["autoPasteDelayMs"],
                23
            );
            assert_eq!(
                restarted.values["dictation-settings"]["smartDestinationEnabled"],
                smart
            );
            assert!(restarted.values["dictation-settings"]
                .get("pasteLastShortcutEnabled")
                .is_none());
        }
    }
    #[test]
    fn malformed_pending_publication_never_seeds_defaults_on_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = engine(dir.path(), true);
        e.lifecycle(Operation::Import(source())).unwrap();
        std::fs::write(
            dir.path().join("settings.json.pending"),
            r#"{"schema":1,"value":{}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("settings.json.required"), b"true").unwrap();
        for _ in 0..3 {
            e = reopen(&e);
            assert!(e.lifecycle(Operation::Status).is_err());
            assert!(e.lifecycle(Operation::Import(source())).is_err());
            assert!(!dir.path().join("settings.json").exists());
        }
    }
}

#[cfg(test)]
mod round_three_limits {
    use super::*;
    #[test]
    fn raw_appearance_and_library_limits_are_checked_before_import_or_publication() {
        for (key, limit, base) in [
            (
                "murmur-appearance",
                65536,
                json!({"version":1,"mode":"dark","theme":{"version":1,"presetId":"custom"}}),
            ),
            (
                "murmur-theme-library",
                1048576,
                json!({"version":1,"revision":2,"themes":[]}),
            ),
        ] {
            for extra in [0, 1] {
                let mut raw = base.to_string();
                raw.push_str(&" ".repeat(limit + extra - raw.len()));
                let sources = BTreeMap::from([(key.into(), Some(raw.clone()))]);
                assert_eq!(decode(&sources, key).is_ok(), extra == 0);
                let dir = tempfile::tempdir().unwrap();
                let service = CompletionService::start(dir.path().into(), true).unwrap();
                let mut sources = TRANSCRIPT_KEYS
                    .iter()
                    .chain(V2_KEYS)
                    .map(|k| ((*k).into(), None))
                    .collect::<BTreeMap<_, _>>();
                sources.insert(key.into(), Some(raw.clone()));
                assert_eq!(
                    service.lifecycle(Operation::Import(sources)).is_ok(),
                    extra == 0,
                    "{key}/{extra}"
                );
                if extra == 1 {
                    assert!(!dir.path().join(UI_STATE).exists());
                    assert!(!dir.path().join("theme-library.json").exists());
                    assert!(save_source(dir.path(), key, &raw).is_err());
                }
            }
        }
    }
}
