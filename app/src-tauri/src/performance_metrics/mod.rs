mod repository;
mod types;

pub use types::*;

pub(crate) use repository::PerformanceStoreError;
use repository::{InitializationOutcome, PerformanceRepository};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::Emitter;

#[derive(Clone)]
pub(crate) struct PerformanceMetrics {
    inner: Arc<Mutex<PerformanceMetricsInner>>,
    operation_lock: Arc<Mutex<()>>,
}

impl Default for PerformanceMetrics {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(PerformanceMetricsInner::default())),
            operation_lock: Arc::new(Mutex::new(())),
        }
    }
}

#[derive(Default)]
struct PerformanceMetricsInner {
    repository: Option<PerformanceRepository>,
    root: Option<PathBuf>,
    app_handle: Option<tauri::AppHandle>,
    health: PerformanceStoreHealthV1,
}

impl PerformanceMetrics {
    pub(crate) fn initialize(
        &self,
        root: PathBuf,
        app_handle: Option<tauri::AppHandle>,
    ) -> Result<(), String> {
        let _operation_guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = PerformanceRepository::initialize(root.clone());
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.app_handle = app_handle;
        inner.root = Some(root);
        match result {
            Ok((repository, outcome)) => {
                inner.repository = Some(repository);
                inner.health.status = PerformanceStoreStatusV1::Available;
                inner.health.recommended_action = PerformanceStoreRecommendedActionV1::None;
                if let InitializationOutcome::Reinitialized { at_ms } = outcome {
                    inner.health.last_recovery = Some(PerformanceStoreRecoveryV1 {
                        action: PerformanceStoreRecoveryActionV1::QuarantinedAndReinitialized,
                        at_ms,
                    });
                }
                Ok(())
            }
            Err(error) => {
                inner.repository = None;
                apply_failure(&mut inner, &error, None, false);
                Err(error.to_string())
            }
        }
    }

    fn repository(
        &self,
        operation: PerformanceStoreOperationV1,
    ) -> Result<PerformanceRepository, PerformanceStoreError> {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner
            .repository
            .clone()
            .ok_or_else(|| PerformanceStoreError {
                operation,
                class: inner
                    .health
                    .last_failure
                    .as_ref()
                    .map(|failure| failure.error_class)
                    .unwrap_or(PerformanceStoreErrorClassV1::Unavailable),
                attempts: 1,
            })
    }

    pub(crate) fn health(&self) -> PerformanceStoreHealthV1 {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .health
            .clone()
    }

    pub(crate) fn recover(
        &self,
        allow_reinitialize: bool,
    ) -> Result<PerformanceStoreHealthV1, String> {
        let _operation_guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let root = {
            let inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if inner.health.status == PerformanceStoreStatusV1::Available
                && inner.repository.is_some()
            {
                return Ok(inner.health.clone());
            }
            inner.root.clone().ok_or_else(|| {
                "The local diagnostics store has not been initialized.".to_string()
            })?
        };
        let result = if allow_reinitialize {
            PerformanceRepository::reinitialize(root)
        } else {
            PerformanceRepository::reopen(root)
        };
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match result {
            Ok((repository, outcome)) => {
                inner.repository = Some(repository);
                inner.health.status = PerformanceStoreStatusV1::Available;
                inner.health.recommended_action = PerformanceStoreRecommendedActionV1::None;
                if let InitializationOutcome::Reinitialized { at_ms } = outcome {
                    inner.health.last_recovery = Some(PerformanceStoreRecoveryV1 {
                        action: PerformanceStoreRecoveryActionV1::QuarantinedAndReinitialized,
                        at_ms,
                    });
                }
                Ok(inner.health.clone())
            }
            Err(error) => {
                apply_failure(&mut inner, &error, None, false);
                if allow_reinitialize {
                    Err(error.to_string())
                } else {
                    // A failed safe probe is itself a valid health result. The
                    // caller can now present the freshly classified recovery
                    // action and require a separate confirmed reinitialize.
                    Ok(inner.health.clone())
                }
            }
        }
    }

