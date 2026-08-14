use crate::audio::{
    AudioBackendOrderSource, AudioFailureKind, AudioInitPhase, AudioStartupDiagnostic,
};
use crate::audio_lifecycle::{self, AudioCancelReason, AudioLifecycleEvent};
use crate::state::DictationStatus;
use crate::{MutexExt, State};
use murmur_capture_helper_protocol::{
    CaptureBackend, CaptureSetupStep, SetupTransition as ProtocolSetupTransition,
};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{Emitter, Manager};

const REPORT_SCHEMA_VERSION: u8 = 1;
const MAX_CYCLES: u8 = 10;
const MAX_DEVICE_ID_BYTES: usize = 4_096;
const MAX_RUN_ID_BYTES: usize = 64;
const MAX_REPORT_BYTES: usize = 128 * 1024;
// The worker may legitimately wait through the full 120-second TCC prompt
// watchdog and then use the 30-second active initialization deadline.
const CYCLE_EVENT_TIMEOUT: Duration = Duration::from_secs(165);
// Cancellation acknowledgement is bounded at 2 seconds, production stop
// callers allow 12 seconds for helper teardown, and the capture worker's own
// termination budgets are shorter. After the first full-cycle timeout, allow
// one bounded grace window for the authoritative post-join Idle event.
const CYCLE_IDLE_AFTER_CANCEL_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CYCLE_TIMEOUTS: u8 = 2;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MicrophoneStartupBenchmarkRequest {
    run_id: String,
    device_id: String,
    cycles: u8,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceSelection {
    SystemDefault,
    Pinned,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackendOrderSource {
    Default,
    SessionFirstPcmMemo,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StartupBackend {
    Auhal,
    Cpal,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AttemptOutcome {
    Ready,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CycleOutcome {
    Ready,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SetupTransition {
    Entered,
    Completed,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StartupFailureKind {
    PermissionDenied,
    DeviceUnavailable,
    HostUnavailable,
    InvalidInput,
    ResourceExhausted,
    StreamInvalidated,
    UnsupportedConfig,
    BackendError,
    ProtocolError,
    FirstBufferTimeout,
    InitializationTimeout,
    PermissionPromptTimeout,
    TerminationUnconfirmed,
    WorkerPanicked,
    SignatureInvalid,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StartupFailurePhase {
    DeviceEnumeration,
    StreamBuild,
    FirstBufferWait,
    Runtime,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StartupSetupStep {
    DeviceResolution,
    AudioUnitCreation,
    AudioUnitNew,
    EnableInputIo,
    DisableOutputIo,
    SetCurrentDevice,
    FormatConfiguration,
    CallbackInstallation,
    DefaultConfig,
    StreamBuild,
    StreamStart,
    AwaitingFirstCallback,
    SystemTapCreate,
    AggregateDeviceCreate,
    IoProcCreate,
    IoProcStart,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MicrophoneStartupAttemptResult {
    resolution_pass: u8,
    attempt_index: u8,
    backend: StartupBackend,
    outcome: AttemptOutcome,
    attempt_start_to_first_pcm_ms: Option<u64>,
    active_elapsed_ms: Option<u64>,
    failure_kind: Option<StartupFailureKind>,
    failure_phase: Option<StartupFailurePhase>,
    last_setup_step: Option<StartupSetupStep>,
    last_setup_transition: Option<SetupTransition>,
    attempt_budget_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MicrophoneStartupCycleResult {
    cycle: u8,
    outcome: CycleOutcome,
    cycle_start_to_first_pcm_ms: Option<u64>,
    backend: Option<StartupBackend>,
    fallback_occurred: bool,
    failure_kind: Option<StartupFailureKind>,
    last_setup_step: Option<StartupSetupStep>,
    last_setup_transition: Option<SetupTransition>,
    backend_order: Vec<StartupBackend>,
    backend_order_source: Option<BackendOrderSource>,
    attempts: Vec<MicrophoneStartupAttemptResult>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MicrophoneStartupBenchmarkReport {
    schema_version: u8,
    run_id: String,
    benchmark_run_id: u64,
    app_version: String,
    platform: String,
    device_selection: DeviceSelection,
    requested_cycles: u8,
    completed_cycles: u8,
    cancelled: bool,
    started_at: String,
    finished_at: String,
    cycles: Vec<MicrophoneStartupCycleResult>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ProgressPhase {
    Starting,
    Capturing,
    Stopping,
    Recovering,
    Waiting,
    Complete,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MicrophoneStartupBenchmarkProgress<'a> {
    schema_version: u8,
    run_id: &'a str,
    benchmark_run_id: u64,
    completed_cycles: u8,
    total_cycles: u8,
    current_cycle: u8,
    phase: ProgressPhase,
    backend: Option<StartupBackend>,
    fallback_occurred: bool,
    last_setup_step: Option<StartupSetupStep>,
    last_setup_transition: Option<SetupTransition>,
}

#[derive(Debug)]
enum CycleEvent {
    Diagnostic(AudioStartupDiagnostic),
    Ready,
    Failed { kind: StartupFailureKind },
    Recovering,
    Idle,
}

struct ActiveRun {
    external_run_id: String,
    benchmark_run_id: u64,
    current_cycle_id: Option<u64>,
    sender: Option<Sender<CycleEvent>>,
    cancelled: bool,
}

/// Exact run/cycle ownership for the microphone benchmark. The normal
/// BenchmarkCoordinator remains the cross-pipeline exclusion slot; this state
/// adds generation-safe progress routing and cancellation.
pub(crate) struct MicrophoneStartupBenchmarkState {
    next_run_id: AtomicU64,
    next_cycle_id: AtomicU64,
    active: Mutex<Option<ActiveRun>>,
}

impl Default for MicrophoneStartupBenchmarkState {
    fn default() -> Self {
        Self {
            next_run_id: AtomicU64::new(1),
            next_cycle_id: AtomicU64::new(1),
            active: Mutex::new(None),
        }
    }
}

impl MicrophoneStartupBenchmarkState {
    fn claim(&self, external_run_id: String) -> Result<u64, String> {
        let mut active = self.active.lock_or_recover();
        if active.is_some() {
            return Err("A microphone startup benchmark is already running".to_string());
        }
        let benchmark_run_id = self.next_run_id.fetch_add(1, Ordering::Relaxed);
        *active = Some(ActiveRun {
            external_run_id,
            benchmark_run_id,
            current_cycle_id: None,
            sender: None,
            cancelled: false,
        });
        Ok(benchmark_run_id)
    }

    fn begin_cycle(&self, benchmark_run_id: u64) -> Option<(u64, Receiver<CycleEvent>)> {
        let mut active = self.active.lock_or_recover();
        let run = active.as_mut().filter(|run| {
            run.benchmark_run_id == benchmark_run_id
                && !run.cancelled
                && run.current_cycle_id.is_none()
        })?;
        let cycle_id = self.next_cycle_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        run.current_cycle_id = Some(cycle_id);
        run.sender = Some(sender);
        Some((cycle_id, receiver))
    }

    fn send_if_current(&self, cycle_id: u64, event: CycleEvent) {
        let sender = self
            .active
            .lock_or_recover()
            .as_ref()
            .filter(|run| run.current_cycle_id == Some(cycle_id))
            .and_then(|run| run.sender.clone());
        if let Some(sender) = sender {
            let _ = sender.send(event);
        }
    }

    fn finish_cycle(&self, benchmark_run_id: u64, cycle_id: u64) {
        let mut active = self.active.lock_or_recover();
        if let Some(run) = active.as_mut().filter(|run| {
            run.benchmark_run_id == benchmark_run_id && run.current_cycle_id == Some(cycle_id)
        }) {
            run.current_cycle_id = None;
            run.sender = None;
        }
    }

    fn cancel_exact(&self, external_run_id: &str) -> Option<(u64, Option<u64>)> {
        let mut active = self.active.lock_or_recover();
        let run = active
            .as_mut()
            .filter(|run| run.external_run_id == external_run_id)?;
        run.cancelled = true;
        Some((run.benchmark_run_id, run.current_cycle_id))
    }

    fn is_cancelled(&self, benchmark_run_id: u64) -> bool {
        self.active
            .lock_or_recover()
            .as_ref()
            .is_none_or(|run| run.benchmark_run_id != benchmark_run_id || run.cancelled)
    }

    fn finish(&self, benchmark_run_id: u64) {
        let mut active = self.active.lock_or_recover();
        if active
            .as_ref()
            .is_some_and(|run| run.benchmark_run_id == benchmark_run_id)
        {
            *active = None;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CycleWaitBudget {
    timeout_count: u8,
}

impl CycleWaitBudget {
    fn new() -> Self {
        Self { timeout_count: 0 }
    }

    fn receive_timeout(self) -> Duration {
        if self.timeout_count == 0 {
            CYCLE_EVENT_TIMEOUT
        } else {
            CYCLE_IDLE_AFTER_CANCEL_TIMEOUT
        }
    }

    fn record_timeout(&mut self) -> bool {
        self.timeout_count = self.timeout_count.saturating_add(1);
        self.timeout_count < MAX_CYCLE_TIMEOUTS
    }
}

struct RunGuard {
    app: tauri::AppHandle,
    benchmark_run_id: u64,
    coordinator: Arc<crate::benchmark::BenchmarkCoordinator>,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        self.app
            .state::<State>()
            .microphone_startup_benchmark
            .finish(self.benchmark_run_id);
        self.coordinator.finish();
    }
}

fn require_main_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err(
            "Microphone startup benchmarking is available only from the main Settings window"
                .to_string(),
        )
    }
}

fn require_app_enabled(disabled: bool) -> Result<(), String> {
    if disabled {
        Err("Enable Murmur before testing the microphone".to_string())
    } else {
        Ok(())
    }
}

fn normalized_request(
    request: MicrophoneStartupBenchmarkRequest,
) -> Result<(String, Option<String>, DeviceSelection, u8), String> {
    if request.run_id.is_empty()
        || request.run_id.len() > MAX_RUN_ID_BYTES
        || uuid::Uuid::parse_str(&request.run_id).is_err()
    {
        return Err("The microphone benchmark run identifier is invalid".to_string());
    }
    if !(1..=MAX_CYCLES).contains(&request.cycles) {
        return Err(format!(
            "Microphone startup cycles must be between 1 and {MAX_CYCLES}"
        ));
    }
    if request.device_id.len() > MAX_DEVICE_ID_BYTES || request.device_id.is_empty() {
        return Err("The selected microphone identifier is invalid".to_string());
    }
    let (device_id, selection) = if request.device_id == "system_default" {
        (None, DeviceSelection::SystemDefault)
    } else {
        // Missing/stale pinned IDs deliberately reach the production helper so
        // the report can diagnose bounded device re-resolution.
        (Some(request.device_id), DeviceSelection::Pinned)
    };
    Ok((request.run_id, device_id, selection, request.cycles))
}

fn startup_backend(value: CaptureBackend) -> StartupBackend {
    match value {
        CaptureBackend::Auhal => StartupBackend::Auhal,
        CaptureBackend::Cpal => StartupBackend::Cpal,
    }
}

fn startup_transition(value: ProtocolSetupTransition) -> SetupTransition {
    match value {
        ProtocolSetupTransition::Entered => SetupTransition::Entered,
        ProtocolSetupTransition::Completed => SetupTransition::Completed,
    }
}

fn startup_failure_kind(value: AudioFailureKind) -> StartupFailureKind {
    match value {
        AudioFailureKind::PermissionDenied => StartupFailureKind::PermissionDenied,
        AudioFailureKind::DeviceUnavailable => StartupFailureKind::DeviceUnavailable,
        AudioFailureKind::HostUnavailable => StartupFailureKind::HostUnavailable,
        AudioFailureKind::InvalidInput => StartupFailureKind::InvalidInput,
        AudioFailureKind::ResourceExhausted => StartupFailureKind::ResourceExhausted,
        AudioFailureKind::StreamInvalidated => StartupFailureKind::StreamInvalidated,
        AudioFailureKind::UnsupportedConfig => StartupFailureKind::UnsupportedConfig,
        AudioFailureKind::BackendError => StartupFailureKind::BackendError,
        AudioFailureKind::ProtocolError => StartupFailureKind::ProtocolError,
        AudioFailureKind::FirstBufferTimeout => StartupFailureKind::FirstBufferTimeout,
        AudioFailureKind::InitializationTimeout => StartupFailureKind::InitializationTimeout,
        AudioFailureKind::PermissionPromptTimeout => StartupFailureKind::PermissionPromptTimeout,
        AudioFailureKind::TerminationUnconfirmed => StartupFailureKind::TerminationUnconfirmed,
        AudioFailureKind::WorkerPanicked => StartupFailureKind::WorkerPanicked,
        AudioFailureKind::SignatureInvalid => StartupFailureKind::SignatureInvalid,
    }
}

fn startup_failure_phase(value: AudioInitPhase) -> StartupFailurePhase {
    match value {
        AudioInitPhase::DeviceEnumeration => StartupFailurePhase::DeviceEnumeration,
        AudioInitPhase::StreamBuild => StartupFailurePhase::StreamBuild,
        AudioInitPhase::FirstBufferWait => StartupFailurePhase::FirstBufferWait,
        AudioInitPhase::Runtime => StartupFailurePhase::Runtime,
    }
}

fn startup_setup_step(value: CaptureSetupStep) -> StartupSetupStep {
    match value {
        CaptureSetupStep::DeviceResolution => StartupSetupStep::DeviceResolution,
        CaptureSetupStep::AudioUnitCreation => StartupSetupStep::AudioUnitCreation,
        CaptureSetupStep::AudioUnitNew => StartupSetupStep::AudioUnitNew,
        CaptureSetupStep::EnableInputIo => StartupSetupStep::EnableInputIo,
        CaptureSetupStep::DisableOutputIo => StartupSetupStep::DisableOutputIo,
        CaptureSetupStep::SetCurrentDevice => StartupSetupStep::SetCurrentDevice,
        CaptureSetupStep::FormatConfiguration => StartupSetupStep::FormatConfiguration,
        CaptureSetupStep::CallbackInstallation => StartupSetupStep::CallbackInstallation,
        CaptureSetupStep::DefaultConfig => StartupSetupStep::DefaultConfig,
        CaptureSetupStep::StreamBuild => StartupSetupStep::StreamBuild,
        CaptureSetupStep::StreamStart => StartupSetupStep::StreamStart,
        CaptureSetupStep::AwaitingFirstCallback => StartupSetupStep::AwaitingFirstCallback,
        CaptureSetupStep::SystemTapCreate => StartupSetupStep::SystemTapCreate,
        CaptureSetupStep::AggregateDeviceCreate => StartupSetupStep::AggregateDeviceCreate,
        CaptureSetupStep::IoProcCreate => StartupSetupStep::IoProcCreate,
        CaptureSetupStep::IoProcStart => StartupSetupStep::IoProcStart,
    }
}

struct CycleAccumulator {
    cycle: u8,
    backend_order: Vec<StartupBackend>,
    backend_order_source: Option<BackendOrderSource>,
    attempts: Vec<MicrophoneStartupAttemptResult>,
    cycle_start_to_first_pcm_ms: Option<u64>,
    backend: Option<StartupBackend>,
    failure_kind: Option<StartupFailureKind>,
    contract_violation: bool,
}

impl CycleAccumulator {
    fn new(cycle: u8) -> Self {
        Self {
            cycle,
            backend_order: Vec::new(),
            backend_order_source: None,
            attempts: Vec::new(),
            cycle_start_to_first_pcm_ms: None,
            backend: None,
            failure_kind: None,
            contract_violation: false,
        }
    }

    fn with_plan(cycle: u8, plan: AudioStartupDiagnostic) -> Self {
        let mut accumulator = Self::new(cycle);
        accumulator.observe(plan);
        accumulator
    }

    fn attempt_mut(
        &mut self,
        resolution_pass: u8,
        attempt_index: u8,
    ) -> Option<&mut MicrophoneStartupAttemptResult> {
        self.attempts.iter_mut().find(|attempt| {
            attempt.resolution_pass == resolution_pass && attempt.attempt_index == attempt_index
        })
    }

    fn observe(&mut self, diagnostic: AudioStartupDiagnostic) {
        match diagnostic {
            AudioStartupDiagnostic::BackendPlan {
                primary,
                fallback,
                source,
            } => {
                let observed_order = vec![startup_backend(primary), startup_backend(fallback)];
                let observed_source = match source {
                    AudioBackendOrderSource::Default => Some(BackendOrderSource::Default),
                    AudioBackendOrderSource::SessionFirstPcmMemo => {
                        Some(BackendOrderSource::SessionFirstPcmMemo)
                    }
                };
                if self.backend_order.is_empty() {
                    self.backend_order = observed_order;
                    self.backend_order_source = observed_source;
                } else if self.backend_order != observed_order
                    || self.backend_order_source != observed_source
                {
                    self.contract_violation = true;
                    self.failure_kind = Some(StartupFailureKind::ProtocolError);
                }
            }
            AudioStartupDiagnostic::AttemptStarted {
                backend,
                resolution_pass,
                attempt_index,
                attempt_budget_ms,
            } => {
                if self.attempts.len() < 6 {
                    self.attempts.push(MicrophoneStartupAttemptResult {
                        resolution_pass,
                        attempt_index,
                        backend: startup_backend(backend),
                        outcome: AttemptOutcome::Cancelled,
                        attempt_start_to_first_pcm_ms: None,
                        active_elapsed_ms: None,
                        failure_kind: None,
                        failure_phase: None,
                        last_setup_step: None,
                        last_setup_transition: None,
                        attempt_budget_ms,
                    });
                }
            }
            AudioStartupDiagnostic::SetupStep {
                resolution_pass,
                attempt_index,
                step,
                transition,
                ..
            } => {
                if let Some(attempt) = self.attempt_mut(resolution_pass, attempt_index) {
                    attempt.last_setup_step = Some(startup_setup_step(step));
                    attempt.last_setup_transition = Some(startup_transition(transition));
                }
            }
            AudioStartupDiagnostic::FirstPcm {
                backend,
                resolution_pass,
                attempt_index,
                attempt_start_to_first_pcm_ms,
                active_elapsed_ms,
            } => {
                self.backend = Some(startup_backend(backend));
                if let Some(attempt) = self.attempt_mut(resolution_pass, attempt_index) {
                    attempt.outcome = AttemptOutcome::Ready;
                    attempt.attempt_start_to_first_pcm_ms = Some(attempt_start_to_first_pcm_ms);
                    attempt.active_elapsed_ms = Some(active_elapsed_ms);
                }
            }
            AudioStartupDiagnostic::CycleReady {
                cycle_start_to_first_pcm_ms,
            } => self.cycle_start_to_first_pcm_ms = Some(cycle_start_to_first_pcm_ms),
            AudioStartupDiagnostic::AttemptFailed {
                resolution_pass,
                attempt_index,
                active_elapsed_ms,
                failure_kind,
                failure_phase,
                ..
            } => {
                if let Some(attempt) = self.attempt_mut(resolution_pass, attempt_index) {
                    // The FIFO publishes FirstPcm before lifecycle Ready. Once
                    // that startup measurement is complete, later runtime/stop
                    // failure is teardown evidence rather than a failed startup
                    // attempt and must not overwrite the winning timing.
                    if attempt.outcome != AttemptOutcome::Ready {
                        attempt.outcome = AttemptOutcome::Failed;
                        attempt.active_elapsed_ms = Some(active_elapsed_ms);
                        attempt.failure_kind = Some(startup_failure_kind(failure_kind));
                        attempt.failure_phase = Some(startup_failure_phase(failure_phase));
                    }
                }
            }
        }
    }

    fn finish(mut self, outcome: CycleOutcome) -> MicrophoneStartupCycleResult {
        match outcome {
            CycleOutcome::Ready => {
                // Runtime/stop noise after lifecycle Ready does not change the
                // completed startup measurement.
                self.failure_kind = None;
            }
            CycleOutcome::Failed | CycleOutcome::Cancelled => {
                // FirstPcm is deliberately observed before lifecycle Ready.
                // If Ready never follows, that provisional observation is not
                // a completed cycle and must not leak into the report.
                self.backend = None;
                self.cycle_start_to_first_pcm_ms = None;
                for attempt in &mut self.attempts {
                    if attempt.outcome == AttemptOutcome::Ready {
                        attempt.outcome = AttemptOutcome::Cancelled;
                        attempt.attempt_start_to_first_pcm_ms = None;
                        attempt.active_elapsed_ms = None;
                    }
                }
                if outcome == CycleOutcome::Failed && self.failure_kind.is_none() {
                    self.failure_kind = Some(StartupFailureKind::BackendError);
                }
                if outcome == CycleOutcome::Cancelled {
                    self.failure_kind = None;
                }
            }
        }
        let last = self.attempts.last();
        MicrophoneStartupCycleResult {
            cycle: self.cycle,
            outcome,
            cycle_start_to_first_pcm_ms: self.cycle_start_to_first_pcm_ms,
            backend: self.backend,
            fallback_occurred: self
                .attempts
                .iter()
                .any(|attempt| attempt.attempt_index > 1),
            failure_kind: self.failure_kind,
            last_setup_step: last.and_then(|attempt| attempt.last_setup_step),
            last_setup_transition: last.and_then(|attempt| attempt.last_setup_transition),
            backend_order: self.backend_order,
            backend_order_source: self.backend_order_source,
            attempts: self.attempts,
        }
    }
}

fn terminal_cycle_outcome(ready: bool, failed: bool, run_cancelled: bool) -> CycleOutcome {
    if ready {
        CycleOutcome::Ready
    } else if failed {
        CycleOutcome::Failed
    } else if run_cancelled {
        CycleOutcome::Cancelled
    } else {
        CycleOutcome::Failed
    }
}

fn report_is_cancelled(
    run_cancelled: bool,
    completed_cycles: usize,
    requested_cycles: u8,
    last_outcome: Option<CycleOutcome>,
) -> bool {
    run_cancelled
        && (completed_cycles < requested_cycles as usize
            || last_outcome == Some(CycleOutcome::Cancelled))
}

// Keep the event's correlation, bounded counters, phase, and exact accumulator
// explicit at call sites; bundling them would hide stale-run mistakes.
#[allow(clippy::too_many_arguments)]
fn emit_progress(
    app: &tauri::AppHandle,
    run_id: &str,
    benchmark_run_id: u64,
    completed_cycles: u8,
    total_cycles: u8,
    current_cycle: u8,
    phase: ProgressPhase,
    accumulator: &CycleAccumulator,
) {
    let last = accumulator.attempts.last();
    let _ = app.emit_to(
        "main",
        "microphone-startup-benchmark-progress",
        MicrophoneStartupBenchmarkProgress {
            schema_version: REPORT_SCHEMA_VERSION,
            run_id,
            benchmark_run_id,
            completed_cycles,
            total_cycles,
            current_cycle,
            phase,
            backend: last.map(|attempt| attempt.backend),
            fallback_occurred: accumulator
                .attempts
                .iter()
                .any(|attempt| attempt.attempt_index > 1),
            last_setup_step: last.and_then(|attempt| attempt.last_setup_step),
            last_setup_transition: last.and_then(|attempt| attempt.last_setup_transition),
        },
    );
}

pub(crate) fn handle_audio_lifecycle(
    app_handle: tauri::AppHandle,
    cycle_id: u64,
    event: AudioLifecycleEvent,
) {
    let state = app_handle.state::<State>();
    let event = match event {
        AudioLifecycleEvent::StartupDiagnostic(diagnostic) => {
            Some(CycleEvent::Diagnostic(diagnostic))
        }
        AudioLifecycleEvent::Ready => Some(CycleEvent::Ready),
        AudioLifecycleEvent::InitializationFailed { kind, .. } => Some(CycleEvent::Failed {
            kind: startup_failure_kind(kind),
        }),
        AudioLifecycleEvent::Recovering { .. } | AudioLifecycleEvent::RecoveryStalled => {
            Some(CycleEvent::Recovering)
        }
        AudioLifecycleEvent::Interrupted { .. } => Some(CycleEvent::Failed {
            kind: StartupFailureKind::StreamInvalidated,
        }),
        AudioLifecycleEvent::Idle => Some(CycleEvent::Idle),
        AudioLifecycleEvent::Accepted | AudioLifecycleEvent::StillConnecting => None,
    };
    if let Some(event) = event {
        state
            .microphone_startup_benchmark
            .send_if_current(cycle_id, event);
    }
}

// Capture ownership, external/internal generations, immutable selection, and
// the production backend snapshot are intentionally separate inputs.
#[allow(clippy::too_many_arguments)]
fn run_cycles(
    app: tauri::AppHandle,
    coordinator: Arc<crate::benchmark::BenchmarkCoordinator>,
    run_id: String,
    benchmark_run_id: u64,
    device_id: Option<String>,
    selection: DeviceSelection,
    requested_cycles: u8,
    backend_plan: AudioStartupDiagnostic,
) -> Result<MicrophoneStartupBenchmarkReport, String> {
    let _guard = RunGuard {
        app: app.clone(),
        benchmark_run_id,
        coordinator,
    };
    let started_at = chrono::Utc::now().to_rfc3339();
    let mut cycles = Vec::with_capacity(requested_cycles as usize);

    for cycle_number in 1..=requested_cycles {
        let state = app.state::<State>();
        if state
            .microphone_startup_benchmark
            .is_cancelled(benchmark_run_id)
        {
            break;
        }
        let Some((cycle_id, receiver)) = state
            .microphone_startup_benchmark
            .begin_cycle(benchmark_run_id)
        else {
            break;
        };
        let mut accumulator = CycleAccumulator::with_plan(cycle_number, backend_plan);
        emit_progress(
            &app,
            &run_id,
            benchmark_run_id,
            cycles.len() as u8,
            requested_cycles,
            cycle_number,
            ProgressPhase::Starting,
            &accumulator,
        );

        let start_result = audio_lifecycle::start_microphone_benchmark_recording(
            app.clone(),
            device_id.clone(),
            cycle_id,
        );
        if let Err(error) = start_result {
            state
                .microphone_startup_benchmark
                .finish_cycle(benchmark_run_id, cycle_id);
            return Err(format!(
                "Could not start the microphone startup benchmark capture: {error}"
            ));
        }

        let mut ready = false;
        let mut cancelled = false;
        let mut idle = false;
        let mut wait_budget = CycleWaitBudget::new();
        while !idle {
            match receiver.recv_timeout(wait_budget.receive_timeout()) {
                Ok(CycleEvent::Diagnostic(diagnostic)) => {
                    accumulator.observe(diagnostic);
                    if accumulator.contract_violation {
                        let _ = audio_lifecycle::cancel_microphone_benchmark_capture(
                            cycle_id,
                            AudioCancelReason::RuntimeFailure,
                        );
                    }
                    emit_progress(
                        &app,
                        &run_id,
                        benchmark_run_id,
                        cycles.len() as u8,
                        requested_cycles,
                        cycle_number,
                        ProgressPhase::Starting,
                        &accumulator,
                    );
                }
                Ok(CycleEvent::Ready) => {
                    ready = true;
                    emit_progress(
                        &app,
                        &run_id,
                        benchmark_run_id,
                        cycles.len() as u8,
                        requested_cycles,
                        cycle_number,
                        ProgressPhase::Capturing,
                        &accumulator,
                    );
                    cancelled = app
                        .state::<State>()
                        .microphone_startup_benchmark
                        .is_cancelled(benchmark_run_id);
                    if cancelled {
                        let _ = audio_lifecycle::cancel_microphone_benchmark_capture(
                            cycle_id,
                            AudioCancelReason::User,
                        );
                    } else {
                        emit_progress(
                            &app,
                            &run_id,
                            benchmark_run_id,
                            cycles.len() as u8,
                            requested_cycles,
                            cycle_number,
                            ProgressPhase::Stopping,
                            &accumulator,
                        );
                        let _ = audio_lifecycle::stop_microphone_benchmark_recording(cycle_id);
                    }
                }
                Ok(CycleEvent::Failed { kind }) => {
                    accumulator.failure_kind = Some(kind);
                    emit_progress(
                        &app,
                        &run_id,
                        benchmark_run_id,
                        cycles.len() as u8,
                        requested_cycles,
                        cycle_number,
                        ProgressPhase::Waiting,
                        &accumulator,
                    );
                }
                Ok(CycleEvent::Recovering) => {
                    emit_progress(
                        &app,
                        &run_id,
                        benchmark_run_id,
                        cycles.len() as u8,
                        requested_cycles,
                        cycle_number,
                        ProgressPhase::Recovering,
                        &accumulator,
                    );
                }
                Ok(CycleEvent::Idle) => idle = true,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    cancelled = app
                        .state::<State>()
                        .microphone_startup_benchmark
                        .is_cancelled(benchmark_run_id);
                    let cancel_reason = if cancelled {
                        AudioCancelReason::User
                    } else {
                        AudioCancelReason::HardDeadline
                    };
                    let _ = audio_lifecycle::cancel_microphone_benchmark_capture(
                        cycle_id,
                        cancel_reason,
                    );
                    if !cancelled {
                        accumulator.failure_kind = Some(StartupFailureKind::InitializationTimeout);
                    }
                    emit_progress(
                        &app,
                        &run_id,
                        benchmark_run_id,
                        cycles.len() as u8,
                        requested_cycles,
                        cycle_number,
                        ProgressPhase::Recovering,
                        &accumulator,
                    );
                    if !wait_budget.record_timeout() {
                        return Err(
                            "The microphone benchmark capture did not reach idle after cancellation"
                                .to_string(),
                        );
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = audio_lifecycle::cancel_microphone_benchmark_capture(
                        cycle_id,
                        AudioCancelReason::RuntimeFailure,
                    );
                    while audio_lifecycle::is_microphone_benchmark_owner(cycle_id) {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    return Err(
                        "Microphone benchmark lifecycle channel closed before post-join Idle"
                            .to_string(),
                    );
                }
            }
        }
        state
            .microphone_startup_benchmark
            .finish_cycle(benchmark_run_id, cycle_id);
        if accumulator.contract_violation {
            return Err(
                "The production microphone backend plan changed during a benchmark cycle"
                    .to_string(),
            );
        }
        let run_cancelled = cancelled
            || state
                .microphone_startup_benchmark
                .is_cancelled(benchmark_run_id);
        let outcome =
            terminal_cycle_outcome(ready, accumulator.failure_kind.is_some(), run_cancelled);
        cycles.push(accumulator.finish(outcome));
        if run_cancelled {
            break;
        }
    }

    let run_cancelled = app
        .state::<State>()
        .microphone_startup_benchmark
        .is_cancelled(benchmark_run_id);
    let cancelled = report_is_cancelled(
        run_cancelled,
        cycles.len(),
        requested_cycles,
        cycles.last().map(|cycle| cycle.outcome),
    );
    let empty = CycleAccumulator::new(cycles.len().max(1) as u8);
    emit_progress(
        &app,
        &run_id,
        benchmark_run_id,
        cycles.len() as u8,
        requested_cycles,
        // A cancellation claimed before cycle one has no current cycle. Keep
        // that exact zero instead of manufacturing cycle one in the terminal
        // progress payload.
        cycles.len() as u8,
        ProgressPhase::Complete,
        &empty,
    );
    let report = MicrophoneStartupBenchmarkReport {
        schema_version: REPORT_SCHEMA_VERSION,
        run_id,
        benchmark_run_id,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        platform: std::env::consts::OS.to_string(),
        device_selection: selection,
        requested_cycles,
        // A cancelled in-flight cycle is included as a terminal partial result.
        completed_cycles: cycles.len() as u8,
        cancelled,
        started_at,
        finished_at: chrono::Utc::now().to_rfc3339(),
        cycles,
    };
    validate_report(&report)?;
    Ok(report)
}

#[tauri::command]
pub async fn run_microphone_startup_benchmark(
    window: tauri::WebviewWindow,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    request: MicrophoneStartupBenchmarkRequest,
) -> Result<MicrophoneStartupBenchmarkReport, String> {
    require_main_window(&window)?;
    let (run_id, device_id, selection, cycles) = normalized_request(request)?;
    let transition =
        super::microphone_preview::transition_after_stopping_preview(&app_handle, state.inner())
            .await?;
    let coordinator = state.benchmark.clone();
    {
        let dictation = state.app_state.dictation.lock_or_recover();
        if state.app_state.meeting_blocks_asr() {
            return Err(
                "Wait for the meeting transcript to finish before testing the microphone"
                    .to_string(),
            );
        }
        if state.app_state.transform_status().blocks_recording()
            || state.transform_runtime.is_transform_busy()
        {
            return Err(
                "Wait for the transform to finish before testing the microphone".to_string(),
            );
        }
        if state.query.status().blocks_pipeline() {
            return Err(
                "Wait for the voice query to finish before testing the microphone".to_string(),
            );
        }
        if dictation.status != DictationStatus::Idle {
            return Err("Stop recording before testing the microphone".to_string());
        }
        if state.app_state.file_transcribing.load(Ordering::SeqCst) {
            return Err("Wait for the file transcription to finish".to_string());
        }
        #[cfg(feature = "internal-benchmark")]
        if state.corpus.is_active() {
            return Err("Finish the corpus recording before testing the microphone".to_string());
        }
        if audio_lifecycle::is_audio_active() {
            return Err(
                "The microphone is still in use or recovering. Wait for Murmur to become ready."
                    .to_string(),
            );
        }
        require_app_enabled(crate::keyboard::is_app_disabled())?;
        if !coordinator.try_start() {
            return Err(if coordinator.is_running() {
                "A microphone startup benchmark is already running".to_string()
            } else {
                "Wait for model preparation to finish before testing the microphone".to_string()
            });
        }
    }
    let benchmark_run_id = match state.microphone_startup_benchmark.claim(run_id.clone()) {
        Ok(id) => id,
        Err(error) => {
            coordinator.finish();
            return Err(error);
        }
    };
    let backend_plan = crate::audio::microphone_startup_backend_plan(device_id.as_deref());
    drop(transition);
    tokio::task::spawn_blocking(move || {
        run_cycles(
            app_handle,
            coordinator,
            run_id,
            benchmark_run_id,
            device_id,
            selection,
            cycles,
            backend_plan,
        )
    })
    .await
    .map_err(|error| format!("Microphone startup benchmark task failed: {error}"))?
}

#[tauri::command]
pub async fn cancel_microphone_startup_benchmark(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, State>,
    run_id: String,
) -> Result<bool, String> {
    require_main_window(&window)?;
    if run_id.len() > MAX_RUN_ID_BYTES {
        return Ok(false);
    }
    let Some((_benchmark_run_id, cycle_id)) =
        state.microphone_startup_benchmark.cancel_exact(&run_id)
    else {
        return Ok(false);
    };
    state.benchmark.cancel();
    if let Some(cycle_id) = cycle_id {
        tokio::task::spawn_blocking(move || {
            audio_lifecycle::cancel_microphone_benchmark_capture(cycle_id, AudioCancelReason::User)
        })
        .await
        .map_err(|error| format!("Microphone benchmark cancel task failed: {error}"))??;
    }
    Ok(true)
}

fn validate_report(report: &MicrophoneStartupBenchmarkReport) -> Result<(), String> {
    let started_at = chrono::DateTime::parse_from_rfc3339(&report.started_at).ok();
    let finished_at = chrono::DateTime::parse_from_rfc3339(&report.finished_at).ok();
    let app_version_valid = valid_semver_provenance(&report.app_version);
    if report.schema_version != REPORT_SCHEMA_VERSION
        || report.run_id.is_empty()
        || report.run_id.len() > MAX_RUN_ID_BYTES
        || uuid::Uuid::parse_str(&report.run_id).is_err()
        || report.benchmark_run_id == 0
        || !(1..=MAX_CYCLES).contains(&report.requested_cycles)
        || report.completed_cycles as usize != report.cycles.len()
        || report.completed_cycles > report.requested_cycles
        || report.cycles.iter().any(|cycle| cycle.attempts.len() > 6)
        || (!report.cancelled && report.completed_cycles != report.requested_cycles)
        || (!report.cancelled
            && report
                .cycles
                .iter()
                .any(|cycle| cycle.outcome == CycleOutcome::Cancelled))
        || (report.cancelled
            && report.completed_cycles == report.requested_cycles
            && report
                .cycles
                .last()
                .is_none_or(|cycle| cycle.outcome != CycleOutcome::Cancelled))
        || !app_version_valid
        || report.platform != std::env::consts::OS
        || report.started_at.len() > 64
        || report.finished_at.len() > 64
        || started_at.is_none()
        || finished_at.is_none()
        || finished_at < started_at
    {
        return Err("The microphone startup benchmark report is invalid".to_string());
    }
    let frozen_plan = report
        .cycles
        .first()
        .map(|cycle| (cycle.backend_order.as_slice(), cycle.backend_order_source));
    for (index, cycle) in report.cycles.iter().enumerate() {
        let last = cycle.attempts.last();
        let ready_attempts = cycle
            .attempts
            .iter()
            .filter(|attempt| attempt.outcome == AttemptOutcome::Ready)
            .collect::<Vec<_>>();
        let fallback_occurred = cycle
            .attempts
            .iter()
            .any(|attempt| attempt.attempt_index == 2);
        let attempts_valid =
            cycle
                .attempts
                .iter()
                .enumerate()
                .all(|(attempt_position, attempt)| {
                    let expected_backend = cycle
                        .backend_order
                        .get(match attempt.attempt_index.checked_sub(1) {
                            Some(index) => index as usize,
                            None => return false,
                        })
                        .copied();
                    let key_is_monotonic = attempt_position == 0
                        || cycle.attempts[attempt_position - 1].resolution_pass
                            < attempt.resolution_pass
                        || (cycle.attempts[attempt_position - 1].resolution_pass
                            == attempt.resolution_pass
                            && cycle.attempts[attempt_position - 1].attempt_index
                                < attempt.attempt_index);
                    let outcome_valid = match attempt.outcome {
                        AttemptOutcome::Ready => {
                            attempt.attempt_start_to_first_pcm_ms.is_some()
                                && attempt.active_elapsed_ms.is_some()
                                && attempt.failure_kind.is_none()
                                && attempt.failure_phase.is_none()
                        }
                        AttemptOutcome::Failed => {
                            attempt.attempt_start_to_first_pcm_ms.is_none()
                                && attempt.active_elapsed_ms.is_some()
                                && attempt.failure_kind.is_some()
                                && attempt.failure_phase.is_some()
                        }
                        AttemptOutcome::Cancelled => {
                            attempt.attempt_start_to_first_pcm_ms.is_none()
                                && attempt.active_elapsed_ms.is_none()
                                && attempt.failure_kind.is_none()
                                && attempt.failure_phase.is_none()
                        }
                    };
                    (1..=3).contains(&attempt.resolution_pass)
                        && (1..=2).contains(&attempt.attempt_index)
                        && key_is_monotonic
                        && expected_backend == Some(attempt.backend)
                        && attempt
                            .active_elapsed_ms
                            .is_none_or(|value| value <= 180_000)
                        && (1..=60_000).contains(&attempt.attempt_budget_ms)
                        && attempt
                            .attempt_start_to_first_pcm_ms
                            .is_none_or(|value| value <= 60_000)
                        && attempt.last_setup_step.is_some()
                            == attempt.last_setup_transition.is_some()
                        && outcome_valid
                });
        let cycle_outcome_valid = match cycle.outcome {
            CycleOutcome::Ready => {
                cycle.cycle_start_to_first_pcm_ms.is_some()
                    && cycle.backend.is_some()
                    && cycle.failure_kind.is_none()
                    && ready_attempts.len() == 1
                    && ready_attempts[0].backend == cycle.backend.unwrap()
                    && cycle
                        .cycle_start_to_first_pcm_ms
                        .is_some_and(|cycle_latency| {
                            ready_attempts[0]
                                .attempt_start_to_first_pcm_ms
                                .is_some_and(|attempt_latency| cycle_latency >= attempt_latency)
                        })
            }
            CycleOutcome::Failed => {
                cycle.cycle_start_to_first_pcm_ms.is_none()
                    && cycle.backend.is_none()
                    && cycle.failure_kind.is_some()
                    && ready_attempts.is_empty()
            }
            CycleOutcome::Cancelled => {
                cycle.cycle_start_to_first_pcm_ms.is_none()
                    && cycle.backend.is_none()
                    && cycle.failure_kind.is_none()
                    && ready_attempts.is_empty()
            }
        };
        if cycle.cycle as usize != index + 1
            || cycle.backend_order.len() != 2
            || cycle.backend_order[0] == cycle.backend_order[1]
            || cycle.backend_order_source.is_none()
            || frozen_plan.is_some_and(|(backend_order, source)| {
                cycle.backend_order.as_slice() != backend_order
                    || cycle.backend_order_source != source
            })
            || cycle
                .cycle_start_to_first_pcm_ms
                .is_some_and(|value| value > 180_000)
            || cycle.fallback_occurred != fallback_occurred
            || cycle.last_setup_step != last.and_then(|attempt| attempt.last_setup_step)
            || cycle.last_setup_transition != last.and_then(|attempt| attempt.last_setup_transition)
            || !attempts_valid
            || !cycle_outcome_valid
        {
            return Err("The microphone startup benchmark report is invalid".to_string());
        }
    }
    Ok(())
}

fn valid_semver_provenance(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-'))
    {
        return false;
    }
    if value.bytes().filter(|byte| *byte == b'+').count() > 1 {
        return false;
    }
    let (without_build, build) = value
        .split_once('+')
        .map_or((value, None), |(base, build)| (base, Some(build)));
    if build.is_some_and(|build| {
        build.is_empty()
            || build
                .split('.')
                .any(|identifier| !valid_semver_identifier(identifier))
    }) {
        return false;
    }
    let (core, prerelease) = without_build
        .split_once('-')
        .map_or((without_build, None), |(core, pre)| (core, Some(pre)));
    if prerelease.is_some_and(|pre| {
        pre.is_empty()
            || pre.split('.').any(|identifier| {
                !valid_semver_identifier(identifier)
                    || (identifier.bytes().all(|byte| byte.is_ascii_digit())
                        && identifier.len() > 1
                        && identifier.starts_with('0'))
            })
    }) {
        return false;
    }
    let parts = core.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (part == &"0" || !part.starts_with('0'))
        })
}

fn valid_semver_identifier(identifier: &str) -> bool {
    !identifier.is_empty()
        && identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[tauri::command]
pub fn save_microphone_startup_benchmark_report(
    window: tauri::WebviewWindow,
    report: MicrophoneStartupBenchmarkReport,
    output_dir: String,
) -> Result<String, String> {
    require_main_window(&window)?;
    write_report(&report, &output_dir)
}

fn write_report(
    report: &MicrophoneStartupBenchmarkReport,
    output_dir: &str,
) -> Result<String, String> {
    validate_report(report)?;
    let json = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("Could not serialize microphone benchmark report: {error}"))?;
    if json.len() > MAX_REPORT_BYTES {
        return Err("The microphone startup benchmark report is too large".to_string());
    }
    let started = chrono::DateTime::parse_from_rfc3339(&report.started_at)
        .map_err(|_| "The microphone startup benchmark report is invalid".to_string())?
        .with_timezone(&chrono::Utc)
        .format("%Y%m%dT%H%M%S%.3fZ");
    let file_name = format!("murmur-microphone-startup-{started}-{}.json", report.run_id);
    let path = crate::file_output::resolve_output_dir(output_dir)?.join(file_name);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                "This microphone startup benchmark report was already saved".to_string()
            } else {
                format!("Failed to create microphone startup benchmark report: {error}")
            }
        })?;
    file.write_all(json.as_bytes())
        .map_err(|error| format!("Failed to write microphone startup benchmark report: {error}"))?;
    tracing::info!(
        target: "pipeline",
        bytes = json.len(),
        "microphone startup benchmark report written to file"
    );
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_report() -> MicrophoneStartupBenchmarkReport {
        MicrophoneStartupBenchmarkReport {
            schema_version: REPORT_SCHEMA_VERSION,
            run_id: uuid::Uuid::new_v4().to_string(),
            benchmark_run_id: 1,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            platform: std::env::consts::OS.to_string(),
            device_selection: DeviceSelection::SystemDefault,
            requested_cycles: 1,
            completed_cycles: 1,
            cancelled: false,
            started_at: "2026-08-14T12:00:00Z".to_string(),
            finished_at: "2026-08-14T12:00:01Z".to_string(),
            cycles: vec![MicrophoneStartupCycleResult {
                cycle: 1,
                outcome: CycleOutcome::Ready,
                cycle_start_to_first_pcm_ms: Some(12),
                backend: Some(StartupBackend::Auhal),
                fallback_occurred: false,
                failure_kind: None,
                last_setup_step: None,
                last_setup_transition: None,
                backend_order: vec![StartupBackend::Auhal, StartupBackend::Cpal],
                backend_order_source: Some(BackendOrderSource::Default),
                attempts: vec![MicrophoneStartupAttemptResult {
                    resolution_pass: 1,
                    attempt_index: 1,
                    backend: StartupBackend::Auhal,
                    outcome: AttemptOutcome::Ready,
                    attempt_start_to_first_pcm_ms: Some(10),
                    active_elapsed_ms: Some(10),
                    failure_kind: None,
                    failure_phase: None,
                    last_setup_step: None,
                    last_setup_transition: None,
                    attempt_budget_ms: 5_000,
                }],
            }],
        }
    }

    #[test]
    fn request_bounds_and_pinned_absence_are_preserved() {
        let request = MicrophoneStartupBenchmarkRequest {
            run_id: uuid::Uuid::new_v4().to_string(),
            device_id: "missing-stable-id".to_string(),
            cycles: 5,
        };
        let (_, device, selection, cycles) = normalized_request(request).unwrap();
        assert_eq!(device.as_deref(), Some("missing-stable-id"));
        assert_eq!(selection, DeviceSelection::Pinned);
        assert_eq!(cycles, 5);

        let invalid = MicrophoneStartupBenchmarkRequest {
            run_id: uuid::Uuid::new_v4().to_string(),
            device_id: "system_default".to_string(),
            cycles: 11,
        };
        assert!(normalized_request(invalid).is_err());
        assert_eq!(
            require_app_enabled(true).unwrap_err(),
            "Enable Murmur before testing the microphone"
        );
        assert!(require_app_enabled(false).is_ok());
    }

    #[test]
    fn stale_cancel_never_targets_the_current_generation() {
        let state = MicrophoneStartupBenchmarkState::default();
        let current = uuid::Uuid::new_v4().to_string();
        let generation = state.claim(current.clone()).unwrap();
        assert!(state
            .cancel_exact("00000000-0000-0000-0000-000000000000")
            .is_none());
        assert!(!state.is_cancelled(generation));
        assert!(state.cancel_exact(&current).is_some());
        assert!(state.is_cancelled(generation));
        assert_eq!(
            terminal_cycle_outcome(false, true, true),
            CycleOutcome::Failed,
            "a cancel during post-failure cleanup must not erase the terminal failure"
        );
        assert_eq!(
            terminal_cycle_outcome(true, false, true),
            CycleOutcome::Ready,
            "first PCM remains a completed measurement when cancellation races stop"
        );
        assert!(report_is_cancelled(
            true,
            1,
            1,
            Some(terminal_cycle_outcome(false, false, true))
        ));
        assert!(!report_is_cancelled(
            true,
            1,
            1,
            Some(terminal_cycle_outcome(true, false, true))
        ));
    }

    #[test]
    fn cycle_wait_budget_allows_one_bounded_post_cancel_grace() {
        let mut budget = CycleWaitBudget::new();
        assert_eq!(budget.receive_timeout(), CYCLE_EVENT_TIMEOUT);
        assert!(budget.record_timeout());
        assert_eq!(budget.receive_timeout(), CYCLE_IDLE_AFTER_CANCEL_TIMEOUT);
        assert!(!budget.record_timeout());
    }

    #[test]
    fn attempt_observations_remain_ordered_and_bounded() {
        let mut cycle = CycleAccumulator::new(1);
        cycle.observe(AudioStartupDiagnostic::BackendPlan {
            primary: CaptureBackend::Auhal,
            fallback: CaptureBackend::Cpal,
            source: AudioBackendOrderSource::Default,
        });
        cycle.observe(AudioStartupDiagnostic::AttemptStarted {
            backend: CaptureBackend::Auhal,
            resolution_pass: 1,
            attempt_index: 1,
            attempt_budget_ms: 5_000,
        });
        cycle.observe(AudioStartupDiagnostic::SetupStep {
            backend: CaptureBackend::Auhal,
            resolution_pass: 1,
            attempt_index: 1,
            step: CaptureSetupStep::StreamStart,
            transition: ProtocolSetupTransition::Entered,
        });
        cycle.observe(AudioStartupDiagnostic::AttemptFailed {
            backend: CaptureBackend::Auhal,
            resolution_pass: 1,
            attempt_index: 1,
            active_elapsed_ms: 4_999,
            failure_kind: AudioFailureKind::InitializationTimeout,
            failure_phase: AudioInitPhase::StreamBuild,
        });
        let result = cycle.finish(CycleOutcome::Failed);
        assert_eq!(
            result.backend_order,
            [StartupBackend::Auhal, StartupBackend::Cpal]
        );
        assert_eq!(result.attempts.len(), 1);
        assert_eq!(result.attempts[0].outcome, AttemptOutcome::Failed);
        assert_eq!(result.last_setup_step, Some(StartupSetupStep::StreamStart));
        assert_eq!(result.last_setup_transition, Some(SetupTransition::Entered));
    }

    #[test]
    fn cycle_rejects_a_worker_plan_that_differs_from_the_frozen_snapshot() {
        let plan = AudioStartupDiagnostic::BackendPlan {
            primary: CaptureBackend::Auhal,
            fallback: CaptureBackend::Cpal,
            source: AudioBackendOrderSource::Default,
        };
        let mut cycle = CycleAccumulator::with_plan(1, plan);
        cycle.observe(plan);
        assert!(!cycle.contract_violation);
        cycle.observe(AudioStartupDiagnostic::BackendPlan {
            primary: CaptureBackend::Cpal,
            fallback: CaptureBackend::Auhal,
            source: AudioBackendOrderSource::SessionFirstPcmMemo,
        });
        assert!(cycle.contract_violation);
        assert_eq!(
            cycle.backend_order,
            [StartupBackend::Auhal, StartupBackend::Cpal]
        );
        assert_eq!(cycle.failure_kind, Some(StartupFailureKind::ProtocolError));
    }

    #[test]
    fn provisional_pcm_is_not_reported_without_lifecycle_ready() {
        let plan = AudioStartupDiagnostic::BackendPlan {
            primary: CaptureBackend::Auhal,
            fallback: CaptureBackend::Cpal,
            source: AudioBackendOrderSource::Default,
        };
        let mut cycle = CycleAccumulator::with_plan(1, plan);
        cycle.observe(AudioStartupDiagnostic::AttemptStarted {
            backend: CaptureBackend::Auhal,
            resolution_pass: 1,
            attempt_index: 1,
            attempt_budget_ms: 5_000,
        });
        cycle.observe(AudioStartupDiagnostic::FirstPcm {
            backend: CaptureBackend::Auhal,
            resolution_pass: 1,
            attempt_index: 1,
            attempt_start_to_first_pcm_ms: 10,
            active_elapsed_ms: 12,
        });
        cycle.observe(AudioStartupDiagnostic::CycleReady {
            cycle_start_to_first_pcm_ms: 14,
        });
        let cancelled = cycle.finish(CycleOutcome::Cancelled);
        assert_eq!(cancelled.backend, None);
        assert_eq!(cancelled.cycle_start_to_first_pcm_ms, None);
        assert_eq!(cancelled.attempts[0].outcome, AttemptOutcome::Cancelled);
        assert_eq!(cancelled.attempts[0].attempt_start_to_first_pcm_ms, None);
        assert_eq!(cancelled.attempts[0].active_elapsed_ms, None);
        assert!(validate_report(&MicrophoneStartupBenchmarkReport {
            cycles: vec![cancelled],
            cancelled: true,
            ..valid_report()
        })
        .is_ok());
    }

    #[test]
    fn post_ready_failure_does_not_erase_a_completed_measurement() {
        let mut report = valid_report();
        let cycle = &mut report.cycles[0];
        let mut accumulator = CycleAccumulator::with_plan(
            1,
            AudioStartupDiagnostic::BackendPlan {
                primary: CaptureBackend::Auhal,
                fallback: CaptureBackend::Cpal,
                source: AudioBackendOrderSource::Default,
            },
        );
        accumulator.attempts = cycle.attempts.clone();
        accumulator.backend = cycle.backend;
        accumulator.cycle_start_to_first_pcm_ms = cycle.cycle_start_to_first_pcm_ms;
        accumulator.observe(AudioStartupDiagnostic::AttemptFailed {
            backend: CaptureBackend::Auhal,
            resolution_pass: 1,
            attempt_index: 1,
            active_elapsed_ms: 20,
            failure_kind: AudioFailureKind::StreamInvalidated,
            failure_phase: AudioInitPhase::Runtime,
        });
        accumulator.failure_kind = Some(StartupFailureKind::StreamInvalidated);
        *cycle = accumulator.finish(CycleOutcome::Ready);
        assert_eq!(cycle.failure_kind, None);
        assert_eq!(cycle.attempts[0].outcome, AttemptOutcome::Ready);
        assert_eq!(cycle.attempts[0].attempt_start_to_first_pcm_ms, Some(10));
        assert!(validate_report(&report).is_ok());
    }

    #[test]
    fn report_validator_enforces_cancelled_cycle_cross_fields() {
        let mut report = valid_report();
        report.cancelled = true;
        report.cycles[0].outcome = CycleOutcome::Cancelled;
        assert!(validate_report(&report).is_err());

        let mut normalized = valid_report();
        normalized.cancelled = true;
        normalized.cycles[0].outcome = CycleOutcome::Cancelled;
        normalized.cycles[0].cycle_start_to_first_pcm_ms = None;
        normalized.cycles[0].backend = None;
        normalized.cycles[0].attempts[0].outcome = AttemptOutcome::Cancelled;
        normalized.cycles[0].attempts[0].attempt_start_to_first_pcm_ms = None;
        normalized.cycles[0].attempts[0].active_elapsed_ms = None;
        assert!(validate_report(&normalized).is_ok());

        normalized.cancelled = false;
        assert!(validate_report(&normalized).is_err());

        let mut full_but_not_terminally_cancelled = valid_report();
        let mut ready_second = full_but_not_terminally_cancelled.cycles[0].clone();
        ready_second.cycle = 2;
        full_but_not_terminally_cancelled.requested_cycles = 2;
        full_but_not_terminally_cancelled.completed_cycles = 2;
        full_but_not_terminally_cancelled.cancelled = true;
        full_but_not_terminally_cancelled.cycles = vec![normalized.cycles[0].clone(), ready_second];
        assert!(validate_report(&full_but_not_terminally_cancelled).is_err());
    }

    #[test]
    fn report_validator_rejects_cycle_latency_below_the_winning_attempt() {
        let mut report = valid_report();
        report.cycles[0].cycle_start_to_first_pcm_ms = Some(9);
        assert!(validate_report(&report).is_err());
    }

    #[test]
    fn report_validator_rejects_zero_attempt_index_without_panicking() {
        let mut report = valid_report();
        report.cycles[0].attempts[0].attempt_index = 0;
        let validation = std::panic::catch_unwind(|| validate_report(&report));
        assert!(validation.is_ok());
        assert!(validation.unwrap().is_err());
    }

    #[test]
    fn report_validator_allows_wall_startup_to_exceed_active_elapsed_time() {
        let mut report = valid_report();
        report.cycles[0].attempts[0].active_elapsed_ms = Some(1);
        assert!(validate_report(&report).is_ok());
    }

    #[test]
    fn report_validator_requires_one_frozen_backend_plan_for_the_run() {
        let mut report = valid_report();
        let mut second = report.cycles[0].clone();
        second.cycle = 2;
        report.requested_cycles = 2;
        report.completed_cycles = 2;
        report.cycles.push(second);
        assert!(validate_report(&report).is_ok());

        report.cycles[1].backend_order_source = Some(BackendOrderSource::SessionFirstPcmMemo);
        assert!(validate_report(&report).is_err());
        report.cycles[1].backend_order_source = Some(BackendOrderSource::Default);
        report.cycles[1].backend_order.reverse();
        assert!(validate_report(&report).is_err());
    }

    #[test]
    fn historical_semver_provenance_remains_exportable() {
        let mut report = valid_report();
        report.app_version = "0.33.1-beta.2+build-7".to_string();
        assert!(validate_report(&report).is_ok());

        for invalid in [
            "SENTINEL_PRIVATE_MIC_NAME",
            "1.2.3+a+b",
            "1.2.3-01",
            "01.2.3",
        ] {
            report.app_version = invalid.to_string();
            assert!(validate_report(&report).is_err(), "accepted {invalid}");
        }
    }

    #[test]
    fn report_transport_rejects_unknown_fields_and_untyped_diagnostics() {
        let report = valid_report();
        assert!(validate_report(&report).is_ok());
        let mut value = serde_json::to_value(report).unwrap();
        value["cycles"][0]["attempts"][0]["failureKind"] =
            serde_json::Value::String("SENTINEL_PRIVATE_MIC_NAME".to_string());
        assert!(serde_json::from_value::<MicrophoneStartupBenchmarkReport>(value).is_err());

        let mut value = serde_json::to_value(valid_report()).unwrap();
        value["deviceName"] = serde_json::Value::String("SENTINEL_PRIVATE_MIC_NAME".to_string());
        assert!(serde_json::from_value::<MicrophoneStartupBenchmarkReport>(value).is_err());
    }

    #[test]
    fn report_export_never_overwrites_an_existing_run() {
        let report = valid_report();
        let dir = std::env::temp_dir().join(format!(
            "murmur-microphone-startup-report-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let first = write_report(&report, dir.to_str().unwrap()).unwrap();
        assert!(std::path::Path::new(&first).is_file());
        assert!(write_report(&report, dir.to_str().unwrap()).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
