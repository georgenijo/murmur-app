//! Event-driven, explicitly consented checks through the existing Preview owner.
//! The controller owns no audio thread or PCM. A revoked permit cannot be reused.

use crate::microphone_auto::{SmartAutoRequest, SmartAutoStatus};
use crate::microphone_signal::SignalVerificationResult;
use crate::{MutexExt, State};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{Emitter, Listener, Manager};
use tokio::sync::Notify;

pub(crate) const PROBE_LIMIT: Duration = Duration::from_secs(8);
const STARTUP_LIMIT: Duration = Duration::from_secs(3);
const ROUND_LIMIT: Duration = Duration::from_secs(75);
const ATTEMPT_AND_STOP_LIMIT: Duration = Duration::from_secs(23);
const MIN_GAP: Duration = Duration::from_secs(10);
const CANDIDATES_PER_ROUND: usize = 3;

/// This deadline is absolute. It must not pause for TCC, HAL, or backend retries.
pub(crate) struct ProbePermit {
    revoked: AtomicBool,
    deadline: Instant,
    deadline_ns: Option<u64>,
    changed: Notify,
}

impl ProbePermit {
    pub(crate) fn new(now: Instant) -> Self {
        let deadline = now + PROBE_LIMIT;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let deadline_ns = murmur_capture_helper_protocol::capture_monotonic_ns()
            .and_then(|now_ns| now_ns.checked_add(remaining.as_nanos() as u64));
        Self {
            revoked: AtomicBool::new(false),
            deadline,
            deadline_ns,
            changed: Notify::new(),
        }
    }

    pub(crate) fn is_valid(&self) -> bool {
        !self.remaining().is_zero()
    }

    pub(crate) fn remaining(&self) -> Duration {
        if self.revoked.load(Ordering::SeqCst) {
            return Duration::ZERO;
        }
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if let Some(deadline_ns) = self.deadline_ns {
            let continuous = murmur_capture_helper_protocol::capture_monotonic_ns()
                .and_then(|now| {
                    murmur_capture_helper_protocol::automatic_probe_remaining_ns(deadline_ns, now)
                })
                .map_or(Duration::ZERO, Duration::from_nanos);
            return remaining.min(continuous);
        }
        #[cfg(target_os = "macos")]
        {
            Duration::ZERO
        }
        #[cfg(not(target_os = "macos"))]
        {
            remaining
        }
    }

    pub(crate) fn deadline_ns(&self) -> Option<u64> {
        self.deadline_ns
    }

    pub(crate) fn revoke(&self) {
        self.revoked.store(true, Ordering::SeqCst);
        self.changed.notify_waiters();
    }

    async fn cancelled(&self) {
        loop {
            let changed = self.changed.notified();
            if !self.is_valid() {
                return;
            }
            tokio::select! {
                _ = changed => {},
                _ = tokio::time::sleep_until(self.deadline.into()) => return,
            }
        }
    }
}

#[derive(Default)]
struct Budget {
    context: Option<(u64, u64)>,
    round_started: Option<Instant>,
    tried: Vec<String>,
    cursor: usize,
    failed_rounds: u8,
    not_before: Option<Instant>,
}

#[derive(Debug, PartialEq, Eq)]
enum Next {
    Park,
    Wait(Instant),
    Probe(String),
}

impl Budget {
    fn context(&mut self, context: (u64, u64)) {
        if self.context != Some(context) {
            self.context = Some(context);
            self.round_started = None;
            self.tried.clear();
            self.cursor = 0;
            self.failed_rounds = 0;
            // Preserve the global minimum gap/backoff through settings or
            // topology flapping. A new context cannot buy a helper restart.
        }
    }