    pub(crate) fn begin(
        &self,
        kind: PerformanceRunKindV1,
        correlation: RunCorrelationV1,
        runtimes: Vec<RuntimeIdentityV1>,
        input: ContentFreeInputSummaryV1,
    ) -> Result<ActiveRunV1, String> {
        let started_at_ms = chrono::Utc::now().timestamp_millis();
        self.begin_diagnosed(kind, correlation, runtimes, input, None, started_at_ms)
            .map_err(|error| error.to_string())
    }

    fn begin_diagnosed(
        &self,
        kind: PerformanceRunKindV1,
        correlation: RunCorrelationV1,
        runtimes: Vec<RuntimeIdentityV1>,
        input: ContentFreeInputSummaryV1,
        recording_id: Option<u64>,
        started_at_ms: i64,
    ) -> Result<ActiveRunV1, PerformanceStoreError> {
        let _operation_guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = self
            .repository(PerformanceStoreOperationV1::Begin)
            .and_then(|repository| {
                repository.begin_at(kind, correlation, runtimes, input, started_at_ms)
            });
        if let Err(error) = &result {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // The health banner's skipped-run counter is specifically the
            // dictation reliability signal requested by #536. Other run kinds
            // still update last_failure, but must not make the UI claim that a
            // dictation continued after their diagnostics row was skipped.
            apply_failure(&mut inner, error, recording_id, recording_id.is_some());
        }
        result
    }

    fn observe<T>(&self, result: Result<T, PerformanceStoreError>) -> Result<T, String> {
        result.map_err(|error| {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            apply_failure(&mut inner, &error, None, false);
            error.to_string()
        })
    }

