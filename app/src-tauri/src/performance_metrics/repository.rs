use super::types::*;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::fs;
use std::path::PathBuf;

const DB_FILE: &str = "performance.sqlite3";
const LATEST_DB_SCHEMA_VERSION: u32 = 1;
const MAX_COMPLETED_RUNS: usize = 200;
const MAX_RESOURCE_SAMPLES: usize = 600;
const MAX_TRANSFORM_FOLLOW_UPS: usize = 8;

#[derive(Clone)]
pub(crate) struct PerformanceRepository {
    db_path: PathBuf,
}

impl PerformanceRepository {
    pub(crate) fn initialize(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&root).map_err(|_| storage_error())?;
        let repository = Self {
            db_path: root.join(DB_FILE),
        };
        let mut connection = repository.open()?;
        migrate(&mut connection)?;
        quick_check(&connection)?;
        repository.recover_stale_runs(&mut connection)?;
        Ok(repository)
    }

    fn open(&self) -> Result<Connection, String> {
        let connection = Connection::open(&self.db_path).map_err(db_error)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=NORMAL;
                 PRAGMA foreign_keys=ON;
                 PRAGMA busy_timeout=2000;",
            )
            .map_err(db_error)?;
        Ok(connection)
    }

    pub(crate) fn begin(
        &self,
        kind: PerformanceRunKindV1,
        correlation: RunCorrelationV1,
        runtimes: Vec<RuntimeIdentityV1>,
        input: ContentFreeInputSummaryV1,
    ) -> Result<ActiveRunV1, String> {
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(db_error)?;
        let run_id: String = transaction
            .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))
            .map_err(db_error)?;
        let started_at_ms = now_ms();
        let clear_epoch = clear_epoch_tx(&transaction)?;
        let active = ActiveRunV1 {
            run_id,
            kind,
            started_at_ms,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            correlation,
            current_stage: initial_stage(kind),
            runtimes,
            stages: Vec::new(),
            input,
            clear_epoch,
        };
        insert_active(&transaction, &active)?;
        transaction.commit().map_err(db_error)?;
        Ok(active)
    }

    pub(crate) fn update_active(
        &self,
        correlation: &RunCorrelationV1,
        update: impl FnOnce(&mut ActiveRunV1),
    ) -> Result<bool, String> {
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(db_error)?;
        let Some(mut active) = active_by_correlation_tx(&transaction, correlation)? else {
            return Ok(false);
        };
        update(&mut active);
        let payload = serde_json::to_string(&active).map_err(|_| invalid_record())?;
        let changed = transaction
            .execute(
                "UPDATE active_runs SET payload_json = ?, current_stage = ?
                 WHERE run_id = ?",
                params![payload, stage_name(active.current_stage), active.run_id],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        Ok(changed == 1)
    }

    pub(crate) fn complete(
        &self,
        correlation: &RunCorrelationV1,
        outcome: RunOutcomeV1,
        stages: Vec<StageTimingV1>,
        input: Option<ContentFreeInputSummaryV1>,
        runtimes: Option<Vec<RuntimeIdentityV1>>,
    ) -> Result<Option<PerformanceRunV1>, String> {
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(db_error)?;
        let Some(mut active) = active_by_correlation_tx(&transaction, correlation)? else {
            return Ok(None);
        };
        if active.clear_epoch != clear_epoch_tx(&transaction)? {
            transaction
                .execute("DELETE FROM active_runs WHERE run_id = ?", [&active.run_id])
                .map_err(db_error)?;
            transaction.commit().map_err(db_error)?;
            return Ok(None);
        }

        merge_stages(&mut active.stages, stages);
        active.stages = canonical_stages(active.stages);
        if let Some(input) = input {
            active.input = input;
        }
        if let Some(runtimes) = runtimes {
            if active.runtimes.is_empty()
                || active
                    .runtimes
                    .iter()
                    .all(|runtime| runtime.warm_state == ModelWarmStateV1::Unknown)
            {
                active.runtimes = runtimes;
            }
        }
        let finished_at_ms = now_ms().max(active.started_at_ms);
        let resources = resource_summary_tx(
            &transaction,
            active.kind,
            active.started_at_ms,
            finished_at_ms,
        )?;
        let run = PerformanceRunV1 {
            schema_version: PERFORMANCE_RUN_SCHEMA_VERSION,
            run_id: active.run_id.clone(),
            kind: active.kind,
            started_at_ms: active.started_at_ms,
            finished_at_ms,
            app_version: active.app_version,
            correlation: active.correlation,
            outcome,
            runtimes: active.runtimes,
            stages: active.stages,
            input: active.input,
            resources,
            follow_ups: Vec::new(),
        };
        let payload = serde_json::to_string(&run).map_err(|_| invalid_record())?;
        let (correlation_kind, correlation_id) = run.correlation.storage_parts();
        transaction
            .execute(
                "INSERT OR IGNORE INTO completed_runs(
                    run_id, record_version, kind, correlation_kind, correlation_id,
                    started_at_ms, finished_at_ms, outcome_code, payload_json
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    run.run_id,
                    PERFORMANCE_RUN_SCHEMA_VERSION,
                    run.kind.as_str(),
                    correlation_kind,
                    to_i64(correlation_id)?,
                    run.started_at_ms,
                    run.finished_at_ms,
                    run.outcome.code(),
                    payload
                ],
            )
            .map_err(db_error)?;
        transaction
            .execute("DELETE FROM active_runs WHERE run_id = ?", [&run.run_id])
            .map_err(db_error)?;
        prune_completed_tx(&transaction)?;
        transaction.commit().map_err(db_error)?;
        Ok(Some(run))
    }

    pub(crate) fn insert_resource_sample(&self, sample: &ResourceSampleV1) -> Result<(), String> {
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(db_error)?;
        let payload = serde_json::to_string(sample).map_err(|_| invalid_record())?;
        transaction
            .execute(
                "INSERT INTO resource_samples(record_version, observed_at_ms, payload_json)
                 VALUES (?, ?, ?)",
                params![
                    RESOURCE_SAMPLE_SCHEMA_VERSION,
                    sample.observed_at_ms,
                    payload
                ],
            )
            .map_err(db_error)?;
        prune_resource_samples_tx(&transaction)?;
        transaction.commit().map_err(db_error)?;
        Ok(())
    }

    pub(crate) fn append_transform_follow_up(
        &self,
        correlation: &RunCorrelationV1,
        follow_up: TransformFollowUpV1,
    ) -> Result<Option<PerformanceRunV1>, String> {
        if !matches!(correlation, RunCorrelationV1::SelectedTextTransform { .. }) {
            return Ok(None);
        }
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(db_error)?;
        let (kind, id) = correlation.storage_parts();
        let row = transaction
            .query_row(
                "SELECT run_id, record_version, payload_json
                 FROM completed_runs
                 WHERE correlation_kind = ? AND correlation_id = ?
                 ORDER BY finished_at_ms DESC, rowid DESC LIMIT 1",
                params![kind, to_i64(id)?],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(db_error)?;
        let Some((run_id, version, payload)) = row else {
            return Ok(None);
        };
        if version != PERFORMANCE_RUN_SCHEMA_VERSION {
            return Ok(None);
        }
        let mut run: PerformanceRunV1 =
            serde_json::from_str(&payload).map_err(|_| invalid_record())?;
        run.follow_ups.push(follow_up);
        if run.follow_ups.len() > MAX_TRANSFORM_FOLLOW_UPS {
            let overflow = run.follow_ups.len() - MAX_TRANSFORM_FOLLOW_UPS;
            run.follow_ups.drain(0..overflow);
        }
        let payload = serde_json::to_string(&run).map_err(|_| invalid_record())?;
        transaction
            .execute(
                "UPDATE completed_runs SET payload_json = ? WHERE run_id = ?",
                params![payload, run_id],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        Ok(Some(run))
    }

    pub(crate) fn list(&self, limit: u32) -> Result<Vec<PerformanceRunV1>, String> {
        let connection = self.open()?;
        let limit = limit.clamp(1, MAX_COMPLETED_RUNS as u32);
        let mut statement = connection
            .prepare(
                "SELECT record_version, payload_json FROM completed_runs
                 ORDER BY finished_at_ms DESC, run_id DESC LIMIT ?",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map([limit], |row| {
                Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_error)?;
        let mut runs = Vec::new();
        for row in rows {
            let (version, payload) = row.map_err(db_error)?;
            if version != PERFORMANCE_RUN_SCHEMA_VERSION {
                continue;
            }
            runs.push(serde_json::from_str(&payload).map_err(|_| invalid_record())?);
        }
        Ok(runs)
    }

    pub(crate) fn get(&self, run_id: &str) -> Result<Option<PerformanceRunV1>, String> {
        if !valid_run_id(run_id) {
            return Err("The performance run ID is invalid.".to_string());
        }
        let connection = self.open()?;
        let row = connection
            .query_row(
                "SELECT record_version, payload_json FROM completed_runs WHERE run_id = ?",
                [run_id],
                |row| Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        match row {
            None => Ok(None),
            Some((version, _)) if version != PERFORMANCE_RUN_SCHEMA_VERSION => {
                Err("This performance run uses an unsupported record version.".to_string())
            }
            Some((_, payload)) => serde_json::from_str(&payload)
                .map(Some)
                .map_err(|_| invalid_record()),
        }
    }

    pub(crate) fn resource_window(&self) -> Result<Vec<ResourceSampleV1>, String> {
        let connection = self.open()?;
        resource_samples_tx(&connection, i64::MIN, i64::MAX)
    }

    pub(crate) fn clear(&self) -> Result<(), String> {
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(db_error)?;
        let next_epoch = clear_epoch_tx(&transaction)?.saturating_add(1);
        transaction
            .execute(
                "UPDATE performance_meta SET value = ? WHERE key = 'clear_epoch'",
                [to_i64(next_epoch)?],
            )
            .map_err(db_error)?;
        transaction
            .execute("DELETE FROM active_runs", [])
            .map_err(db_error)?;
        transaction
            .execute("DELETE FROM completed_runs", [])
            .map_err(db_error)?;
        transaction
            .execute("DELETE FROM resource_samples", [])
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)
    }

    #[cfg(test)]
    pub(crate) fn counts(&self) -> Result<(u64, u64, u64), String> {
        let connection = self.open()?;
        let active = count(&connection, "active_runs")?;
        let completed = count(&connection, "completed_runs")?;
        let samples = count(&connection, "resource_samples")?;
        Ok((active, completed, samples))
    }

    fn recover_stale_runs(&self, connection: &mut Connection) -> Result<(), String> {
        let correlations = {
            let mut statement = connection
                .prepare("SELECT payload_json FROM active_runs")
                .map_err(db_error)?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(db_error)?
                .filter_map(|row| row.ok())
                .filter_map(|json| serde_json::from_str::<ActiveRunV1>(&json).ok())
                .map(|run| (run.correlation, run.current_stage, run.input, run.runtimes))
                .collect::<Vec<_>>();
            rows
        };
        for (correlation, stage, input, runtimes) in correlations {
            let _ = self.complete(
                &correlation,
                RunOutcomeV1::Interrupted {
                    stage,
                    error_code: StableRunErrorV1::InterruptedByRestart,
                },
                Vec::new(),
                Some(input),
                Some(runtimes),
            )?;
        }
        Ok(())
    }
}

fn migrate(connection: &mut Connection) -> Result<(), String> {
    let current: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(db_error)?;
    if current > LATEST_DB_SCHEMA_VERSION {
        return Err(format!(
            "The diagnostics database uses schema version {current}, which is newer than this Murmur build supports."
        ));
    }
    if current == 0 {
        let transaction = connection.transaction().map_err(db_error)?;
        transaction
            .execute_batch(
                r#"
                CREATE TABLE performance_meta (
                    key TEXT PRIMARY KEY NOT NULL,
                    value INTEGER NOT NULL
                );
                INSERT INTO performance_meta(key, value) VALUES ('clear_epoch', 0);

                CREATE TABLE active_runs (
                    run_id TEXT PRIMARY KEY NOT NULL,
                    kind TEXT NOT NULL,
                    correlation_kind TEXT NOT NULL,
                    correlation_id INTEGER NOT NULL,
                    started_at_ms INTEGER NOT NULL,
                    current_stage TEXT NOT NULL,
                    payload_json TEXT NOT NULL,
                    UNIQUE(correlation_kind, correlation_id)
                );

                CREATE TABLE completed_runs (
                    run_id TEXT PRIMARY KEY NOT NULL,
                    record_version INTEGER NOT NULL,
                    kind TEXT NOT NULL,
                    correlation_kind TEXT NOT NULL,
                    correlation_id INTEGER NOT NULL,
                    started_at_ms INTEGER NOT NULL,
                    finished_at_ms INTEGER NOT NULL,
                    outcome_code TEXT NOT NULL,
                    payload_json TEXT NOT NULL
                );
                CREATE INDEX completed_runs_finished
                    ON completed_runs(finished_at_ms DESC, run_id DESC);

                CREATE TABLE resource_samples (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    record_version INTEGER NOT NULL,
                    observed_at_ms INTEGER NOT NULL,
                    payload_json TEXT NOT NULL
                );
                CREATE INDEX resource_samples_observed
                    ON resource_samples(observed_at_ms, id);
                "#,
            )
            .map_err(db_error)?;
        transaction
            .pragma_update(None, "user_version", LATEST_DB_SCHEMA_VERSION)
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
    }
    Ok(())
}

fn insert_active(transaction: &Transaction<'_>, active: &ActiveRunV1) -> Result<(), String> {
    let payload = serde_json::to_string(active).map_err(|_| invalid_record())?;
    let (correlation_kind, correlation_id) = active.correlation.storage_parts();
    transaction
        .execute(
            "INSERT INTO active_runs(
                run_id, kind, correlation_kind, correlation_id, started_at_ms,
                current_stage, payload_json
             ) VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                active.run_id,
                active.kind.as_str(),
                correlation_kind,
                to_i64(correlation_id)?,
                active.started_at_ms,
                stage_name(active.current_stage),
                payload
            ],
        )
        .map_err(db_error)?;
    Ok(())
}