    fn next(&mut self, now: Instant, candidates: &[String], fresh_for: Option<Duration>) -> Next {
        if let Some(fresh_for) = fresh_for {
            self.round_started = None;
            self.tried.clear();
            self.cursor = 0;
            self.failed_rounds = 0;
            return Next::Wait(now + fresh_for);
        }
        if candidates.is_empty() || self.failed_rounds >= 3 {
            return Next::Park;
        }
        if let Some(not_before) = self.not_before.filter(|time| now < *time) {
            return Next::Wait(not_before);
        }
        let candidate = candidates
            .iter()
            .enumerate()
            .cycle()
            .skip(self.cursor % candidates.len())
            .take(candidates.len())
            .find(|(_, id)| !self.tried.contains(id));
        if self.tried.len() >= CANDIDATES_PER_ROUND
            || candidate.is_none()
            || self
                .round_started
                .is_some_and(|start| now + ATTEMPT_AND_STOP_LIMIT > start + ROUND_LIMIT)
        {
            self.failed_rounds += 1;
            self.round_started = None;
            self.tried.clear();
            self.not_before =
                Some(now + Duration::from_secs(if self.failed_rounds == 1 { 60 } else { 300 }));
            return if self.failed_rounds >= 3 {
                Next::Park
            } else {
                Next::Wait(self.not_before.unwrap())
            };
        }
        let (index, id) = candidate.expect("candidate absence handled above");
        let id = id.clone();
        self.cursor = (index + 1) % candidates.len();
        self.round_started.get_or_insert(now);
        self.tried.push(id.clone());
        Next::Probe(id)
    }

    fn stopped(&mut self, now: Instant) {
        self.not_before = Some(
            self.not_before
                .map_or(now + MIN_GAP, |time| time.max(now + MIN_GAP)),
        );
    }

    fn preempt(&mut self, now: Instant) {
        self.round_started = None;
        self.tried.clear();
        self.stopped(now);
    }

    fn retry(&mut self, now: Instant) {
        self.failed_rounds = 0;
        self.preempt(now);
    }
}

struct Active {
    generation: u64,
    device_id: String,
    preview_id: u64,
    phase: &'static str,
    permit: Arc<ProbePermit>,
}

#[derive(Default)]
struct Inner {
    generation: u64,
    interruption: u64,
    request: Option<SmartAutoRequest>,
    budget: Budget,
    active: Option<Active>,
    blocked: Option<&'static str>,
}

#[derive(Default)]
struct Controller {
    inner: Mutex<Inner>,
    changed: Notify,
    app: OnceLock<tauri::AppHandle>,
}

fn controller() -> &'static Controller {
    static INSTANCE: OnceLock<Controller> = OnceLock::new();
    INSTANCE.get_or_init(Controller::default)
}

pub(crate) fn wake() {
    controller().changed.notify_one();
}

#[cfg(test)]
pub(crate) async fn notified_for_test() {
    controller().changed.notified().await;
}

fn emit() {
    if let Some(app) = controller().app.get() {
        let _ = app.emit("smart-auto-microphone-changed", ());
    }
}

pub(crate) fn inventory_changed() {
    let mut inner = controller().inner.lock_or_recover();
    inner.interruption = inner.interruption.wrapping_add(1);
    if let Some(active) = &inner.active {
        active.permit.revoke();
    }
    drop(inner);
    wake();
}

/// Called before acquiring the shared transition for every real pipeline.
/// Revocation cannot wait behind a stalled supervisor acceptance.
pub(crate) fn preempt() {
    let mut inner = controller().inner.lock_or_recover();
    inner.interruption = inner.interruption.wrapping_add(1);
    if let Some(active) = &inner.active {
        active.permit.revoke();
    }
    inner.budget.preempt(Instant::now());
    drop(inner);
    wake();
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProbePolicy {
    enabled: bool,
    request: Option<SmartAutoRequest>,
}

#[tauri::command]
pub(crate) fn configure_smart_auto_probe(
    window: tauri::WebviewWindow,
    policy: serde_json::Value,
) -> Result<u64, String> {
    if window.label() != "main" {
        return Err("Automatic input checks can only be configured in the main window.".into());
    }
    let mut inner = controller().inner.lock_or_recover();
    let result = apply_policy(&mut inner, policy);
    drop(inner);
    wake();
    emit();
    result.map_err(str::to_string)
}

fn apply_policy(inner: &mut Inner, policy: serde_json::Value) -> Result<u64, &'static str> {
    // Fail closed before validation, with no async work between revoke and
    // installing the new generation. Malformed policy cannot retain consent.
    if let Some(active) = &inner.active {
        active.permit.revoke();
    }
    inner.interruption = inner.interruption.wrapping_add(1);
    let next = serde_json::from_value::<ProbePolicy>(policy)
        .map_err(|_| "Automatic input check policy is invalid.")
        .and_then(|policy| {
            if policy.enabled {
                policy
                    .request
                    .ok_or("Choose approved microphones before enabling automatic checks.")
                    .and_then(|request| {
                        crate::microphone_auto::validate(&request)?;
                        Ok(request)
                    })
                    .map(Some)
            } else if policy.request.is_some() {
                Err("A disabled automatic-check policy cannot include microphone approvals.")
            } else {
                Ok(None)
            }
        });
    let error = next.as_ref().err().copied();
    let request = next.unwrap_or(None);
    if inner.request != request || error.is_some() {
        inner.generation = inner.generation.wrapping_add(1);
        inner.request = request;
        inner.blocked = None;
    }
    let generation = inner.generation;
    error.map_or(Ok(generation), Err)
}