    #[cfg(test)]
    pub(crate) fn begin_dictation(
        &self,
        recording_id: u64,
        runtimes: Vec<RuntimeIdentityV1>,
    ) -> Result<ActiveRunV1, String> {
        self.begin_dictation_diagnosed(recording_id, runtimes)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn begin_dictation_diagnosed(
        &self,
        recording_id: u64,
        runtimes: Vec<RuntimeIdentityV1>,
    ) -> Result<ActiveRunV1, PerformanceStoreError> {
        self.begin_dictation_diagnosed_at(
            recording_id,
            runtimes,
            chrono::Utc::now().timestamp_millis(),
        )
    }

    pub(crate) fn begin_dictation_diagnosed_at(
        &self,
        recording_id: u64,
        runtimes: Vec<RuntimeIdentityV1>,
        started_at_ms: i64,
    ) -> Result<ActiveRunV1, PerformanceStoreError> {
        self.begin_diagnosed(
            PerformanceRunKindV1::Dictation,
            RunCorrelationV1::Dictation { recording_id },
            runtimes,
            ContentFreeInputSummaryV1::default(),
            Some(recording_id),
            started_at_ms,
        )
    }

    pub(crate) fn begin_file_transcription(
        &self,
        file_run_id: u64,
        runtimes: Vec<RuntimeIdentityV1>,
    ) -> Result<ActiveRunV1, String> {
        self.begin(
            PerformanceRunKindV1::FileTranscription,
            RunCorrelationV1::FileTranscription { file_run_id },
            runtimes,
            ContentFreeInputSummaryV1::default(),
        )
    }

    pub(crate) fn begin_selected_text_transform(
        &self,
        transform_pass_id: u64,
        runtimes: Vec<RuntimeIdentityV1>,
    ) -> Result<ActiveRunV1, String> {
        self.begin(
            PerformanceRunKindV1::SelectedTextTransform,
            RunCorrelationV1::SelectedTextTransform { transform_pass_id },
            runtimes,
            ContentFreeInputSummaryV1::default(),
        )
    }

    pub(crate) fn begin_voice_query(&self, query_pass_id: u64) -> Result<ActiveRunV1, String> {
        self.begin(
            PerformanceRunKindV1::VoiceQuery,
            RunCorrelationV1::VoiceQuery { query_pass_id },
            Vec::new(),
            ContentFreeInputSummaryV1::default(),
        )
    }

    pub(crate) fn set_query_process(
        &self,
        query_pass_id: u64,
        query_process: QueryProcessSummaryV1,
    ) -> Result<bool, String> {
        self.update_active(&RunCorrelationV1::VoiceQuery { query_pass_id }, |active| {
            active.query_process = Some(query_process)
        })
    }

    pub(crate) fn update_active(
        &self,
        correlation: &RunCorrelationV1,
        update: impl FnOnce(&mut ActiveRunV1) + Clone,
    ) -> Result<bool, String> {
        let _operation_guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = self
            .repository(PerformanceStoreOperationV1::Update)
            .and_then(|repository| repository.update_active(correlation, update));
        self.observe(result)
    }

    pub(crate) fn record_stage(
        &self,
        correlation: &RunCorrelationV1,
        timing: StageTimingV1,
    ) -> Result<bool, String> {
        self.update_active(correlation, |active| {
            active.current_stage = timing.stage;
            if let Some(existing) = active
                .stages
                .iter_mut()
                .find(|stage| stage.stage == timing.stage)
            {
                *existing = timing;
            } else {
                active.stages.push(timing);
            }
        })
    }

    pub(crate) fn set_current_stage(
        &self,
        correlation: &RunCorrelationV1,
        stage: PerformanceStageV1,
    ) -> Result<bool, String> {
        self.update_active(correlation, |active| active.current_stage = stage)
    }

    pub(crate) fn complete(
        &self,
        correlation: &RunCorrelationV1,
        outcome: RunOutcomeV1,
        stages: Vec<StageTimingV1>,
        input: Option<ContentFreeInputSummaryV1>,
        runtimes: Option<Vec<RuntimeIdentityV1>>,
    ) -> Result<Option<PerformanceRunV1>, String> {
        let run = {
            let _operation_guard = self
                .operation_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let result = self
                .repository(PerformanceStoreOperationV1::Complete)
                .and_then(|repository| {
                    repository.complete(correlation, outcome, stages, input, runtimes)
                });
            self.observe(result)?
        };
        if let Some(run) = &run {
            let app_handle = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .app_handle
                .clone();
            if let Some(app_handle) = app_handle {
                let _ = app_handle.emit("performance-run-completed", run);
            }
        }
        Ok(run)
    }

    pub(crate) fn guard(
        &self,
        correlation: RunCorrelationV1,
        stage: PerformanceStageV1,
    ) -> PerformanceRunGuard {
        let _ = self.set_current_stage(&correlation, stage);
        PerformanceRunGuard {
            metrics: self.clone(),
            correlation,
            current_stage: stage,
            finished: false,
        }
    }

    pub(crate) fn insert_resource_sample(&self, sample: &ResourceSampleV1) -> Result<(), String> {
        {
            let _operation_guard = self
                .operation_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let result = self
                .repository(PerformanceStoreOperationV1::Write)
                .and_then(|repository| repository.insert_resource_sample(sample));
            self.observe(result)?;
        }
        let app_handle = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .app_handle
            .clone();
        if let Some(app_handle) = app_handle {
            let _ = app_handle.emit("performance-resource-sample", sample);
        }
        Ok(())
    }

    pub(crate) fn append_transform_follow_up(
        &self,
        transform_pass_id: u64,
        follow_up: TransformFollowUpV1,
    ) -> Result<Option<PerformanceRunV1>, String> {
        let _operation_guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = self
            .repository(PerformanceStoreOperationV1::Write)
            .and_then(|repository| {
                repository.append_transform_follow_up(
                    &RunCorrelationV1::SelectedTextTransform { transform_pass_id },
                    follow_up,
                )
            });
        self.observe(result)
    }

    pub(crate) fn list(&self, limit: u32) -> Result<PerformanceRunListV1, String> {
        let _operation_guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = self
            .repository(PerformanceStoreOperationV1::Read)
            .and_then(|repository| repository.list(limit));
        Ok(PerformanceRunListV1 {
            schema_version: PERFORMANCE_RUN_SCHEMA_VERSION,
            runs: self.observe(result)?,
        })
    }

    pub(crate) fn get(&self, run_id: &str) -> Result<Option<PerformanceRunV1>, String> {
        if run_id.len() != 32 || !run_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("The performance run ID is invalid.".to_string());
        }
        let _operation_guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = self
            .repository(PerformanceStoreOperationV1::Read)
            .and_then(|repository| repository.get(run_id));
        self.observe(result)
    }

