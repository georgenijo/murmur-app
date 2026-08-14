use crate::telemetry::AppEvent;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const CAPTURE_HEALTH_SCHEMA_VERSION: u32 = 1;
const CAPTURE_HEALTH_FILE: &str = "capture-health-v1.json";
const MAX_CAPTURE_OBSERVATIONS: usize = 20;
const FALLBACK_CORRELATION_WINDOW_MS: i64 = 35_000;
const START_SUMMARY: &str = "audio initialization accepted";
const READY_SUMMARY: &str = "audio readiness accepted";
const FALLBACK_SUMMARY: &str =
    "capture backend failed before retained audio; trying bounded fallback";

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CaptureBackendV1 {
    Auhal,
    Cpal,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureHealthObservationV1 {
    pub(crate) startup_ms: u64,
    pub(crate) used_fallback: bool,
    pub(crate) fallback_from_backend: Option<CaptureBackendV1>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureHealthHistoryV1 {
    pub(crate) schema_version: u32,
    pub(crate) observations: Vec<CaptureHealthObservationV1>,
}

impl Default for CaptureHealthHistoryV1 {
    fn default() -> Self {
        Self {
            schema_version: CAPTURE_HEALTH_SCHEMA_VERSION,
            observations: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingFallback {
    at_ms: i64,
    from_backend: Option<CaptureBackendV1>,
}

#[derive(Default)]
struct CaptureHealthAccumulator {
    observations: VecDeque<CaptureHealthObservationV1>,
    pending_fallbacks: HashMap<u64, PendingFallback>,
}

impl CaptureHealthAccumulator {
    fn from_history(history: CaptureHealthHistoryV1) -> Self {
        let observations = history
            .observations
            .into_iter()
            .rev()
            .take(MAX_CAPTURE_OBSERVATIONS)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        Self {
            observations,
            pending_fallbacks: HashMap::new(),
        }
    }

    fn history(&self) -> CaptureHealthHistoryV1 {
        CaptureHealthHistoryV1 {
            schema_version: CAPTURE_HEALTH_SCHEMA_VERSION,
            observations: self.observations.iter().cloned().collect(),
        }
    }

    fn clear(&mut self) {
        self.observations.clear();
        self.pending_fallbacks.clear();
    }

    fn observe(&mut self, event: &AppEvent) -> bool {
        // Every shared-audio lifecycle mutation is dictation-scoped. Owner
        // counters are independent per owner kind, so keying a pending
        // fallback by the numeric ID alone would let a preview, query, or
        // diagnostic benchmark cycle collide with a later dictation.
        if string_field(event, "owner_kind") != Some("dictation") {
            return false;
        }
        let Some(owner) = numeric_field(event, "owner") else {
            return false;
        };

        if is_start(event) {
            self.pending_fallbacks.remove(&owner);
            return false;
        }
        if is_fallback(event) {
            if let Some(at_ms) = observed_at_ms(event) {
                self.pending_fallbacks.insert(
                    owner,
                    PendingFallback {
                        at_ms,
                        from_backend: backend_field(event, "from_backend"),
                    },
                );
            }
            return false;
        }
        if is_capture_failed(event) {
            self.pending_fallbacks.remove(&owner);
            return false;
        }
        if !is_ready(event) {
            return false;
        }

        let Some(startup_ms) = numeric_field(event, "startup_ms") else {
            return false;
        };
        let Some(ready_at_ms) = observed_at_ms(event) else {
            return false;
        };
        let fallback = self
            .pending_fallbacks
            .get(&owner)
            .filter(|pending| {
                ready_at_ms >= pending.at_ms
                    && ready_at_ms - pending.at_ms <= FALLBACK_CORRELATION_WINDOW_MS
            })
            .copied();

        self.pending_fallbacks.remove(&owner);
        if self.observations.len() >= MAX_CAPTURE_OBSERVATIONS {
            self.observations.pop_front();
        }
        self.observations.push_back(CaptureHealthObservationV1 {
            startup_ms,
            used_fallback: fallback.is_some(),
            fallback_from_backend: fallback.and_then(|pending| pending.from_backend),
        });
        true
    }
}

#[derive(Clone, Default)]
pub(crate) struct CaptureHealthDiagnostics {
    inner: Arc<Mutex<CaptureHealthInner>>,
}

#[derive(Default)]
struct CaptureHealthInner {
    accumulator: CaptureHealthAccumulator,
    file_path: Option<PathBuf>,
    writer_running: bool,
}

impl CaptureHealthDiagnostics {
    pub(crate) fn initialize(
        &self,
        diagnostics_root: PathBuf,
        event_log_paths: &[PathBuf],
    ) -> Result<(), String> {
        fs::create_dir_all(&diagnostics_root).map_err(|_| storage_error())?;
        let file_path = diagnostics_root.join(CAPTURE_HEALTH_FILE);
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.file_path = Some(file_path.clone());

        if file_path.exists() {
            inner.accumulator = CaptureHealthAccumulator::from_history(load_history(&file_path)?);
            return Ok(());
        }

        let mut migrated = CaptureHealthAccumulator::default();
        for path in event_log_paths {
            replay_event_log(path, &mut migrated);
        }
        let history = migrated.history();
        save_history(&file_path, &history)?;
        inner.accumulator = migrated;
        Ok(())
    }

    pub(crate) fn observe(&self, event: &AppEvent) {
        let should_persist = {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if inner.accumulator.observe(event)
                && inner.file_path.is_some()
                && !inner.writer_running
            {
                inner.writer_running = true;
                true
            } else {
                false
            }
        };
        if should_persist {
            #[cfg(test)]
            self.persist_current();
            #[cfg(not(test))]
            {
                let diagnostics = self.clone();
                if std::thread::Builder::new()
                    .name("capture-health-writer".to_string())
                    .spawn(move || diagnostics.persist_current())
                    .is_err()
                {
                    self.inner
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .writer_running = false;
                }
            }
        }
    }

    pub(crate) fn history(&self) -> CaptureHealthHistoryV1 {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .accumulator
            .history()
    }

    pub(crate) fn clear(&self) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.accumulator.clear();
        if let Some(path) = &inner.file_path {
            save_history(path, &inner.accumulator.history())?;
        }
        Ok(())
    }

    fn persist_current(&self) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(path) = &inner.file_path {
            let _ = save_history(path, &inner.accumulator.history());
        }
        inner.writer_running = false;
    }
}

#[tauri::command]
pub(crate) fn get_capture_health_history(
    state: tauri::State<'_, crate::State>,
) -> CaptureHealthHistoryV1 {
    state.capture_health.history()
}

fn event_code(event: &AppEvent) -> Option<&str> {
    string_field(event, "event_code")
}

fn string_field<'a>(event: &'a AppEvent, key: &str) -> Option<&'a str> {
    event.data.get(key)?.as_str()
}

fn numeric_field(event: &AppEvent, key: &str) -> Option<u64> {
    event.data.get(key)?.as_u64()
}

fn backend_field(event: &AppEvent, key: &str) -> Option<CaptureBackendV1> {
    match string_field(event, key) {
        Some("auhal") => Some(CaptureBackendV1::Auhal),
        Some("cpal") => Some(CaptureBackendV1::Cpal),
        _ => None,
    }
}

fn observed_at_ms(event: &AppEvent) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(&event.timestamp)
        .ok()
        .map(|timestamp| timestamp.timestamp_millis())
}

