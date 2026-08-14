use super::types::*;
use rusqlite::{params, Connection, ErrorCode, OptionalExtension, Transaction};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

const DB_FILE: &str = "performance.sqlite3";
const LATEST_DB_SCHEMA_VERSION: u32 = 2;
const MAX_COMPLETED_RUNS: usize = 200;
const MAX_RESOURCE_SAMPLES: usize = 600;
const MAX_TRANSFORM_FOLLOW_UPS: usize = 8;
const BUSY_TIMEOUT: Duration = Duration::from_millis(25);
const RETRY_BACKOFFS: [Duration; 2] = [Duration::from_millis(5), Duration::from_millis(15)];
const MAX_OPERATION_ATTEMPTS: u8 = 3;

pub(crate) type StoreResult<T> = Result<T, PerformanceStoreError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PerformanceStoreError {
    pub(crate) operation: PerformanceStoreOperationV1,
    pub(crate) class: PerformanceStoreErrorClassV1,
    pub(crate) attempts: u8,
}

impl PerformanceStoreError {
    fn new(operation: PerformanceStoreOperationV1, class: PerformanceStoreErrorClassV1) -> Self {
        Self {
            operation,
            class,
            attempts: 1,
        }
    }

    fn for_operation(mut self, operation: PerformanceStoreOperationV1) -> Self {
        self.operation = operation;
        self
    }

    pub(crate) fn retry_exhausted(&self) -> bool {
        self.class == PerformanceStoreErrorClassV1::BusyLocked
            && self.attempts >= MAX_OPERATION_ATTEMPTS
    }
}

impl fmt::Display for PerformanceStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self.class {
            PerformanceStoreErrorClassV1::BusyLocked => {
                "The local diagnostics database is busy. Dictation was not affected."
            }
            PerformanceStoreErrorClassV1::StorageFull => {
                "The local diagnostics storage is full. Free disk space, then retry."
            }
            PerformanceStoreErrorClassV1::ReadOnly => {
                "The local diagnostics storage is read-only. Check its permissions, then retry."
            }
            PerformanceStoreErrorClassV1::Io => {
                "The local diagnostics storage could not be accessed. Retry after checking local storage."
            }
            PerformanceStoreErrorClassV1::CorruptIntegrity => {
                "The local diagnostics database failed its integrity check."
            }
            PerformanceStoreErrorClassV1::SchemaMigration => {
                "The local diagnostics database schema is not supported by this Murmur build."
            }
            PerformanceStoreErrorClassV1::InvalidRecord => {
                "The local diagnostics database contains an invalid record."
            }
            PerformanceStoreErrorClassV1::Unavailable => {
                "The local diagnostics store is unavailable."
            }
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for PerformanceStoreError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InitializationOutcome {
    Ready,
    Reinitialized { at_ms: i64 },
}

#[derive(Debug, Clone)]
pub(crate) struct PerformanceRepository {
    root: PathBuf,
    db_path: PathBuf,
}

impl PerformanceRepository {
    pub(crate) fn initialize(root: PathBuf) -> StoreResult<(Self, InitializationOutcome)> {
        fs::create_dir_all(&root).map_err(storage_error)?;
        let repository = Self {
            db_path: root.join(DB_FILE),
            root,
        };
        let existed = repository.db_path.exists();
        match repository.initialize_database() {
            Ok(()) => Ok((repository, InitializationOutcome::Ready)),
            Err(error)
                if existed && error.class == PerformanceStoreErrorClassV1::CorruptIntegrity =>
            {
                repository.quarantine_database()?;
                repository.initialize_database()?;
                Ok((
                    repository,
                    InitializationOutcome::Reinitialized { at_ms: now_ms() },
                ))
            }
            Err(error) => Err(error),
        }
    }

    /// Reopen and validate an existing store without ever quarantining it.
    /// This is the safe first step of user-requested recovery.
    pub(crate) fn reopen(root: PathBuf) -> StoreResult<(Self, InitializationOutcome)> {
        fs::create_dir_all(&root).map_err(storage_error)?;
        let repository = Self {
            db_path: root.join(DB_FILE),
            root,
        };
        repository.initialize_database()?;
        Ok((repository, InitializationOutcome::Ready))
    }