    pub(crate) fn resource_window(&self) -> Result<Vec<ResourceSampleV1>, String> {
        let _operation_guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = self
            .repository(PerformanceStoreOperationV1::Read)
            .and_then(|repository| repository.resource_window());
        self.observe(result)
    }

    pub(crate) fn clear(&self) -> Result<(), String> {
        {
            let _operation_guard = self
                .operation_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let result = self
                .repository(PerformanceStoreOperationV1::Clear)
                .and_then(|repository| repository.clear());
            self.observe(result)?;
        }
        let app_handle = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .app_handle
            .clone();
        if let Some(app_handle) = app_handle {
            let _ = app_handle.emit("performance-diagnostics-cleared", ());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn counts(&self) -> Result<(u64, u64, u64), String> {
        let _operation_guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = self
            .repository(PerformanceStoreOperationV1::Read)
            .and_then(|repository| repository.counts());
        self.observe(result)
    }
}

fn recommended_action(class: PerformanceStoreErrorClassV1) -> PerformanceStoreRecommendedActionV1 {
    match class {
        PerformanceStoreErrorClassV1::BusyLocked
        | PerformanceStoreErrorClassV1::Io
        | PerformanceStoreErrorClassV1::Unavailable => PerformanceStoreRecommendedActionV1::Retry,
        PerformanceStoreErrorClassV1::StorageFull => PerformanceStoreRecommendedActionV1::FreeDisk,
        PerformanceStoreErrorClassV1::ReadOnly => {
            PerformanceStoreRecommendedActionV1::CheckPermissions
        }
        PerformanceStoreErrorClassV1::CorruptIntegrity
        | PerformanceStoreErrorClassV1::InvalidRecord => {
            PerformanceStoreRecommendedActionV1::ReinitializeStore
        }
        PerformanceStoreErrorClassV1::SchemaMigration => {
            PerformanceStoreRecommendedActionV1::RestartApp
        }
    }
}

fn apply_failure(
    inner: &mut PerformanceMetricsInner,
    error: &PerformanceStoreError,
    recording_id: Option<u64>,
    skipped_run: bool,
) {
    if skipped_run {
        inner.health.skipped_run_count = inner.health.skipped_run_count.saturating_add(1);
    }
    let retry_exhausted = error.retry_exhausted();
    inner.health.last_failure = Some(PerformanceStoreFailureV1 {
        operation: error.operation,
        error_class: error.class,
        attempt_count: error.attempts,
        retry_exhausted,
        at_ms: chrono::Utc::now().timestamp_millis(),
        recording_id,
    });
    inner.health.recommended_action = recommended_action(error.class);
    if error.class != PerformanceStoreErrorClassV1::BusyLocked {
        inner.health.status = PerformanceStoreStatusV1::Unavailable;
        inner.repository = None;
    } else if inner.repository.is_some() {
        inner.health.status = PerformanceStoreStatusV1::Available;
    }
}

pub(crate) struct PerformanceRunGuard {
    metrics: PerformanceMetrics,
    correlation: RunCorrelationV1,
    current_stage: PerformanceStageV1,
    finished: bool,
}

impl PerformanceRunGuard {
    pub(crate) fn enter(&mut self, stage: PerformanceStageV1) {
        self.current_stage = stage;
        let _ = self.metrics.set_current_stage(&self.correlation, stage);
    }

    pub(crate) fn record(&mut self, timing: StageTimingV1) {
        self.current_stage = timing.stage;
        let _ = self.metrics.record_stage(&self.correlation, timing);
    }

    /// Leave a deliberately long-lived run active across command boundaries.
    /// The next command creates its own panic guard for the same correlation.
    pub(crate) fn defer(mut self) {
        self.finished = true;
    }

    pub(crate) fn finish(
        mut self,
        outcome: RunOutcomeV1,
        stages: Vec<StageTimingV1>,
        input: Option<ContentFreeInputSummaryV1>,
        runtimes: Option<Vec<RuntimeIdentityV1>>,
    ) -> Result<Option<PerformanceRunV1>, String> {
        let run = self
            .metrics
            .complete(&self.correlation, outcome, stages, input, runtimes)?;
        self.finished = true;
        Ok(run)
    }
}

impl Drop for PerformanceRunGuard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let _ = self.metrics.complete(
            &self.correlation,
            RunOutcomeV1::Failed {
                stage: self.current_stage,
                error_code: StableRunErrorV1::InternalEarlyExit,
            },
            Vec::new(),
            None,
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    fn metrics() -> (tempfile::TempDir, PerformanceMetrics) {
        let temp = tempfile::tempdir().unwrap();
        let metrics = PerformanceMetrics::default();
        metrics
            .initialize(temp.path().join("diagnostics"), None)
            .unwrap();
        (temp, metrics)
    }

    #[test]
    fn guard_closes_early_exit_once() {
        let (_temp, metrics) = metrics();
        metrics.begin_dictation(1, Vec::new()).unwrap();
        {
            let mut guard = metrics.guard(
                RunCorrelationV1::Dictation { recording_id: 1 },
                PerformanceStageV1::Vad,
            );
            guard.enter(PerformanceStageV1::InferenceDecode);
        }
        let runs = metrics.list(10).unwrap().runs;
        assert_eq!(runs.len(), 1);
        assert!(matches!(
            runs[0].outcome,
            RunOutcomeV1::Failed {
                stage: PerformanceStageV1::InferenceDecode,
                error_code: StableRunErrorV1::InternalEarlyExit
            }
        ));
    }

    #[test]
    fn guard_is_panic_safe_under_unwind() {
        let (_temp, metrics) = metrics();
        metrics.begin_dictation(2, Vec::new()).unwrap();
        let clone = metrics.clone();
        let result = std::panic::catch_unwind(move || {
            let mut guard = clone.guard(
                RunCorrelationV1::Dictation { recording_id: 2 },
                PerformanceStageV1::Vad,
            );
            guard.enter(PerformanceStageV1::ModelLoad);
            panic!("test unwind");
        });
        assert!(result.is_err());
        let runs = metrics.list(10).unwrap().runs;
        assert_eq!(runs.len(), 1);
        assert!(matches!(
            runs[0].outcome,
            RunOutcomeV1::Failed {
                stage: PerformanceStageV1::ModelLoad,
                ..
            }
        ));
    }

    #[test]
    fn deferred_transform_run_is_closed_by_the_next_command_guard() {
        let (_temp, metrics) = metrics();
        let correlation = RunCorrelationV1::SelectedTextTransform {
            transform_pass_id: 8,
        };
        metrics
            .begin_selected_text_transform(8, Vec::new())
            .unwrap();
        metrics
            .guard(correlation.clone(), PerformanceStageV1::SelectedTextCapture)
            .defer();
        assert!(metrics.list(10).unwrap().runs.is_empty());

        let mut next_command = metrics.guard(correlation, PerformanceStageV1::InstructionCapture);
        next_command.enter(PerformanceStageV1::InstructionAsr);
        drop(next_command);
        let runs = metrics.list(10).unwrap().runs;
        assert_eq!(runs.len(), 1);
        assert!(matches!(
            runs[0].outcome,
            RunOutcomeV1::Failed {
                stage: PerformanceStageV1::InstructionAsr,
                error_code: StableRunErrorV1::InternalEarlyExit,
            }
        ));
    }

    #[test]
    fn measured_zero_is_not_unavailable_or_not_applicable() {
        let measured = MeasurementV1::measured(0_u64);
        assert!(matches!(measured, MeasurementV1::Measured { value: 0 }));
        assert_ne!(measured, MeasurementV1::NotApplicable);
        assert_ne!(
            measured,
            MeasurementV1::Unavailable {
                reason: UnavailableReasonV1::NoSamples
            }
        );
    }

    #[test]
    fn v1_tagged_values_have_stable_json_and_round_trip() {
        assert_eq!(
            serde_json::to_value(MeasurementV1::measured(0_u64)).unwrap(),
            serde_json::json!({ "status": "measured", "value": 0 })
        );
        assert_eq!(
            serde_json::to_value(RunOutcomeV1::Failed {
                stage: PerformanceStageV1::FileDecode,
                error_code: StableRunErrorV1::DecodeFailed,
            })
            .unwrap(),
            serde_json::json!({
                "status": "failed",
                "stage": "fileDecode",
                "errorCode": "decodeFailed"
            })
        );
        assert_eq!(
            serde_json::to_value(RunCorrelationV1::Dictation { recording_id: 9 }).unwrap(),
            serde_json::json!({ "kind": "dictation", "recordingId": 9 })
        );

        let (_temp, metrics) = metrics();
        metrics.begin_dictation(6, Vec::new()).unwrap();
        let run = metrics
            .complete(
                &RunCorrelationV1::Dictation { recording_id: 6 },
                RunOutcomeV1::Success,
                Vec::new(),
                Some(ContentFreeInputSummaryV1::audio(250)),
                None,
            )
            .unwrap()
            .unwrap();
        let payload = serde_json::to_string(&run).unwrap();
        let decoded: PerformanceRunV1 = serde_json::from_str(&payload).unwrap();
        assert_eq!(decoded, run);
        assert_eq!(decoded.schema_version, PERFORMANCE_RUN_SCHEMA_VERSION);
        assert_eq!(decoded.stages.len(), 27);
    }

    #[test]
    fn serialized_run_has_no_free_form_content_fields() {
        let (_temp, metrics) = metrics();
        metrics.begin_dictation(7, Vec::new()).unwrap();
        metrics
            .complete(
                &RunCorrelationV1::Dictation { recording_id: 7 },
                RunOutcomeV1::Failed {
                    stage: PerformanceStageV1::InferenceDecode,
                    error_code: StableRunErrorV1::InferenceFailed,
                },
                Vec::new(),
                Some(ContentFreeInputSummaryV1::audio(123)),
                None,
            )
            .unwrap();
        let json = serde_json::to_string(&metrics.list(10).unwrap()).unwrap();
        for forbidden in [
            "SECRET transcript",
            "/Users/private/file.wav",
            "com.private.app",
            "private window title",
            "private profile",
            "clipboard secret",
            "native stderr secret",
        ] {
            assert!(!json.contains(forbidden));
        }
        for forbidden_key in [
            "\"text\"",
            "\"path\"",
            "\"bundleId\"",
            "\"windowTitle\"",
            "\"profileName\"",
            "\"clipboard\"",
            "\"stderr\"",
            "\"errorMessage\"",
        ] {
            assert!(!json.contains(forbidden_key));
        }
    }

    #[test]
    fn busy_begin_is_a_skipped_run_without_disabling_the_store() {
        let (_temp, metrics) = metrics();
        let repository = metrics
            .repository(PerformanceStoreOperationV1::Read)
            .unwrap();
        let blocker = repository.test_connection().unwrap();
        blocker.execute_batch("BEGIN IMMEDIATE;").unwrap();

        let file_error = metrics
            .begin_file_transcription(66, Vec::new())
            .unwrap_err();
        assert!(file_error.contains("busy"));
        assert_eq!(metrics.health().skipped_run_count, 0);

        let error = metrics
            .begin_dictation_diagnosed(77, Vec::new())
            .unwrap_err();
        assert_eq!(error.operation, PerformanceStoreOperationV1::Begin);
        assert_eq!(error.class, PerformanceStoreErrorClassV1::BusyLocked);
        assert_eq!(error.attempts, 3);
        let health = metrics.health();
        assert_eq!(health.status, PerformanceStoreStatusV1::Available);
        assert_eq!(health.skipped_run_count, 1);
        let failure = health.last_failure.unwrap();
        assert_eq!(failure.recording_id, Some(77));
        assert_eq!(failure.attempt_count, 3);
        assert!(failure.retry_exhausted);
        assert!(failure.at_ms >= 0);
        assert_eq!(
            health.recommended_action,
            PerformanceStoreRecommendedActionV1::Retry
        );

        blocker.execute_batch("ROLLBACK;").unwrap();
        metrics.begin_dictation_diagnosed(78, Vec::new()).unwrap();
        assert_eq!(metrics.health().skipped_run_count, 1);
    }

    #[test]
    fn invalid_run_id_does_not_change_store_health() {
        let (_temp, metrics) = metrics();
        let before = metrics.health();
        assert!(metrics.get("not-a-run-id").is_err());
        assert_eq!(metrics.health(), before);
    }

    #[test]
    fn proven_invalid_record_becomes_unavailable_and_recovers_with_evidence() {
        let (temp, metrics) = metrics();
        let repository = metrics
            .repository(PerformanceStoreOperationV1::Read)
            .unwrap();
        repository
            .test_connection()
            .unwrap()
            .execute(
                "INSERT INTO completed_runs(
                    run_id, record_version, kind, correlation_kind, correlation_id,
                    started_at_ms, finished_at_ms, outcome_code, payload_json
                 ) VALUES ('bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', 1, 'dictation',
                           'dictation', 1, 1, 2, 'success', 'not json')",
                [],
            )
            .unwrap();

        assert!(metrics.list(10).is_err());
        let unavailable = metrics.health();
        assert_eq!(unavailable.status, PerformanceStoreStatusV1::Unavailable);
        assert_eq!(
            unavailable.last_failure.unwrap().error_class,
            PerformanceStoreErrorClassV1::InvalidRecord
        );
        assert_eq!(
            unavailable.recommended_action,
            PerformanceStoreRecommendedActionV1::ReinitializeStore
        );

        let still_unavailable = metrics.recover(false).unwrap();
        assert_eq!(
            still_unavailable.status,
            PerformanceStoreStatusV1::Unavailable
        );
        assert_eq!(
            still_unavailable.last_failure.unwrap().error_class,
            PerformanceStoreErrorClassV1::InvalidRecord
        );
        assert!(!temp.path().join("diagnostics/quarantine").exists());

        let recovered = metrics.recover(true).unwrap();
        assert_eq!(recovered.status, PerformanceStoreStatusV1::Available);
        assert_eq!(
            recovered.last_recovery.unwrap().action,
            PerformanceStoreRecoveryActionV1::QuarantinedAndReinitialized
        );
        let quarantine = temp.path().join("diagnostics/quarantine");
        assert!(quarantine.exists());
        assert!(!std::fs::read_dir(quarantine)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn recovery_is_a_no_op_while_store_is_available() {
        let (temp, metrics) = metrics();
        let before = metrics.health();
        assert_eq!(metrics.recover(false).unwrap(), before);
        assert_eq!(metrics.recover(true).unwrap(), before);
        assert!(!temp.path().join("diagnostics/quarantine").exists());
    }

    #[test]
    fn recovery_waits_for_an_in_flight_failure_and_cannot_be_cleared_by_it() {
        let (temp, metrics) = metrics();
        let (operation_started_tx, operation_started_rx) = mpsc::channel();
        let (release_operation_tx, release_operation_rx) = mpsc::channel();
        let ordinary = {
            let metrics = metrics.clone();
            thread::spawn(move || {
                let _operation_guard = metrics
                    .operation_lock
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                operation_started_tx.send(()).unwrap();
                release_operation_rx.recv().unwrap();
                let mut inner = metrics
                    .inner
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                apply_failure(
                    &mut inner,
                    &PerformanceStoreError {
                        operation: PerformanceStoreOperationV1::Read,
                        class: PerformanceStoreErrorClassV1::InvalidRecord,
                        attempts: 1,
                    },
                    None,
                    false,
                );
            })
        };
        operation_started_rx.recv().unwrap();

        let (recovery_finished_tx, recovery_finished_rx) = mpsc::channel();
        let recovery = {
            let metrics = metrics.clone();
            thread::spawn(move || {
                let result = metrics.recover(true);
                recovery_finished_tx.send(result).unwrap();
            })
        };
        assert!(recovery_finished_rx
            .recv_timeout(Duration::from_millis(25))
            .is_err());
        release_operation_tx.send(()).unwrap();
        ordinary.join().unwrap();
        let recovered = recovery_finished_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        recovery.join().unwrap();

        assert_eq!(recovered.status, PerformanceStoreStatusV1::Available);
        assert_eq!(metrics.health().status, PerformanceStoreStatusV1::Available);
        assert!(!temp.path().join("diagnostics/quarantine").exists());
    }
}