fn active_by_correlation_tx(
    transaction: &Transaction<'_>,
    correlation: &RunCorrelationV1,
) -> Result<Option<ActiveRunV1>, String> {
    let (kind, id) = correlation.storage_parts();
    transaction
        .query_row(
            "SELECT payload_json FROM active_runs
             WHERE correlation_kind = ? AND correlation_id = ?",
            params![kind, to_i64(id)?],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(db_error)?
        .map(|json| serde_json::from_str(&json).map_err(|_| invalid_record()))
        .transpose()
}

fn clear_epoch_tx(transaction: &Transaction<'_>) -> Result<u64, String> {
    transaction
        .query_row(
            "SELECT value FROM performance_meta WHERE key = 'clear_epoch'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(db_error)?
        .try_into()
        .map_err(|_| invalid_record())
}

fn merge_stages(existing: &mut Vec<StageTimingV1>, incoming: Vec<StageTimingV1>) {
    for stage in incoming {
        if let Some(slot) = existing.iter_mut().find(|entry| entry.stage == stage.stage) {
            if !matches!(
                stage.stage,
                PerformanceStageV1::ModelQueue | PerformanceStageV1::ModelLoad
            ) {
                *slot = stage;
            }
        } else {
            existing.push(stage);
        }
    }
}

const ALL_STAGES: &[PerformanceStageV1] = &[
    PerformanceStageV1::CaptureFinalization,
    PerformanceStageV1::FileDecode,
    PerformanceStageV1::Vad,
    PerformanceStageV1::ModelQueue,
    PerformanceStageV1::ModelLoad,
    PerformanceStageV1::InferenceDecode,
    PerformanceStageV1::TranscriptTransform,
    PerformanceStageV1::Cleanup,
    PerformanceStageV1::VoiceCommands,
    PerformanceStageV1::SmartCorrection,
    PerformanceStageV1::SmartFormatting,
    PerformanceStageV1::IdeContext,
    PerformanceStageV1::CliCommand,
    PerformanceStageV1::FileOutput,
    PerformanceStageV1::ClipboardPaste,
    PerformanceStageV1::FileReturn,
    PerformanceStageV1::TotalProcessing,
    PerformanceStageV1::SelectedTextCapture,
    PerformanceStageV1::InstructionCapture,
    PerformanceStageV1::InstructionAsr,
    PerformanceStageV1::SidecarSpawnLoad,
    PerformanceStageV1::Generation,
    PerformanceStageV1::ReviewReady,
    PerformanceStageV1::Apply,
    PerformanceStageV1::Undo,
];

fn canonical_stages(measured: Vec<StageTimingV1>) -> Vec<StageTimingV1> {
    ALL_STAGES
        .iter()
        .map(|stage| {
            measured
                .iter()
                .find(|timing| timing.stage == *stage)
                .cloned()
                .unwrap_or_else(|| StageTimingV1::not_applicable(*stage))
        })
        .collect()
}

fn prune_completed_tx(transaction: &Transaction<'_>) -> Result<(), String> {
    transaction
        .execute(
            "DELETE FROM completed_runs
             WHERE run_id NOT IN (
                 SELECT run_id FROM completed_runs
                 ORDER BY finished_at_ms DESC, run_id DESC LIMIT ?
             )",
            [MAX_COMPLETED_RUNS as u32],
        )
        .map_err(db_error)?;
    Ok(())
}

fn prune_resource_samples_tx(transaction: &Transaction<'_>) -> Result<(), String> {
    transaction
        .execute(
            "DELETE FROM resource_samples
             WHERE id NOT IN (
                 SELECT id FROM resource_samples ORDER BY id DESC LIMIT ?
             )",
            [MAX_RESOURCE_SAMPLES as u32],
        )
        .map_err(db_error)?;
    Ok(())
}

fn resource_summary_tx(
    transaction: &Transaction<'_>,
    kind: PerformanceRunKindV1,
    started_at_ms: i64,
    finished_at_ms: i64,
) -> Result<ResourceSummaryV1, String> {
    let samples = resource_samples_tx(transaction, started_at_ms, finished_at_ms)?;
    if samples.is_empty() {
        return Ok(ResourceSummaryV1::unavailable_for(kind));
    }
    let host_cpu = samples
        .iter()
        .filter_map(|sample| sample.host.cpu_percent.value().copied())
        .collect::<Vec<_>>();
    let main_cpu = samples
        .iter()
        .filter_map(|sample| sample.main_process.cpu_percent.value().copied())
        .collect::<Vec<_>>();
    let rss = samples
        .iter()
        .filter_map(|sample| sample.main_process.rss_bytes.value().copied())
        .collect::<Vec<_>>();
    let rust_heap = samples
        .iter()
        .filter_map(|sample| sample.main_process.rust_heap_bytes.value().copied())
        .collect::<Vec<_>>();
    let ffi_heap = samples
        .iter()
        .filter_map(|sample| sample.main_process.ffi_native_heap_bytes.value().copied())
        .collect::<Vec<_>>();
    let sidecar = if kind == PerformanceRunKindV1::SelectedTextTransform {
        let sidecar_cpu = samples
            .iter()
            .filter_map(|sample| sample.sidecar_process.cpu_percent.value().copied())
            .collect::<Vec<_>>();
        let sidecar_rss = samples
            .iter()
            .filter_map(|sample| sample.sidecar_process.rss_bytes.value().copied())
            .collect::<Vec<_>>();
        SidecarResourceSummaryV1 {
            cpu_percent: range_f32(&sidecar_cpu),
            rss_bytes: range_u64(&sidecar_rss),
        }
    } else {
        SidecarResourceSummaryV1 {
            cpu_percent: ResourceRangeV1::not_applicable(),
            rss_bytes: ResourceRangeV1::not_applicable(),
        }
    };
    Ok(ResourceSummaryV1 {
        sample_count: samples.len().try_into().unwrap_or(u32::MAX),
        host: HostResourceSummaryV1 {
            cpu_percent: range_f32(&host_cpu),
        },
        main_process: ProcessResourceSummaryV1 {
            cpu_percent: range_f32(&main_cpu),
            rss_bytes: range_u64(&rss),
            rust_heap_bytes: range_u64(&rust_heap),
            ffi_native_heap_bytes: range_u64(&ffi_heap),
        },
        sidecar_process: sidecar,
    })
}

fn resource_samples_tx(
    connection: &Connection,
    started_at_ms: i64,
    finished_at_ms: i64,
) -> Result<Vec<ResourceSampleV1>, String> {
    let mut statement = connection
        .prepare(
            "SELECT record_version, payload_json FROM resource_samples
             WHERE observed_at_ms BETWEEN ? AND ?
             ORDER BY observed_at_ms ASC, id ASC",
        )
        .map_err(db_error)?;
    let rows = statement
        .query_map(params![started_at_ms, finished_at_ms], |row| {
            Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(db_error)?;
    let mut samples = Vec::new();
    for row in rows {
        let (version, payload) = row.map_err(db_error)?;
        if version != RESOURCE_SAMPLE_SCHEMA_VERSION {
            continue;
        }
        samples.push(serde_json::from_str(&payload).map_err(|_| invalid_record())?);
    }
    Ok(samples)
}

fn range_f32(values: &[f32]) -> ResourceRangeV1<f32> {
    if values.is_empty() {
        return ResourceRangeV1::unavailable(UnavailableReasonV1::NoSamples);
    }
    ResourceRangeV1 {
        start: MeasurementV1::measured(values[0]),
        average: MeasurementV1::measured(
            values.iter().map(|value| f64::from(*value)).sum::<f64>() as f32 / values.len() as f32,
        ),
        peak: MeasurementV1::measured(values.iter().copied().fold(f32::NEG_INFINITY, f32::max)),
        end: MeasurementV1::measured(values[values.len() - 1]),
    }
}

fn range_u64(values: &[u64]) -> ResourceRangeV1<u64> {
    if values.is_empty() {
        return ResourceRangeV1::unavailable(UnavailableReasonV1::NoSamples);
    }
    ResourceRangeV1 {
        start: MeasurementV1::measured(values[0]),
        average: MeasurementV1::measured(
            (values.iter().map(|value| u128::from(*value)).sum::<u128>() / values.len() as u128)
                as u64,
        ),
        peak: MeasurementV1::measured(values.iter().copied().max().unwrap_or(0)),
        end: MeasurementV1::measured(values[values.len() - 1]),
    }
}

fn initial_stage(kind: PerformanceRunKindV1) -> PerformanceStageV1 {
    match kind {
        PerformanceRunKindV1::Dictation => PerformanceStageV1::CaptureFinalization,
        PerformanceRunKindV1::FileTranscription => PerformanceStageV1::FileDecode,
        PerformanceRunKindV1::SelectedTextTransform => PerformanceStageV1::SelectedTextCapture,
    }
}

fn stage_name(stage: PerformanceStageV1) -> String {
    serde_json::to_string(&stage)
        .unwrap_or_else(|_| "\"unknown\"".to_string())
        .trim_matches('"')
        .to_string()
}

fn valid_run_id(run_id: &str) -> bool {
    run_id.len() == 32 && run_id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn quick_check(connection: &Connection) -> Result<(), String> {
    let result: String = connection
        .pragma_query_value(None, "quick_check", |row| row.get(0))
        .map_err(db_error)?;
    if result == "ok" {
        Ok(())
    } else {
        Err("The diagnostics database failed its integrity check.".to_string())
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn to_i64(value: u64) -> Result<i64, String> {
    value
        .try_into()
        .map_err(|_| "The diagnostics correlation ID is out of range.".to_string())
}

#[cfg(test)]
fn count(connection: &Connection, table: &str) -> Result<u64, String> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    connection
        .query_row(&sql, [], |row| row.get::<_, i64>(0))
        .and_then(|value| {
            value.try_into().map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Integer,
                    Box::new(error),
                )
            })
        })
        .map_err(db_error)
}

fn db_error(_error: rusqlite::Error) -> String {
    "The local diagnostics database operation failed.".to_string()
}

fn storage_error() -> String {
    "The local diagnostics storage directory is unavailable.".to_string()
}

fn invalid_record() -> String {
    "The local diagnostics database contains an invalid record.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository() -> (tempfile::TempDir, PerformanceRepository) {
        let temp = tempfile::tempdir().unwrap();
        let repository =
            PerformanceRepository::initialize(temp.path().join("diagnostics")).unwrap();
        (temp, repository)
    }

    fn correlation(id: u64) -> RunCorrelationV1 {
        RunCorrelationV1::Dictation { recording_id: id }
    }

    fn sample(at: i64, cpu: MeasurementV1<f32>) -> ResourceSampleV1 {
        ResourceSampleV1 {
            schema_version: RESOURCE_SAMPLE_SCHEMA_VERSION,
            observed_at_ms: at,
            host: HostResourceSampleV1 {
                cpu_percent: cpu.clone(),
            },
            main_process: ProcessResourceSampleV1 {
                cpu_percent: cpu,
                rss_bytes: MeasurementV1::measured(100),
                rust_heap_bytes: MeasurementV1::measured(20),
                ffi_native_heap_bytes: MeasurementV1::measured(30),
            },
            sidecar_process: SidecarResourceSampleV1::unavailable(
                UnavailableReasonV1::DependencyPending,
            ),
        }
    }

    fn transform_correlation(id: u64) -> RunCorrelationV1 {
        RunCorrelationV1::SelectedTextTransform {
            transform_pass_id: id,
        }
    }

    #[test]
    fn completed_runs_are_capped_and_completion_is_idempotent() {
        let (_temp, repository) = repository();
        for id in 1..=201 {
            repository
                .begin(
                    PerformanceRunKindV1::Dictation,
                    correlation(id),
                    Vec::new(),
                    ContentFreeInputSummaryV1::audio(1_000),
                )
                .unwrap();
            assert!(repository
                .complete(
                    &correlation(id),
                    RunOutcomeV1::Success,
                    vec![StageTimingV1::measured(
                        PerformanceStageV1::TotalProcessing,
                        id
                    )],
                    None,
                    None,
                )
                .unwrap()
                .is_some());
        }
        assert_eq!(repository.counts().unwrap(), (0, 200, 0));
        assert!(repository
            .complete(
                &correlation(201),
                RunOutcomeV1::Success,
                Vec::new(),
                None,
                None
            )
            .unwrap()
            .is_none());
    }

    #[test]
    fn resource_window_is_capped_and_summarizes_scopes() {
        let (_temp, repository) = repository();
        let active = repository
            .begin(
                PerformanceRunKindV1::Dictation,
                correlation(1),
                Vec::new(),
                ContentFreeInputSummaryV1::audio(1_000),
            )
            .unwrap();
        for index in 0..601 {
            repository
                .insert_resource_sample(&sample(
                    active.started_at_ms,
                    MeasurementV1::measured(index as f32),
                ))
                .unwrap();
        }
        let run = repository
            .complete(
                &correlation(1),
                RunOutcomeV1::Success,
                Vec::new(),
                None,
                None,
            )
            .unwrap()
            .unwrap();
        assert_eq!(repository.counts().unwrap(), (0, 1, 600));
        assert_eq!(run.resources.sample_count, 600);
        assert!(matches!(
            run.resources.sidecar_process.rss_bytes.start,
            MeasurementV1::NotApplicable
        ));
    }

    #[test]
    fn transform_summarizes_measured_sidecar_and_bounds_follow_ups() {
        let (_temp, repository) = repository();
        let correlation = transform_correlation(41);
        let active = repository
            .begin(
                PerformanceRunKindV1::SelectedTextTransform,
                correlation.clone(),
                vec![RuntimeIdentityV1 {
                    role: RuntimeRoleV1::Generation,
                    model_id: "catalog-model".to_string(),
                    backend: RuntimeBackendV1::LlamaCpp,
                    accelerator: AcceleratorV1::MetalGpu,
                    warm_state: ModelWarmStateV1::Warm,
                }],
                ContentFreeInputSummaryV1::default(),
            )
            .unwrap();
        let mut resource = sample(active.started_at_ms, MeasurementV1::measured(1.0));
        resource.sidecar_process = SidecarResourceSampleV1 {
            cpu_percent: MeasurementV1::measured(25.0),
            rss_bytes: MeasurementV1::measured(456),
        };
        repository.insert_resource_sample(&resource).unwrap();
        let run = repository
            .complete(
                &correlation,
                RunOutcomeV1::Success,
                vec![StageTimingV1::measured(PerformanceStageV1::Generation, 12)],
                None,
                None,
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            run.resources.sidecar_process.rss_bytes.peak,
            MeasurementV1::measured(456)
        );

        for index in 0..10 {
            repository
                .append_transform_follow_up(
                    &correlation,
                    TransformFollowUpV1 {
                        kind: if index % 2 == 0 {
                            TransformFollowUpKindV1::Apply
                        } else {
                            TransformFollowUpKindV1::Undo
                        },
                        at_ms: index,
                        duration_ms: MeasurementV1::measured(index as u64),
                        outcome: StageOutcomeV1::Completed,
                    },
                )
                .unwrap();
        }
        let updated = repository.get(&run.run_id).unwrap().unwrap();
        assert_eq!(updated.follow_ups.len(), MAX_TRANSFORM_FOLLOW_UPS);
        assert_eq!(updated.follow_ups[0].at_ms, 2);
        repository.clear().unwrap();
        assert!(repository
            .append_transform_follow_up(
                &correlation,
                TransformFollowUpV1 {
                    kind: TransformFollowUpKindV1::Apply,
                    at_ms: 11,
                    duration_ms: MeasurementV1::measured(1),
                    outcome: StageOutcomeV1::Completed,
                },
            )
            .unwrap()
            .is_none());
    }

    #[test]
    fn clear_removes_only_diagnostics_and_invalidates_active_runs() {
        let (temp, repository) = repository();
        let unrelated = temp.path().join("logs").join("app.log");
        fs::create_dir_all(unrelated.parent().unwrap()).unwrap();
        fs::write(&unrelated, "keep").unwrap();
        repository
            .begin(
                PerformanceRunKindV1::Dictation,
                correlation(1),
                Vec::new(),
                ContentFreeInputSummaryV1::default(),
            )
            .unwrap();
        repository
            .insert_resource_sample(&sample(1, MeasurementV1::measured(0.0)))
            .unwrap();
        repository.clear().unwrap();
        assert_eq!(repository.counts().unwrap(), (0, 0, 0));
        assert!(repository
            .complete(
                &correlation(1),
                RunOutcomeV1::Success,
                Vec::new(),
                None,
                None
            )
            .unwrap()
            .is_none());
        assert_eq!(fs::read_to_string(unrelated).unwrap(), "keep");
    }

    #[test]
    fn restart_closes_stale_run_and_allows_reused_session_correlation() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("diagnostics");
        let repository = PerformanceRepository::initialize(root.clone()).unwrap();
        repository
            .begin(
                PerformanceRunKindV1::Dictation,
                correlation(1),
                Vec::new(),
                ContentFreeInputSummaryV1::default(),
            )
            .unwrap();
        let restarted = PerformanceRepository::initialize(root).unwrap();
        let interrupted = restarted.list(10).unwrap();
        assert_eq!(interrupted.len(), 1);
        assert!(matches!(
            interrupted[0].outcome,
            RunOutcomeV1::Interrupted {
                error_code: StableRunErrorV1::InterruptedByRestart,
                ..
            }
        ));

        restarted
            .begin(
                PerformanceRunKindV1::Dictation,
                correlation(1),
                Vec::new(),
                ContentFreeInputSummaryV1::default(),
            )
            .unwrap();
        restarted
            .complete(
                &correlation(1),
                RunOutcomeV1::Success,
                Vec::new(),
                None,
                None,
            )
            .unwrap();
        assert_eq!(restarted.list(10).unwrap().len(), 2);
    }

    #[test]
    fn unsupported_future_database_version_fails_without_rewriting_it() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("diagnostics");
        fs::create_dir_all(&root).unwrap();
        let db = root.join(DB_FILE);
        let connection = Connection::open(&db).unwrap();
        connection.pragma_update(None, "user_version", 99).unwrap();
        drop(connection);

        assert!(PerformanceRepository::initialize(root).is_err());
        let connection = Connection::open(db).unwrap();
        let version: u32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 99);
    }
}