#[tauri::command]
pub(crate) fn retry_smart_auto_probe(window: tauri::WebviewWindow) -> Result<u64, String> {
    if window.label() != "main" {
        return Err("Retry automatic checks from the main Settings window.".into());
    }
    let mut inner = controller().inner.lock_or_recover();
    if inner.request.is_none() {
        return Err("Automatic input checks are not enabled.".into());
    }
    if inner.active.is_some() {
        return Err("An automatic input check is already in progress.".into());
    }
    inner.budget.retry(Instant::now());
    inner.blocked = None;
    let generation = inner.generation;
    drop(inner);
    wake();
    emit();
    Ok(generation)
}

pub(crate) fn status(request: &SmartAutoRequest) -> Option<SmartAutoStatus> {
    let inner = controller().inner.lock_or_recover();
    if inner.request.as_ref() != Some(request) {
        return None;
    }
    if let Some(active) = &inner.active {
        if active.generation != inner.generation
            || !request.approved_device_ids.contains(&active.device_id)
        {
            return Some(SmartAutoStatus::Blocked {
                message: "Waiting for the previous input check to stop safely.".into(),
                retry_after_ms: None,
            });
        }
        return Some(SmartAutoStatus::Probing {
            device_id: active.device_id.clone(),
            phase: active.phase,
            remaining_ms: active.permit.remaining().as_millis() as u64,
        });
    }
    inner.blocked.map(|message| SmartAutoStatus::Blocked {
        message: message.into(),
        retry_after_ms: inner
            .budget
            .not_before
            .filter(|_| inner.budget.failed_rounds < 3)
            .map(|time| time.saturating_duration_since(Instant::now()).as_millis() as u64)
            .filter(|ms| *ms > 0),
    })
}

fn idle(state: &State) -> bool {
    state.app_state.dictation.lock_or_recover().status == crate::state::DictationStatus::Idle
        && !state.app_state.file_transcribing.load(Ordering::SeqCst)
        && !state.app_state.meeting_blocks_asr()
        && !state
            .app_state
            .meeting_summary_active
            .load(Ordering::SeqCst)
        && !crate::meeting_diarization::is_active()
        && !state.app_state.transform_status().blocks_recording()
        && !state.transform_runtime.is_transform_busy()
        && !state.query.status().blocks_pipeline()
        && !state.benchmark.is_running()
        && !state.microphone_startup_benchmark.is_active()
        && !state.app_state.microphone_preview.is_active()
        && !crate::audio_lifecycle::is_audio_active()
        && !crate::keyboard::is_app_disabled()
        && corpus_idle(state)
}

fn corpus_idle(_state: &State) -> bool {
    #[cfg(feature = "internal-benchmark")]
    {
        !_state.corpus.is_active()
    }
    #[cfg(not(feature = "internal-benchmark"))]
    {
        true
    }
}

pub(crate) fn initialize(app: tauri::AppHandle) {
    if controller().app.set(app.clone()).is_err() {
        return;
    }
    // Notifications only wake a pending finite budget. Their payloads never
    // authorize work; admission re-reads all backend owner state under lock.
    for event in [
        "recording-status-changed",
        "file-transcription-status-changed",
        "transform-state-changed",
        "query-state-changed",
        "meeting-status-changed",
        "app-disabled-changed",
        "microphone-preview-status",
    ] {
        app.listen(event, |_| wake());
    }
    tauri::async_runtime::spawn(run(app));
}