    fn initialize_database(&self) -> StoreResult<()> {
        (|| {
            let mut connection = self.open()?;
            migrate(&mut connection)?;
            quick_check(&connection)?;
            validate_records(&connection)?;
            self.recover_stale_runs(&mut connection)
        })()
        .map_err(|error: PerformanceStoreError| {
            error.for_operation(PerformanceStoreOperationV1::Initialize)
        })
    }

    fn open(&self) -> StoreResult<Connection> {
        let connection = Connection::open(&self.db_path).map_err(db_error)?;
        connection.busy_timeout(BUSY_TIMEOUT).map_err(db_error)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=NORMAL;
                 PRAGMA foreign_keys=ON;",
            )
            .map_err(db_error)?;
        Ok(connection)
    }

    pub(crate) fn reinitialize(root: PathBuf) -> StoreResult<(Self, InitializationOutcome)> {
        let repository = Self {
            db_path: root.join(DB_FILE),
            root,
        };
        if !repository.db_path.exists() {
            repository.initialize_database()?;
            return Ok((repository, InitializationOutcome::Ready));
        }
        match repository.initialize_database() {
            Ok(()) => return Ok((repository, InitializationOutcome::Ready)),
            Err(error)
                if matches!(
                    error.class,
                    PerformanceStoreErrorClassV1::CorruptIntegrity
                        | PerformanceStoreErrorClassV1::InvalidRecord
                ) => {}
            Err(error) => return Err(error),
        }
        // Reinitialization is permitted only after this fresh validation proves
        // physical corruption or an undecodable supported-version record.
        repository.quarantine_database()?;
        repository.initialize_database()?;
        Ok((
            repository,
            InitializationOutcome::Reinitialized { at_ms: now_ms() },
        ))
    }