fn is_start(event: &AppEvent) -> bool {
    event_code(event) == Some("audio.capture_started") || event.summary == START_SUMMARY
}

fn is_ready(event: &AppEvent) -> bool {
    event_code(event) == Some("audio.capture_ready") || event.summary == READY_SUMMARY
}

fn is_fallback(event: &AppEvent) -> bool {
    event_code(event) == Some("audio.fallback_started") || event.summary == FALLBACK_SUMMARY
}

fn is_capture_failed(event: &AppEvent) -> bool {
    event_code(event) == Some("audio.capture_failed")
}

fn load_history(path: &Path) -> Result<CaptureHealthHistoryV1, String> {
    let bytes = fs::read(path).map_err(|_| storage_error())?;
    let history: CaptureHealthHistoryV1 = serde_json::from_slice(&bytes).map_err(|_| {
        quarantine_corrupt_file(path);
        storage_error()
    })?;
    if history.schema_version != CAPTURE_HEALTH_SCHEMA_VERSION {
        return Err("The capture-health diagnostics schema is unsupported.".to_string());
    }
    Ok(history)
}

fn save_history(path: &Path, history: &CaptureHealthHistoryV1) -> Result<(), String> {
    let bytes = serde_json::to_vec(history).map_err(|_| storage_error())?;
    let temp_path = path.with_extension("json.tmp");
    let write_result = (|| {
        let mut file = fs::File::create(&temp_path).map_err(|_| storage_error())?;
        file.write_all(&bytes).map_err(|_| storage_error())?;
        file.sync_all().map_err(|_| storage_error())?;
        fs::rename(&temp_path, path).map_err(|_| storage_error())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    write_result
}

fn replay_event_log(path: &Path, accumulator: &mut CaptureHealthAccumulator) {
    let Ok(contents) = fs::read_to_string(path) else {
        return;
    };
    for line in contents.lines() {
        if let Ok(event) = serde_json::from_str::<AppEvent>(line) {
            accumulator.observe(&event);
        }
    }
}

fn quarantine_corrupt_file(path: &Path) {
    let suffix = chrono::Utc::now().timestamp();
    let quarantine = path.with_extension(format!("json.corrupt-{suffix}"));
    let _ = fs::rename(path, quarantine);
}

fn storage_error() -> String {
    "The local capture-health diagnostics store is unavailable.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(at_ms: i64, summary: &str, data: serde_json::Value) -> AppEvent {
        AppEvent {
            timestamp: chrono::DateTime::from_timestamp_millis(at_ms)
                .unwrap()
                .to_rfc3339(),
            stream: "audio".to_string(),
            level: "info".to_string(),
            summary: summary.to_string(),
            data,
        }
    }

    fn start(at_ms: i64, owner: u64, kind: &str) -> AppEvent {
        event(
            at_ms,
            START_SUMMARY,
            serde_json::json!({
                "event_code": "audio.capture_started",
                "owner": owner,
                "owner_kind": kind,
            }),
        )
    }

    fn fallback(at_ms: i64, owner: u64, backend: &str, kind: &str) -> AppEvent {
        event(
            at_ms,
            FALLBACK_SUMMARY,
            serde_json::json!({
                "event_code": "audio.fallback_started",
                "owner": owner,
                "owner_kind": kind,
                "from_backend": backend,
                "device_label": "SENTINEL DEVICE",
                "device_uid": "SENTINEL UID",
                "transcript": "SENTINEL CONTENT",
            }),
        )
    }

    fn ready(at_ms: i64, owner: u64, startup_ms: u64, kind: &str) -> AppEvent {
        event(
            at_ms,
            READY_SUMMARY,
            serde_json::json!({
                "event_code": "audio.capture_ready",
                "owner": owner,
                "owner_kind": kind,
                "startup_ms": startup_ms,
            }),
        )
    }

    #[test]
    fn finalizes_only_successful_dictation_observations() {
        let mut accumulator = CaptureHealthAccumulator::default();
        accumulator.observe(&start(0, 1, "transform"));
        accumulator.observe(&fallback(100, 1, "auhal", "transform"));
        accumulator.observe(&ready(200, 1, 100, "transform"));
        accumulator.observe(&start(300, 2, "dictation"));
        accumulator.observe(&ready(500, 2, 200, "dictation"));

        assert_eq!(
            accumulator.history().observations,
            vec![CaptureHealthObservationV1 {
                startup_ms: 200,
                used_fallback: false,
                fallback_from_backend: None,
            }]
        );
    }

    #[test]
    fn non_dictation_owner_collision_and_stale_window_cannot_invent_fallback() {
        let mut accumulator = CaptureHealthAccumulator::default();
        accumulator.observe(&start(0, 7, "microphone_benchmark"));
        accumulator.observe(&fallback(500, 7, "auhal", "microphone_benchmark"));
        accumulator.observe(&ready(1_000, 7, 500, "microphone_benchmark"));
        assert!(accumulator.pending_fallbacks.is_empty());

        accumulator.observe(&start(1_500, 7, "dictation"));
        accumulator.observe(&ready(2_000, 7, 1_000, "dictation"));
        accumulator.observe(&fallback(3_000, 8, "auhal", "dictation"));
        accumulator.observe(&ready(38_001, 8, 1_000, "dictation"));

        assert_eq!(accumulator.history().observations.len(), 2);
        assert!(accumulator
            .history()
            .observations
            .iter()
            .all(|observation| !observation.used_fallback));
    }

    #[test]
    fn unknown_backend_still_records_recovered_fallback_without_its_label() {
        let mut accumulator = CaptureHealthAccumulator::default();
        accumulator.observe(&fallback(0, 9, "private-device-label", "dictation"));
        accumulator.observe(&ready(1_000, 9, 1_000, "dictation"));

        let observation = &accumulator.history().observations[0];
        assert!(observation.used_fallback);
        assert_eq!(observation.fallback_from_backend, None);
    }

    #[test]
    fn keeps_only_the_newest_bounded_observations() {
        let mut accumulator = CaptureHealthAccumulator::default();
        for owner in 0..(MAX_CAPTURE_OBSERVATIONS as u64 + 5) {
            accumulator.observe(&ready(owner as i64, owner, owner, "dictation"));
        }

        let history = accumulator.history();
        assert_eq!(history.observations.len(), MAX_CAPTURE_OBSERVATIONS);
        assert_eq!(history.observations.first().unwrap().startup_ms, 5);
    }

    #[test]
    fn migration_persists_only_content_free_finalized_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let event_log = temp.path().join("events.jsonl");
        let events = [
            start(0, 9, "dictation"),
            fallback(1_000, 9, "auhal", "dictation"),
            ready(2_000, 9, 2_000, "dictation"),
        ];
        let jsonl = events
            .iter()
            .map(|event| serde_json::to_string(event).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&event_log, jsonl).unwrap();

        let diagnostics = CaptureHealthDiagnostics::default();
        let root = temp.path().join("diagnostics");
        diagnostics.initialize(root.clone(), &[event_log]).unwrap();

        let history = diagnostics.history();
        assert_eq!(history.observations.len(), 1);
        assert_eq!(
            history.observations[0].fallback_from_backend,
            Some(CaptureBackendV1::Auhal)
        );
        assert!(history.observations[0].used_fallback);
        let persisted = fs::read_to_string(root.join(CAPTURE_HEALTH_FILE)).unwrap();
        assert!(!persisted.contains("SENTINEL"));
        assert!(!persisted.contains("owner"));
        assert!(!persisted.contains("timestamp"));
    }

    #[test]
    fn persisted_history_survives_restart_and_clear() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("diagnostics");
        let first = CaptureHealthDiagnostics::default();
        first.initialize(root.clone(), &[]).unwrap();
        first.observe(&ready(2_000, 1, 240, "dictation"));

        let restarted = CaptureHealthDiagnostics::default();
        restarted.initialize(root.clone(), &[]).unwrap();
        assert_eq!(restarted.history().observations.len(), 1);

        restarted.clear().unwrap();
        let cleared = CaptureHealthDiagnostics::default();
        cleared.initialize(root, &[]).unwrap();
        assert!(cleared.history().observations.is_empty());
    }
}
