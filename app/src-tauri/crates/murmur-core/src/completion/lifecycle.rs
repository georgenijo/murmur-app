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
}
impl Default for UiState {
    fn default() -> Self {
        Self {
            schema: 1,
            v2_imported: false,
            values: BTreeMap::new(),
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
        Some(s) if s.len() <= 32 * 1024 * 1024 => serde_json::from_str(s)
            .map(Some)
            .map_err(|_| Error::Invalid),
        _ => Err(Error::Invalid),
    }
}
impl Engine {
    fn ui_state(&self) -> Result<UiState> {
        self.storage.reject_future(UI_STATE, "schema", 1)?;
        let existed = self.storage.root.join(UI_STATE).exists();
        let state = self.storage.read(UI_STATE, |v: &UiState| {
            v.schema == 1
                && v.values
                    .iter()
                    .all(|(k, v)| crate::state::normalize(k, v.clone()).is_ok())
        })?;
        if state.is_none() && (existed || self.migration()?.lifecycle.v2) {
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
        if existed && value.is_none() {
            return Err(Error::Invalid);
        }
        value
            .map(|v: Value| crate::state::normalize(key, v))
            .transpose()
    }
    pub(super) fn lifecycle(&mut self, op: Operation) -> Result<Receipt> {
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
                if let Some(file) = crate::state::file(&key) {
                    self.storage.write(file, &value)?;
                } else {
                    state.values.insert(key, value);
                    self.storage.write(UI_STATE, &state)?;
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
                            }
                        }
                        state.values = all;
                        state.v2_imported = true;
                        self.storage.write(UI_STATE, &state)?;
                    }
                    m.lifecycle.v2 = true;
                    m.schema = 1;
                    self.storage.write(MIGRATION, &m)?;
                }
            }
            Operation::BeginRetirement | Operation::Retired => {
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
    use super::super::tests::{engine, restart};
    use super::*;
    fn source() -> BTreeMap<String, Option<String>> {
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
                e = restart(&e);
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
                    e = restart(&e);
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
                e = restart(&e);
                e.lifecycle(Operation::BeginRetirement).unwrap();
                e.storage.fail = Some((MIGRATION.into(), boundary));
                assert!(e.lifecycle(Operation::Retired).is_err());
                e = restart(&e);
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
            e = super::super::tests::restart(&e);
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