    fn quarantine_database(&self) -> StoreResult<()> {
        let quarantine = self.root.join("quarantine");
        fs::create_dir_all(&quarantine).map_err(storage_error)?;
        let stamp = now_ms();
        for (index, source) in [
            self.db_path.clone(),
            sqlite_sidecar_path(&self.db_path, "-wal"),
            sqlite_sidecar_path(&self.db_path, "-shm"),
        ]
        .into_iter()
        .enumerate()
        {
            if !source.exists() {
                continue;
            }
            let suffix = match index {
                0 => "sqlite3",
                1 => "sqlite3-wal",
                _ => "sqlite3-shm",
            };
            let destination = unique_quarantine_path(&quarantine, stamp, suffix);
            fs::rename(source, destination).map_err(storage_error)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn begin(
        &self,
        kind: PerformanceRunKindV1,
        correlation: RunCorrelationV1,
        runtimes: Vec<RuntimeIdentityV1>,
        input: ContentFreeInputSummaryV1,
    ) -> StoreResult<ActiveRunV1> {
        self.begin_at(kind, correlation, runtimes, input, now_ms())
    }

    pub(crate) fn begin_at(
        &self,
        kind: PerformanceRunKindV1,
        correlation: RunCorrelationV1,
        runtimes: Vec<RuntimeIdentityV1>,
        input: ContentFreeInputSummaryV1,
        started_at_ms: i64,
    ) -> StoreResult<ActiveRunV1> {
        retry_operation(PerformanceStoreOperationV1::Begin, || {
            self.begin_once(
                kind,
                correlation.clone(),
                runtimes.clone(),
                input.clone(),
                started_at_ms,
            )
        })
    }

    fn begin_once(
        &self,
        kind: PerformanceRunKindV1,
        correlation: RunCorrelationV1,
        runtimes: Vec<RuntimeIdentityV1>,
        input: ContentFreeInputSummaryV1,
        started_at_ms: i64,
    ) -> StoreResult<ActiveRunV1> {
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(db_error)?;
        let run_id: String = transaction
            .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))
            .map_err(db_error)?;
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
            query_process: None,
        };
        insert_active(&transaction, &active)?;
        transaction.commit().map_err(db_error)?;
        Ok(active)
    }

    pub(crate) fn update_active(
        &self,
        correlation: &RunCorrelationV1,
        update: impl FnOnce(&mut ActiveRunV1) + Clone,
    ) -> StoreResult<bool> {
        retry_operation(PerformanceStoreOperationV1::Update, || {
            self.update_active_once(correlation, update.clone())
        })
    }

    fn update_active_once(
        &self,
        correlation: &RunCorrelationV1,
        update: impl FnOnce(&mut ActiveRunV1),
    ) -> StoreResult<bool> {
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
    ) -> StoreResult<Option<PerformanceRunV1>> {
        retry_operation(PerformanceStoreOperationV1::Complete, || {
            self.complete_once(
                correlation,
                outcome.clone(),
                stages.clone(),
                input.clone(),
                runtimes.clone(),
            )
        })
    }

    fn complete_once(
        &self,
        correlation: &RunCorrelationV1,
        outcome: RunOutcomeV1,
        stages: Vec<StageTimingV1>,
        input: Option<ContentFreeInputSummaryV1>,
        runtimes: Option<Vec<RuntimeIdentityV1>>,
    ) -> StoreResult<Option<PerformanceRunV1>> {
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
            query_process: active.query_process,
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

    pub(crate) fn insert_resource_sample(&self, sample: &ResourceSampleV1) -> StoreResult<()> {
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
    ) -> StoreResult<Option<PerformanceRunV1>> {
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

    pub(crate) fn list(&self, limit: u32) -> StoreResult<Vec<PerformanceRunV1>> {
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

    pub(crate) fn get(&self, run_id: &str) -> StoreResult<Option<PerformanceRunV1>> {
        if !valid_run_id(run_id) {
            return Err(PerformanceStoreError::new(
                PerformanceStoreOperationV1::Read,
                PerformanceStoreErrorClassV1::InvalidRecord,
            ));
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
                Err(PerformanceStoreError::new(
                    PerformanceStoreOperationV1::Read,
                    PerformanceStoreErrorClassV1::SchemaMigration,
                ))
            }
            Some((_, payload)) => serde_json::from_str(&payload)
                .map(Some)
                .map_err(|_| invalid_record()),
        }
    }

    pub(crate) fn resource_window(&self) -> StoreResult<Vec<ResourceSampleV1>> {
        let connection = self.open()?;
        resource_samples_tx(&connection, i64::MIN, i64::MAX)
    }

    pub(crate) fn clear(&self) -> StoreResult<()> {
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
    pub(crate) fn counts(&self) -> StoreResult<(u64, u64, u64)> {
        let connection = self.open()?;
        let active = count(&connection, "active_runs")?;
        let completed = count(&connection, "completed_runs")?;
        let samples = count(&connection, "resource_samples")?;
        Ok((active, completed, samples))
    }

    #[cfg(test)]
    pub(crate) fn test_connection(&self) -> StoreResult<Connection> {
        self.open()
    }

    fn recover_stale_runs(&self, connection: &mut Connection) -> StoreResult<()> {
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

fn migrate(connection: &mut Connection) -> StoreResult<()> {
    let current: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(db_error)?;
    if current > LATEST_DB_SCHEMA_VERSION {
        return Err(PerformanceStoreError::new(
            PerformanceStoreOperationV1::Initialize,
            PerformanceStoreErrorClassV1::SchemaMigration,
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
    } else if current == 1 {
        // V2 only extends the versioned JSON payload with an optional,
        // content-free Voice Query process summary. The relational shape and
        // V1 run envelope remain unchanged.
        let transaction = connection.transaction().map_err(db_error)?;
        transaction
            .pragma_update(None, "user_version", LATEST_DB_SCHEMA_VERSION)
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
    }
    Ok(())
}

fn insert_active(transaction: &Transaction<'_>, active: &ActiveRunV1) -> StoreResult<()> {
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
) -> StoreResult<Option<ActiveRunV1>> {
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

fn clear_epoch_tx(transaction: &Transaction<'_>) -> StoreResult<u64> {
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
    PerformanceStageV1::SpokenStructure,
    PerformanceStageV1::SpokenNumbers,
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

fn prune_completed_tx(transaction: &Transaction<'_>) -> StoreResult<()> {
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

fn prune_resource_samples_tx(transaction: &Transaction<'_>) -> StoreResult<()> {
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
) -> StoreResult<ResourceSummaryV1> {
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
    let sidecar = match kind {
        PerformanceRunKindV1::SelectedTextTransform => {
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
        }
        PerformanceRunKindV1::Dictation
        | PerformanceRunKindV1::FileTranscription
        | PerformanceRunKindV1::VoiceQuery => SidecarResourceSummaryV1 {
            cpu_percent: ResourceRangeV1::not_applicable(),
            rss_bytes: ResourceRangeV1::not_applicable(),
        },
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
) -> StoreResult<Vec<ResourceSampleV1>> {
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
        PerformanceRunKindV1::VoiceQuery => PerformanceStageV1::InstructionCapture,
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

fn retry_operation<T>(
    operation: PerformanceStoreOperationV1,
    mut attempt: impl FnMut() -> StoreResult<T>,
) -> StoreResult<T> {
    for attempt_index in 0..MAX_OPERATION_ATTEMPTS {
        match attempt() {
            Ok(value) => return Ok(value),
            Err(error) if error.class == PerformanceStoreErrorClassV1::BusyLocked => {
                let attempts = attempt_index.saturating_add(1);
                if attempts >= MAX_OPERATION_ATTEMPTS {
                    return Err(PerformanceStoreError {
                        operation,
                        class: PerformanceStoreErrorClassV1::BusyLocked,
                        attempts,
                    });
                }
                thread::sleep(RETRY_BACKOFFS[usize::from(attempt_index)]);
            }
            Err(error) => return Err(error.for_operation(operation)),
        }
    }
    unreachable!("the bounded retry loop always returns")
}

fn unique_quarantine_path(root: &Path, stamp: i64, suffix: &str) -> PathBuf {
    for sequence in 0_u16..=u16::MAX {
        let candidate = root.join(format!("performance-{stamp}-{sequence}.{suffix}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    root.join(format!("performance-{stamp}.overflow.{suffix}"))
}

fn sqlite_sidecar_path(database: &Path, suffix: &str) -> PathBuf {
    let mut path = database.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

fn quick_check(connection: &Connection) -> StoreResult<()> {
    let result: String = connection
        .pragma_query_value(None, "quick_check", |row| row.get(0))
        .map_err(db_error)?;
    if result == "ok" {
        Ok(())
    } else {
        Err(PerformanceStoreError::new(
            PerformanceStoreOperationV1::Initialize,
            PerformanceStoreErrorClassV1::CorruptIntegrity,
        ))
    }
}

fn validate_records(connection: &Connection) -> StoreResult<()> {
    {
        let mut active = connection
            .prepare("SELECT payload_json FROM active_runs")
            .map_err(db_error)?;
        let active_rows = active
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(db_error)?;
        for row in active_rows {
            let payload = row.map_err(db_error)?;
            serde_json::from_str::<ActiveRunV1>(&payload).map_err(|_| invalid_record())?;
        }
    }
    {
        let mut completed = connection
            .prepare("SELECT record_version, payload_json FROM completed_runs")
            .map_err(db_error)?;
        let completed_rows = completed
            .query_map([], |row| {
                Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_error)?;
        for row in completed_rows {
            let (version, payload) = row.map_err(db_error)?;
            if version == PERFORMANCE_RUN_SCHEMA_VERSION {
                serde_json::from_str::<PerformanceRunV1>(&payload).map_err(|_| invalid_record())?;
            }
        }
    }
    {
        let mut samples = connection
            .prepare("SELECT record_version, payload_json FROM resource_samples")
            .map_err(db_error)?;
        let sample_rows = samples
            .query_map([], |row| {
                Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_error)?;
        for row in sample_rows {
            let (version, payload) = row.map_err(db_error)?;
            if version == RESOURCE_SAMPLE_SCHEMA_VERSION {
                serde_json::from_str::<ResourceSampleV1>(&payload).map_err(|_| invalid_record())?;
            }
        }
    }
    Ok(())
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn to_i64(value: u64) -> StoreResult<i64> {
    value.try_into().map_err(|_| invalid_record())
}

#[cfg(test)]
fn count(connection: &Connection, table: &str) -> StoreResult<u64> {
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

fn db_error(error: rusqlite::Error) -> PerformanceStoreError {
    let class = match error {
        rusqlite::Error::SqliteFailure(sqlite, _) => match sqlite.code {
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => {
                PerformanceStoreErrorClassV1::BusyLocked
            }
            ErrorCode::DiskFull => PerformanceStoreErrorClassV1::StorageFull,
            ErrorCode::ReadOnly
            | ErrorCode::PermissionDenied
            | ErrorCode::AuthorizationForStatementDenied => PerformanceStoreErrorClassV1::ReadOnly,
            ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase => {
                PerformanceStoreErrorClassV1::CorruptIntegrity
            }
            ErrorCode::SchemaChanged => PerformanceStoreErrorClassV1::SchemaMigration,
            ErrorCode::SystemIoFailure
            | ErrorCode::CannotOpen
            | ErrorCode::FileLockingProtocolFailed
            | ErrorCode::NoLargeFileSupport => PerformanceStoreErrorClassV1::Io,
            _ => PerformanceStoreErrorClassV1::Io,
        },
        rusqlite::Error::InvalidColumnType(..)
        | rusqlite::Error::FromSqlConversionFailure(..)
        | rusqlite::Error::IntegralValueOutOfRange(..)
        | rusqlite::Error::Utf8Error(..) => PerformanceStoreErrorClassV1::InvalidRecord,
        rusqlite::Error::InvalidQuery
        | rusqlite::Error::InvalidParameterName(..)
        | rusqlite::Error::InvalidColumnName(..)
        | rusqlite::Error::InvalidColumnIndex(..) => PerformanceStoreErrorClassV1::SchemaMigration,
        _ => PerformanceStoreErrorClassV1::Io,
    };
    PerformanceStoreError::new(PerformanceStoreOperationV1::Read, class)
}

fn storage_error(error: std::io::Error) -> PerformanceStoreError {
    let class = match error.kind() {
        std::io::ErrorKind::PermissionDenied => PerformanceStoreErrorClassV1::ReadOnly,
        std::io::ErrorKind::StorageFull => PerformanceStoreErrorClassV1::StorageFull,
        _ => PerformanceStoreErrorClassV1::Io,
    };
    PerformanceStoreError::new(PerformanceStoreOperationV1::Initialize, class)
}

fn invalid_record() -> PerformanceStoreError {
    PerformanceStoreError::new(
        PerformanceStoreOperationV1::Read,
        PerformanceStoreErrorClassV1::InvalidRecord,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::time::Instant;

    fn repository() -> (tempfile::TempDir, PerformanceRepository) {
        let temp = tempfile::tempdir().unwrap();
        let repository = PerformanceRepository::initialize(temp.path().join("diagnostics"))
            .unwrap()
            .0;
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
        let repository = PerformanceRepository::initialize(root.clone()).unwrap().0;
        repository
            .begin(
                PerformanceRunKindV1::Dictation,
                correlation(1),
                Vec::new(),
                ContentFreeInputSummaryV1::default(),
            )
            .unwrap();
        let restarted = PerformanceRepository::initialize(root).unwrap().0;
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

    #[test]
    fn v1_database_migrates_without_losing_completed_or_active_runs() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("diagnostics");
        let repository = PerformanceRepository::initialize(root.clone()).unwrap().0;
        repository
            .begin(
                PerformanceRunKindV1::Dictation,
                correlation(71),
                Vec::new(),
                ContentFreeInputSummaryV1::default(),
            )
            .unwrap();
        let completed = repository
            .complete(
                &correlation(71),
                RunOutcomeV1::Success,
                Vec::new(),
                None,
                None,
            )
            .unwrap()
            .unwrap();
        repository
            .begin(
                PerformanceRunKindV1::Dictation,
                correlation(72),
                Vec::new(),
                ContentFreeInputSummaryV1::default(),
            )
            .unwrap();
        drop(repository);

        let db = root.join(DB_FILE);
        let connection = Connection::open(&db).unwrap();
        for table in ["completed_runs", "active_runs"] {
            let payload: String = connection
                .query_row(
                    &format!("SELECT payload_json FROM {table} LIMIT 1"),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(!payload.contains("queryProcess"));
        }
        connection.pragma_update(None, "user_version", 1).unwrap();
        drop(connection);

        let migrated = PerformanceRepository::initialize(root).unwrap().0;
        let connection = migrated.open().unwrap();
        let version: u32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2);
        drop(connection);
        let runs = migrated.list(10).unwrap();
        assert_eq!(runs.len(), 2);
        assert!(runs.iter().any(|run| run.run_id == completed.run_id));
        assert!(runs.iter().any(|run| matches!(
            run.outcome,
            RunOutcomeV1::Interrupted {
                error_code: StableRunErrorV1::InterruptedByRestart,
                ..
            }
        )));
    }

    #[test]
    fn voice_query_keeps_v1_run_envelope_and_canonical_stage_count() {
        let (_temp, repository) = repository();
        let correlation = RunCorrelationV1::VoiceQuery { query_pass_id: 9 };
        repository
            .begin(
                PerformanceRunKindV1::VoiceQuery,
                correlation.clone(),
                Vec::new(),
                ContentFreeInputSummaryV1::default(),
            )
            .unwrap();
        repository
            .update_active(&correlation, |active| {
                active.query_process = Some(QueryProcessSummaryV1 {
                    exit_code: Some(7),
                    stderr_present: true,
                });
            })
            .unwrap();
        let run = repository
            .complete(
                &correlation,
                RunOutcomeV1::Failed {
                    stage: PerformanceStageV1::Generation,
                    error_code: StableRunErrorV1::QueryFailed,
                },
                vec![StageTimingV1::measured(
                    PerformanceStageV1::TotalProcessing,
                    12,
                )],
                None,
                None,
            )
            .unwrap()
            .unwrap();
        assert_eq!(run.schema_version, 1);
        assert_eq!(run.stages.len(), 27);
        assert_eq!(
            run.query_process,
            Some(QueryProcessSummaryV1 {
                exit_code: Some(7),
                stderr_present: true,
            })
        );
        assert!(matches!(
            run.resources.sidecar_process.rss_bytes.start,
            MeasurementV1::NotApplicable
        ));
    }

    #[test]
    fn retry_policy_is_bounded_and_only_retries_busy_locked() {
        let attempts = AtomicU8::new(0);
        let started = Instant::now();
        let error = retry_operation(PerformanceStoreOperationV1::Begin, || {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err::<(), _>(PerformanceStoreError::new(
                PerformanceStoreOperationV1::Read,
                PerformanceStoreErrorClassV1::BusyLocked,
            ))
        })
        .unwrap_err();
        assert_eq!(attempts.load(Ordering::SeqCst), MAX_OPERATION_ATTEMPTS);
        assert_eq!(error.operation, PerformanceStoreOperationV1::Begin);
        assert_eq!(error.attempts, MAX_OPERATION_ATTEMPTS);
        assert!(error.retry_exhausted());
        assert!(started.elapsed() < Duration::from_millis(500));

        let attempts = AtomicU8::new(0);
        let value = retry_operation(PerformanceStoreOperationV1::Update, || {
            let attempt = attempts.fetch_add(1, Ordering::SeqCst) + 1;
            if attempt < MAX_OPERATION_ATTEMPTS {
                Err(PerformanceStoreError::new(
                    PerformanceStoreOperationV1::Read,
                    PerformanceStoreErrorClassV1::BusyLocked,
                ))
            } else {
                Ok(41)
            }
        })
        .unwrap();
        assert_eq!(value, 41);
        assert_eq!(attempts.load(Ordering::SeqCst), MAX_OPERATION_ATTEMPTS);

        let attempts = AtomicU8::new(0);
        let error = retry_operation(PerformanceStoreOperationV1::Complete, || {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err::<(), _>(PerformanceStoreError::new(
                PerformanceStoreOperationV1::Read,
                PerformanceStoreErrorClassV1::Io,
            ))
        })
        .unwrap_err();
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert_eq!(error.class, PerformanceStoreErrorClassV1::Io);
    }

    #[test]
    fn locked_writer_exhausts_quickly_and_a_later_begin_recovers() {
        let (_temp, repository) = repository();
        let blocker = repository.open().unwrap();
        blocker.execute_batch("BEGIN IMMEDIATE;").unwrap();
        let started = Instant::now();
        let error = repository
            .begin(
                PerformanceRunKindV1::Dictation,
                correlation(901),
                Vec::new(),
                ContentFreeInputSummaryV1::default(),
            )
            .unwrap_err();
        assert_eq!(error.class, PerformanceStoreErrorClassV1::BusyLocked);
        assert_eq!(error.attempts, MAX_OPERATION_ATTEMPTS);
        assert!(started.elapsed() < Duration::from_millis(500));
        blocker.execute_batch("ROLLBACK;").unwrap();
        assert!(repository
            .begin(
                PerformanceRunKindV1::Dictation,
                correlation(901),
                Vec::new(),
                ContentFreeInputSummaryV1::default(),
            )
            .is_ok());
    }

    #[test]
    fn corrupt_database_is_quarantined_with_sidecars_and_reinitialized() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("diagnostics");
        fs::create_dir_all(&root).unwrap();
        let db_path = root.join(DB_FILE);
        fs::write(&db_path, b"not a sqlite database").unwrap();
        fs::write(sqlite_sidecar_path(&db_path, "-wal"), b"wal evidence").unwrap();
        fs::write(sqlite_sidecar_path(&db_path, "-shm"), b"shm evidence").unwrap();

        let (repository, outcome) = PerformanceRepository::initialize(root.clone()).unwrap();
        assert!(matches!(
            outcome,
            InitializationOutcome::Reinitialized { .. }
        ));
        assert_eq!(repository.counts().unwrap(), (0, 0, 0));
        let quarantined = fs::read_dir(root.join("quarantine"))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(!quarantined.is_empty());
        assert!(quarantined.len() <= 3);
    }

    #[test]
    fn quarantine_preserves_database_wal_and_shm_as_separate_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("diagnostics");
        fs::create_dir_all(&root).unwrap();
        let repository = PerformanceRepository {
            db_path: root.join(DB_FILE),
            root: root.clone(),
        };
        fs::write(&repository.db_path, b"database evidence").unwrap();
        fs::write(
            sqlite_sidecar_path(&repository.db_path, "-wal"),
            b"wal evidence",
        )
        .unwrap();
        fs::write(
            sqlite_sidecar_path(&repository.db_path, "-shm"),
            b"shm evidence",
        )
        .unwrap();

        repository.quarantine_database().unwrap();
        assert!(!repository.db_path.exists());
        assert!(!sqlite_sidecar_path(&repository.db_path, "-wal").exists());
        assert!(!sqlite_sidecar_path(&repository.db_path, "-shm").exists());
        let mut evidence = fs::read_dir(root.join("quarantine"))
            .unwrap()
            .map(|entry| fs::read(entry.unwrap().path()).unwrap())
            .collect::<Vec<_>>();
        evidence.sort();
        let mut expected = vec![
            b"database evidence".to_vec(),
            b"wal evidence".to_vec(),
            b"shm evidence".to_vec(),
        ];
        expected.sort();
        assert_eq!(evidence, expected);
    }

    #[test]
    fn explicit_recovery_does_not_quarantine_a_healthy_database() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("diagnostics");
        let (_repository, _) = PerformanceRepository::initialize(root.clone()).unwrap();
        let (_repository, outcome) = PerformanceRepository::reinitialize(root.clone()).unwrap();
        assert_eq!(outcome, InitializationOutcome::Ready);
        assert!(!root.join("quarantine").exists());
    }

    #[test]
    fn reopen_reports_corruption_without_quarantining() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("diagnostics");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(DB_FILE), b"not a sqlite database").unwrap();

        let error = PerformanceRepository::reopen(root.clone()).unwrap_err();
        assert_eq!(error.class, PerformanceStoreErrorClassV1::CorruptIntegrity);
        assert!(root.join(DB_FILE).exists());
        assert!(!root.join("quarantine").exists());
    }

    #[test]
    fn invalid_supported_record_requires_explicit_evidence_based_reinitialization() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("diagnostics");
        let (repository, _) = PerformanceRepository::initialize(root.clone()).unwrap();
        repository
            .open()
            .unwrap()
            .execute(
                "INSERT INTO completed_runs(
                    run_id, record_version, kind, correlation_kind, correlation_id,
                    started_at_ms, finished_at_ms, outcome_code, payload_json
                 ) VALUES ('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 1, 'dictation',
                           'dictation', 1, 1, 2, 'success', 'not json')",
                [],
            )
            .unwrap();
        drop(repository);

        let error = PerformanceRepository::initialize(root.clone()).unwrap_err();
        assert_eq!(error.class, PerformanceStoreErrorClassV1::InvalidRecord);
        assert!(!root.join("quarantine").exists());

        let (recovered, outcome) = PerformanceRepository::reinitialize(root.clone()).unwrap();
        assert!(matches!(
            outcome,
            InitializationOutcome::Reinitialized { .. }
        ));
        assert_eq!(recovered.counts().unwrap(), (0, 0, 0));
        assert!(!fs::read_dir(root.join("quarantine"))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn sqlite_and_filesystem_failures_map_to_stable_safe_classes() {
        let classified = |code| {
            db_error(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(code),
                Some("private SQL and /private/path".to_string()),
            ))
        };
        assert_eq!(
            classified(rusqlite::ffi::SQLITE_BUSY).class,
            PerformanceStoreErrorClassV1::BusyLocked
        );
        assert_eq!(
            classified(rusqlite::ffi::SQLITE_FULL).class,
            PerformanceStoreErrorClassV1::StorageFull
        );
        let read_only = classified(rusqlite::ffi::SQLITE_READONLY);
        assert_eq!(read_only.class, PerformanceStoreErrorClassV1::ReadOnly);
        assert!(!read_only.to_string().contains("private"));
        assert_eq!(
            classified(rusqlite::ffi::SQLITE_IOERR).class,
            PerformanceStoreErrorClassV1::Io
        );
        assert_eq!(
            classified(rusqlite::ffi::SQLITE_CORRUPT).class,
            PerformanceStoreErrorClassV1::CorruptIntegrity
        );
        assert_eq!(
            classified(rusqlite::ffi::SQLITE_SCHEMA).class,
            PerformanceStoreErrorClassV1::SchemaMigration
        );
        assert_eq!(
            storage_error(std::io::Error::from(std::io::ErrorKind::StorageFull)).class,
            PerformanceStoreErrorClassV1::StorageFull
        );
        assert_eq!(
            storage_error(std::io::Error::from(std::io::ErrorKind::PermissionDenied)).class,
            PerformanceStoreErrorClassV1::ReadOnly
        );

        let temp = tempfile::tempdir().unwrap();
        let error = Connection::open(temp.path()).unwrap_err();
        assert_eq!(db_error(error).class, PerformanceStoreErrorClassV1::Io);
    }
}