async fn wait(next: Next) {
    match next {
        Next::Wait(deadline) => tokio::select! {
            _ = controller().changed.notified() => {},
            _ = tokio::time::sleep_until(deadline.into()) => {},
        },
        _ => controller().changed.notified().await,
    }
}

async fn run(app: tauri::AppHandle) {
    loop {
        let state = app.state::<State>();
        {
            let mut inner = controller().inner.lock_or_recover();
            if inner.active.as_ref().is_some_and(|active| {
                !state
                    .app_state
                    .microphone_preview
                    .is_current(active.preview_id)
            }) {
                inner.active = None;
                inner.budget.stopped(Instant::now());
            }
        }
        let desired = {
            let inner = controller().inner.lock_or_recover();
            inner
                .request
                .clone()
                .map(|request| (inner.generation, request))
        };
        let Some((generation, request)) = desired else {
            wait(Next::Park).await;
            continue;
        };
        let (epoch, candidates) = crate::audio_inventory::probe_candidates(&request);
        let status = crate::audio_inventory::cached_smart_auto_status(&request);
        let fresh = match status {
            SmartAutoStatus::Ready { valid_for_ms, .. } => {
                Some(Duration::from_millis(valid_for_ms))
            }
            _ => None,
        };
        let transition = state.app_state.recording_transition.lock().await;
        let allowed = idle(&state);
        let granted =
            crate::commands::permissions::check_microphone_permission_status() == "granted";
        let admission_at = Instant::now();
        let (next, interruption) = {
            let mut inner = controller().inner.lock_or_recover();
            if inner.generation != generation || inner.request.as_ref() != Some(&request) {
                drop(inner);
                drop(transition);
                continue;
            }
            inner.budget.context((generation, epoch));
            if fresh.is_some() {
                inner.blocked = None;
            }
            let next = if !allowed || !granted {
                if fresh.is_none() {
                    inner.blocked = Some(if !granted {
                        "Automatic checks need microphone permission already granted. No permission prompt was opened."
                    } else {
                        "Automatic input checks are waiting for Murmur to become idle."
                    });
                }
                Next::Park
            } else {
                let next = inner.budget.next(admission_at, &candidates, fresh);
                if fresh.is_none() {
                    inner.blocked = Some(if inner.budget.failed_rounds >= 3 {
                        "Automatic checks stopped after three unsuccessful rounds. Retry in Settings or pin a microphone."
                    } else if candidates.is_empty() {
                        "No approved microphone is available in the current inventory."
                    } else {
                        "No recently verified input. Automatic checks are waiting for their cooldown."
                    });
                }
                next
            };
            (next, inner.interruption)
        };
        let Next::Probe(device_id) = next else {
            drop(transition);
            emit();
            wait(next).await;
            continue;
        };
        let started_at = admission_at;
        let permit = Arc::new(ProbePermit::new(started_at));
        let preview = &state.app_state.microphone_preview;
        let Ok(preview_id) = preview.claim_automatic() else {
            drop(transition);
            wait(Next::Park).await;
            continue;
        };
        preview.bind_signal_evidence(
            preview_id,
            crate::audio_inventory::signal_evidence_key(Some(&device_id)),
        );
        {
            let mut inner = controller().inner.lock_or_recover();
            if inner.generation != generation
                || inner.interruption != interruption
                || inner.request.as_ref() != Some(&request)
            {
                preview.clear_if(preview_id);
                drop(inner);
                drop(transition);
                continue;
            }
            inner.active = Some(Active {
                generation,
                device_id: device_id.clone(),
                preview_id,
                phase: "connecting",
                permit: permit.clone(),
            });
            inner.blocked = None;
        }
        // Installing the permit before this recheck closes invalidation in
        // both directions: earlier changes fail this snapshot; later changes
        // synchronously revoke the published permit before an audio open.
        let (current_epoch, current_candidates) =
            crate::audio_inventory::probe_candidates(&request);
        if current_epoch != epoch || !current_candidates.contains(&device_id) {
            permit.revoke();
        }
        emit();
        let start_app = app.clone();
        let start_permit = permit.clone();
        let start = tokio::task::spawn_blocking(move || {
            crate::audio_lifecycle::start_automatic_preview_recording(
                start_app,
                device_id,
                preview_id,
                start_permit,
            )
        })
        .await;
        if !matches!(start, Ok(Ok(()))) {
            if crate::audio_lifecycle::is_audio_active() {
                preview.set_error_if(
                    preview_id,
                    "start_failed",
                    "Automatic input check is stopping after startup failed.",
                );
            } else {
                preview.fail_and_clear(
                    preview_id,
                    "start_failed",
                    "Automatic input check was refused.",
                );
            }
        }
        drop(transition);
        let startup_deadline = started_at + STARTUP_LIMIT;
        let ready = tokio::select! {
            ready = preview.wait_until_ready(preview_id, startup_deadline) => ready,
            _ = permit.cancelled() => false,
        };
        let mut evidence = (SignalVerificationResult::Interrupted, None);
        let mut verified_at = Instant::now();
        if ready && permit.is_valid() {
            if let Ok(deadline) = preview.begin_signal_verification(preview_id, Instant::now()) {
                set_phase(preview_id, "verifying");
                tokio::select! {
                    _ = tokio::time::sleep_until(deadline.into()) => {
                        verified_at = Instant::now();
                        evidence = preview.take_signal_verification(preview_id);
                    },
                    _ = permit.cancelled() => {},
                }
            }
        }
        let consent_live = !permit.revoked.load(Ordering::SeqCst);
        permit.revoke();
        set_phase(preview_id, "stopping");
        let stopped = crate::commands::microphone_preview::stop_exact_preview(
            &app,
            state.inner(),
            preview_id,
        )
        .await
        .is_ok();
        {
            let mut inner = controller().inner.lock_or_recover();
            let owned = inner.active.as_ref().is_some_and(|active| {
                active.generation == generation && active.preview_id == preview_id
            });
            let publish = owned
                && stopped
                && consent_live
                && inner.generation == generation
                && inner.interruption == interruption
                && inner.request.as_ref() == Some(&request);
            if owned && stopped {
                inner.active = None;
            }
            inner.budget.stopped(Instant::now());
            // Keep the policy generation locked through evidence publication.
            // Inventory independently checks topology and failure epochs.
            if publish {
                if let Some(key) = evidence.1 {
                    crate::audio_inventory::record_signal_evidence_at(
                        &key,
                        evidence.0,
                        verified_at,
                    );
                }
            }
        }
        emit();
        // Failed teardown leaves the Preview and supervisor claims untouched.
        // The next loop parks on authoritative busy state until actual Idle.
    }
}

