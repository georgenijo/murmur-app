mod repository;
mod types;

pub use types::*;

use repository::PerformanceRepository;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::Emitter;

#[derive(Clone, Default)]
pub(crate) struct PerformanceMetrics {
    inner: Arc<Mutex<PerformanceMetricsInner>>,
}

#[derive(Default)]
struct PerformanceMetricsInner {
    repository: Option<PerformanceRepository>,
    app_handle: Option<tauri::AppHandle>,
    initialization_error: Option<String>,
}

impl PerformanceMetrics {
    pub(crate) fn initialize(
        &self,
        root: PathBuf,
        app_handle: Option<tauri::AppHandle>,
    ) -> Result<(), String> {
        let result = PerformanceRepository::initialize(root);
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.app_handle = app_handle;
        match result {
            Ok(repository) => {
                inner.repository = Some(repository);
                inner.initialization_error = None;
                Ok(())
            }
            Err(error) => {
                inner.repository = None;
                inner.initialization_error = Some(error.clone());
                Err(error)
            }
        }
    }

    fn repository(&self) -> Result<PerformanceRepository, String> {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.repository.clone().ok_or_else(|| {
            inner
                .initialization_error
                .clone()
                .unwrap_or_else(|| "The local diagnostics store is unavailable.".to_string())
        })
    }

    pub(crate) fn begin(
        &self,
        kind: PerformanceRunKindV1,
        correlation: RunCorrelationV1,
        runtimes: Vec<RuntimeIdentityV1>,
        input: ContentFreeInputSummaryV1,
    ) -> Result<ActiveRunV1, String> {
        self.repository()?.begin(kind, correlation, runtimes, input)
    }

    pub(crate) fn begin_dictation(
        &self,
        recording_id: u64,
        runtimes: Vec<RuntimeIdentityV1>,
    ) -> Result<ActiveRunV1, String> {
        self.begin(
            PerformanceRunKindV1::Dictation,
            RunCorrelationV1::Dictation { recording_id },
            runtimes,
            ContentFreeInputSummaryV1::default(),
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

    pub(crate) fn update_active(
        &self,
        correlation: &RunCorrelationV1,
        update: impl FnOnce(&mut ActiveRunV1),
    ) -> Result<bool, String> {
        self.repository()?.update_active(correlation, update)
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
        let run = self
            .repository()?
            .complete(correlation, outcome, stages, input, runtimes)?;
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
        self.repository()?.insert_resource_sample(sample)?;
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

    pub(crate) fn list(&self, limit: u32) -> Result<PerformanceRunListV1, String> {
        Ok(PerformanceRunListV1 {
            schema_version: PERFORMANCE_RUN_SCHEMA_VERSION,
            runs: self.repository()?.list(limit)?,
        })
    }

    pub(crate) fn get(&self, run_id: &str) -> Result<Option<PerformanceRunV1>, String> {
        self.repository()?.get(run_id)
    }

    pub(crate) fn resource_window(&self) -> Result<Vec<ResourceSampleV1>, String> {
        self.repository()?.resource_window()
    }

    pub(crate) fn clear(&self) -> Result<(), String> {
        self.repository()?.clear()?;
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
        assert_eq!(decoded.stages.len(), 25);
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
}