fn set_phase(preview_id: u64, phase: &'static str) {
    if let Some(active) = controller()
        .inner
        .lock_or_recover()
        .active
        .as_mut()
        .filter(|active| active.preview_id == preview_id)
    {
        active.phase = phase;
    }
    emit();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> Vec<String> {
        ["a", "b", "c", "d"].map(str::to_owned).to_vec()
    }

    fn enabled_policy() -> serde_json::Value {
        serde_json::json!({"enabled": true, "request": {"approvedDeviceIds": ["a"], "preferredDeviceIds": ["a"], "allowContinuity": false}})
    }

    #[test]
    fn backend_defaults_disabled_and_invalid_policy_revokes_before_rejection() {
        for invalid in [
            serde_json::json!({"enabled": true}),
            serde_json::json!({"enabled": "true"}),
            serde_json::json!({"enabled": true, "request": {"approvedDeviceIds": [], "preferredDeviceIds": [], "allowContinuity": false}}),
            serde_json::json!({"enabled": false, "request": {"approvedDeviceIds": ["a"], "preferredDeviceIds": [], "allowContinuity": false}}),
        ] {
            let mut inner = Inner::default();
            assert!(inner.request.is_none());
            let generation = apply_policy(&mut inner, enabled_policy()).unwrap();
            let permit = Arc::new(ProbePermit::new(Instant::now()));
            inner.active = Some(Active {
                generation,
                device_id: "a".into(),
                preview_id: 1,
                phase: "connecting",
                permit: permit.clone(),
            });
            assert!(apply_policy(&mut inner, invalid).is_err());
            assert!(!permit.is_valid());
            assert!(inner.request.is_none());
            assert_ne!(inner.generation, generation);
            // Revocation is not evidence of teardown. Keep the old owner until
            // the supervisor confirms its exact generation has exited.
            assert_eq!(inner.active.as_ref().unwrap().preview_id, 1);
        }
    }

    #[test]
    fn consent_revoke_and_preference_change_invalidate_in_flight_generation() {
        let mut inner = Inner::default();
        let first = apply_policy(&mut inner, enabled_policy()).unwrap();
        let interruption = inner.interruption;
        let same = apply_policy(&mut inner, enabled_policy()).unwrap();
        assert_eq!(same, first);
        assert_ne!(inner.interruption, interruption);
        let changed = apply_policy(&mut inner, serde_json::json!({"enabled":true,"request":{"approvedDeviceIds":["a"],"preferredDeviceIds":[],"allowContinuity":false}})).unwrap();
        assert_ne!(changed, first);
        let disabled = apply_policy(&mut inner, serde_json::json!({"enabled":false})).unwrap();
        assert_ne!(changed, disabled);
        assert!(inner.request.is_none());
    }

    #[test]
    fn successful_reverification_resets_retry_budget_without_selecting_another_candidate() {
        let now = Instant::now();
        let mut budget = Budget {
            failed_rounds: 2,
            tried: vec!["a".into()],
            ..Budget::default()
        };
        assert_eq!(
            budget.next(now, &ids(), Some(Duration::from_secs(120))),
            Next::Wait(now + Duration::from_secs(120))
        );
        assert_eq!(budget.failed_rounds, 0);
        assert!(budget.tried.is_empty());
    }

    #[test]
    fn silent_candidates_are_bounded_backed_off_and_parked() {
        let mut budget = Budget::default();
        let mut now = Instant::now();
        budget.context((1, 1));
        for (round, order) in [["a", "b", "c"], ["d", "a", "b"], ["c", "d", "a"]]
            .into_iter()
            .enumerate()
        {
            for id in order {
                assert_eq!(budget.next(now, &ids(), None), Next::Probe(id.into()));
                budget.stopped(now + Duration::from_secs(8));
                now += Duration::from_secs(18);
            }
            let next = budget.next(now, &ids(), None);
            if round == 2 {
                assert_eq!(next, Next::Park);
            } else {
                let Next::Wait(deadline) = next else {
                    panic!("missing backoff")
                };
                now = deadline;
            }
        }
        assert_eq!(
            budget.next(now + Duration::from_secs(10_000), &ids(), None),
            Next::Park
        );
    }

    #[test]
    fn evidence_expiry_is_one_shot_and_context_changes_preserve_cooldown() {
        let now = Instant::now();
        let mut budget = Budget::default();
        budget.context((1, 1));
        assert_eq!(
            budget.next(now, &ids(), Some(Duration::from_secs(120))),
            Next::Wait(now + Duration::from_secs(120))
        );
        assert_eq!(
            budget.next(now + Duration::from_secs(120), &ids(), None),
            Next::Probe("a".into())
        );
        budget.preempt(now + Duration::from_secs(121));
        budget.context((2, 3));
        assert_eq!(
            budget.next(now + Duration::from_secs(122), &ids(), None),
            Next::Wait(now + Duration::from_secs(131))
        );
        assert_eq!(
            budget.next(now + Duration::from_secs(131), &["b".into()], None),
            Next::Probe("b".into())
        );
    }

    #[test]
    fn zombie_cleanup_consumes_total_round_budget_and_empty_topology_parks() {
        let now = Instant::now();
        let mut budget = Budget::default();
        assert_eq!(budget.next(now, &[], None), Next::Park);
        assert_eq!(budget.next(now, &ids(), None), Next::Probe("a".into()));
        budget.stopped(now + ATTEMPT_AND_STOP_LIMIT);
        assert_eq!(
            budget.next(now + Duration::from_secs(33), &ids(), None),
            Next::Probe("b".into())
        );
        budget.stopped(now + Duration::from_secs(56));
        assert!(matches!(
            budget.next(now + Duration::from_secs(66), &ids(), None),
            Next::Wait(_)
        ));
        assert_eq!(budget.failed_rounds, 1);
    }

    #[test]
    fn revoked_permit_never_revives() {
        let permit = ProbePermit::new(Instant::now());
        assert!(permit.is_valid());
        permit.revoke();
        assert!(!permit.is_valid());
        assert_eq!(permit.remaining(), Duration::ZERO);
    }
}
