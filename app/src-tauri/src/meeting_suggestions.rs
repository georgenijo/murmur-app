use crate::calendar::{self, CalendarWindow, SuggestionEvent};
use crate::{MutexExt, State};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{Emitter, Listener, Manager};
use tokio::sync::Notify;

const HORIZON_MS: u64 = 24 * 60 * 60 * 1_000;
const MIN_QUERY_GAP_MS: u64 = 30_000;
const SEEN_RETENTION_MS: u64 = 7 * HORIZON_MS;
const MAX_SEEN_OCCURRENCES: usize = 2_048;
const STALE_PROMPT: &str =
    "This meeting suggestion is no longer available. You can start Notetaker manually.";

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSuggestion {
    pub token: String,
    pub title: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

struct PendingSuggestion {
    prompt: MeetingSuggestion,
    event: SuggestionEvent,
}

#[derive(Default)]
struct DecisionState {
    enabled: bool,
    generation: u64,
    snapshot_until_ms: u64,
    query_not_before_ms: u64,
    refresh_needed: bool,
    query_failed: bool,
    waiting_for_operation: bool,
    permit: Option<Arc<AtomicBool>>,
    events: Vec<SuggestionEvent>,
    pending: Option<PendingSuggestion>,
    seen: BTreeMap<String, u64>,
}

struct QueryPlan {
    generation: u64,
    window: CalendarWindow,
    permit: Arc<AtomicBool>,
}

impl DecisionState {
    fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(permit) = self.permit.take() {
            permit.store(false, Ordering::Release);
        }
        self.events.clear();
        self.pending = None;
        self.refresh_needed = true;
    }

    fn configure(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.invalidate();
            self.enabled = enabled;
        }
    }

    fn query_plan(&mut self, now_ms: u64) -> Option<QueryPlan> {
        if !self.enabled
            || self.permit.is_some()
            || now_ms < self.query_not_before_ms
            || (!self.refresh_needed && now_ms < self.snapshot_until_ms)
        {
            return None;
        }
        let window = CalendarWindow::new(now_ms, now_ms.checked_add(HORIZON_MS)?).ok()?;
        let permit = Arc::new(AtomicBool::new(true));
        self.permit = Some(permit.clone());
        self.query_not_before_ms = now_ms.saturating_add(MIN_QUERY_GAP_MS);
        Some(QueryPlan {
            generation: self.generation,
            window,
            permit,
        })
    }

    fn complete_query(&mut self, plan: &QueryPlan, events: Result<Vec<SuggestionEvent>, String>) {
        if !self.enabled
            || self.generation != plan.generation
            || !plan.permit.load(Ordering::Acquire)
        {
            return;
        }
        self.permit = None;
        self.snapshot_until_ms = plan.window.end_ms;
        self.refresh_needed = false;
        self.query_failed = events.is_err();
        self.waiting_for_operation = events
            .as_ref()
            .is_err_and(|error| error == calendar::OPERATION_BUSY);
        self.events = events.unwrap_or_default();
        self.events.sort_by(|a, b| {
            a.start_ms
                .cmp(&b.start_ms)
                .then_with(|| a.occurrence_key.cmp(&b.occurrence_key))
        });
    }

    fn prompt(&self) -> Option<MeetingSuggestion> {
        self.pending.as_ref().map(|pending| pending.prompt.clone())
    }

    fn operation_available(&mut self) {
        if self.enabled && self.waiting_for_operation {
            self.waiting_for_operation = false;
            self.refresh_needed = true;
        }
    }

    fn evaluate(&mut self, now_ms: u64, bundle_id: Option<&str>, busy: bool) {
        self.seen
            .retain(|_, end_ms| now_ms <= end_ms.saturating_add(SEEN_RETENTION_MS));
        if !self.enabled || busy || !bundle_id.is_some_and(supported_meeting_app) {
            self.pending = None;
            return;
        }
        if self.pending.as_ref().is_some_and(|pending| {
            now_ms < pending.event.start_ms || now_ms >= pending.event.end_ms
        }) {
            self.pending = None;
        }
        if self.pending.is_some() || self.seen.len() >= MAX_SEEN_OCCURRENCES {
            return;
        }
        if let Some(event) = self.events.iter().find(|event| {
            event.start_ms <= now_ms
                && now_ms < event.end_ms
                && !self.seen.contains_key(&event.occurrence_key)
        }) {
            self.seen.insert(event.occurrence_key.clone(), event.end_ms);
            self.pending = Some(PendingSuggestion {
                prompt: MeetingSuggestion {
                    token: uuid::Uuid::new_v4().to_string(),
                    title: event.title.clone(),
                    start_ms: event.start_ms,
                    end_ms: event.end_ms,
                },
                event: event.clone(),
            });
        }
    }

    fn next_deadline(&self, now_ms: u64) -> Option<u64> {
        if !self.enabled {
            return None;
        }
        let refresh = if self.refresh_needed {
            self.query_not_before_ms.max(now_ms)
        } else {
            self.snapshot_until_ms.max(now_ms)
        };
        self.events
            .iter()
            .flat_map(|event| [event.start_ms, event.end_ms])
            .filter(|time| *time > now_ms)
            .chain(std::iter::once(refresh))
            .min()
    }

    fn accept(
        &mut self,
        token: &str,
        now_ms: u64,
        bundle_id: Option<&str>,
        busy: bool,
    ) -> Result<SuggestionEvent, String> {
        if !self.enabled
            || busy
            || !bundle_id.is_some_and(supported_meeting_app)
            || !self.pending.as_ref().is_some_and(|pending| {
                pending.prompt.token == token
                    && pending.event.start_ms <= now_ms
                    && now_ms < pending.event.end_ms
            })
        {
            return Err(STALE_PROMPT.into());
        }
        self.pending
            .take()
            .map(|pending| pending.event)
            .ok_or_else(|| STALE_PROMPT.into())
    }
}

fn supported_meeting_app(bundle_id: &str) -> bool {
    matches!(
        bundle_id,
        "us.zoom.xos"
            | "com.microsoft.teams"
            | "com.microsoft.teams2"
            | "com.tinyspeck.slackmacgap"
            | "com.cisco.webexmeetingsapp"
            | "com.webex.meetingmanager"
            | "com.apple.FaceTime"
            | "com.apple.Safari"
            | "com.google.Chrome"
            | "org.chromium.Chromium"
            | "org.mozilla.firefox"
            | "com.microsoft.edgemac"
            | "company.thebrowser.Browser"
            | "com.brave.Browser"
    )
}

#[derive(Default)]
struct Controller {
    state: Mutex<DecisionState>,
    changed: Notify,
    shutdown: AtomicBool,
    publish_dirty: AtomicBool,
}

#[derive(Clone, Default)]
pub(crate) struct MeetingSuggestions(Arc<Controller>);

impl MeetingSuggestions {
    fn wake(&self, invalidate_calendar: bool) {
        let mut state = self.0.state.lock_or_recover();
        if !state.enabled {
            return;
        }
        if state.query_failed {
            state.refresh_needed = true;
        }
        if invalidate_calendar {
            if state.pending.is_some() {
                self.0.publish_dirty.store(true, Ordering::Release);
            }
            state.invalidate();
        }
        drop(state);
        self.0.changed.notify_one();
    }

    fn prompt(&self) -> Option<MeetingSuggestion> {
        self.0.state.lock_or_recover().prompt()
    }

    pub(crate) fn accept(
        &self,
        token: &str,
        bundle_id: Option<&str>,
        busy: bool,
    ) -> Result<SuggestionEvent, String> {
        self.0
            .state
            .lock_or_recover()
            .accept(token, now_ms(), bundle_id, busy)
    }

    pub(crate) fn shutdown(&self) {
        self.0.shutdown.store(true, Ordering::Release);
        self.0.state.lock_or_recover().configure(false);
        self.0.changed.notify_one();
    }
}

fn now_ms() -> u64 {
    crate::sqlite_support::now_ms().try_into().unwrap_or(0)
}

pub(crate) fn is_busy(state: &State) -> bool {
    let dictation = state.app_state.dictation.lock_or_recover().status;
    workload_busy(
        dictation,
        state.app_state.transform_status(),
        state.query.status(),
        [
            state.transform_runtime.is_transform_busy()
                || model_work_busy(state.app_state.model_runtime.lifecycle_states()),
            state.app_state.meeting_blocks_asr(),
            state.app_state.file_transcribing.load(Ordering::SeqCst),
            state.app_state.microphone_preview.is_active(),
            state.benchmark.is_busy(),
            state.microphone_startup_benchmark.is_active(),
            crate::audio_lifecycle::is_audio_active(),
            crate::meeting_diarization::is_active(),
            crate::keyboard::is_app_disabled(),
            corpus_active(state),
        ],
    )
}

fn model_work_busy(
    states: Option<Vec<(String, crate::model_runtime::LifecycleState, bool)>>,
) -> bool {
    use crate::model_runtime::LifecycleState;
    states.is_none_or(|states| {
        states.iter().any(|(_, phase, _)| {
            matches!(
                phase,
                LifecycleState::Loading | LifecycleState::Warming | LifecycleState::Unloading
            )
        })
    })
}

fn workload_busy(
    dictation: crate::state::DictationStatus,
    transform: crate::state::TransformStatus,
    query: crate::query_flow::QueryStatus,
    owners: [bool; 10],
) -> bool {
    dictation != crate::state::DictationStatus::Idle
        || transform.blocks_recording()
        || query.blocks_pipeline()
        || owners.into_iter().any(|active| active)
}

fn corpus_active(state: &State) -> bool {
    #[cfg(feature = "internal-benchmark")]
    {
        state.corpus.is_active()
    }
    #[cfg(not(feature = "internal-benchmark"))]
    {
        let _ = state;
        false
    }
}

pub(crate) fn publish(app: &tauri::AppHandle) {
    let prompt = app.state::<State>().meeting_suggestions.prompt();
    for window in ["main", "overlay"] {
        let _ = app.emit_to(window, "meeting-suggestion-changed", &prompt);
    }
}

pub(crate) fn busy_changed(app: &tauri::AppHandle) {
    let state = app.state::<State>();
    let coordinator = &state.meeting_suggestions;
    let removed = coordinator
        .0
        .state
        .lock_or_recover()
        .pending
        .take()
        .is_some();
    coordinator.wake(false);
    if removed {
        publish(app);
    }
}

fn require_window(label: &str, main_only: bool) -> Result<(), String> {
    if label == "main" || (!main_only && label == "overlay") {
        Ok(())
    } else {
        Err("Meeting suggestions are only available from the main window and overlay.".into())
    }
}

#[tauri::command]
pub async fn configure_meeting_suggestions(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    enabled: bool,
    state: tauri::State<'_, State>,
) -> Result<(), String> {
    require_window(window.label(), true)?;
    let _transition = state.app_state.recording_transition.lock().await;
    if enabled && calendar::permission_status() != calendar::CalendarPermissionStatus::Granted {
        return Err(calendar::ACCESS_REQUIRED.into());
    }
    if enabled {
        calendar::set_suggestion_observation(&app, true).await?;
    }
    state
        .meeting_suggestions
        .0
        .state
        .lock_or_recover()
        .configure(enabled);
    if !enabled {
        calendar::set_suggestion_observation(&app, false).await?;
    }
    state.meeting_suggestions.0.changed.notify_one();
    publish(&app);
    Ok(())
}

#[tauri::command]
pub fn get_meeting_suggestion(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, State>,
) -> Result<Option<MeetingSuggestion>, String> {
    require_window(window.label(), false)?;
    if is_busy(&state)
        || state
            .meeting_suggestions
            .prompt()
            .is_some_and(|prompt| now_ms() >= prompt.end_ms)
    {
        state.meeting_suggestions.0.state.lock_or_recover().pending = None;
        publish(&app);
    }
    Ok(state.meeting_suggestions.prompt())
}

#[tauri::command]
pub fn dismiss_meeting_suggestion(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    token: String,
    state: tauri::State<'_, State>,
) -> Result<(), String> {
    require_window(window.label(), false)?;
    let mut inner = state.meeting_suggestions.0.state.lock_or_recover();
    if inner
        .pending
        .as_ref()
        .is_some_and(|pending| pending.prompt.token == token)
    {
        inner.pending = None;
    }
    drop(inner);
    publish(&app);
    Ok(())
}

pub(crate) fn initialize(app: tauri::AppHandle) {
    let coordinator = app.state::<State>().meeting_suggestions.clone();
    for event in [
        "recording-status-changed",
        "file-transcription-status-changed",
        "transform-state-changed",
        "query-state-changed",
        "meeting-status-changed",
        "meeting-summary-status-changed",
        "app-disabled-changed",
        "microphone-preview-status",
        "benchmark-progress",
        "microphone-startup-benchmark-progress",
        "model-runtime-status-changed",
    ] {
        let app_for_event = app.clone();
        app.listen_any(event, move |_| busy_changed(&app_for_event));
    }
    register_native_observers(&app, &coordinator);
    tauri::async_runtime::spawn(run(app, coordinator));
}

fn register_native_observers(app: &tauri::AppHandle, coordinator: &MeetingSuggestions) {
    use block2::RcBlock;
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceDidActivateApplicationNotification, NSWorkspaceDidWakeNotification,
    };
    use objc2_foundation::{
        NSNotification, NSNotificationCenter, NSSystemClockDidChangeNotification,
        NSSystemTimeZoneDidChangeNotification,
    };
    let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
    let default = NSNotificationCenter::defaultCenter();
    for (center, name, refresh) in unsafe {
        [
            (
                &*workspace,
                NSWorkspaceDidActivateApplicationNotification,
                false,
            ),
            (&*workspace, NSWorkspaceDidWakeNotification, true),
            (&*default, NSSystemClockDidChangeNotification, true),
            (&*default, NSSystemTimeZoneDidChangeNotification, true),
            (
                &*default,
                objc2_event_kit::EKEventStoreChangedNotification,
                true,
            ),
        ]
    } {
        let coordinator = coordinator.clone();
        let app = app.clone();
        let callback = RcBlock::new(move |_: std::ptr::NonNull<NSNotification>| {
            coordinator.wake(refresh);
            if refresh {
                publish(&app);
            } else {
                busy_changed(&app);
            }
        });
        let observer = unsafe {
            center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &callback)
        };
        std::mem::forget(observer);
    }
}

pub(crate) async fn frontmost_bundle_id(app: &tauri::AppHandle) -> Option<String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(crate::frontmost::query_frontmost_app_identity().bundle_id);
    })
    .ok()?;
    tokio::time::timeout(Duration::from_secs(2), receiver)
        .await
        .ok()?
        .ok()?
}

async fn run(app: tauri::AppHandle, coordinator: MeetingSuggestions) {
    loop {
        if coordinator.0.shutdown.load(Ordering::Acquire) {
            return;
        }
        let notified = coordinator.0.changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let operation_finished = calendar::OPERATION_FINISHED.notified();
        tokio::pin!(operation_finished);
        operation_finished.as_mut().enable();
        let plan = coordinator.0.state.lock_or_recover().query_plan(now_ms());
        if let Some(plan) = plan {
            let result = calendar::query_suggestion_events(plan.window, plan.permit.clone()).await;
            coordinator
                .0
                .state
                .lock_or_recover()
                .complete_query(&plan, result);
        }
        if !calendar::operation_in_progress() {
            coordinator.0.state.lock_or_recover().operation_available();
        }
        let state = app.state::<State>();
        let transition = state.app_state.recording_transition.lock().await;
        let enabled = coordinator.0.state.lock_or_recover().enabled;
        let bundle_id = if enabled {
            frontmost_bundle_id(&app).await
        } else {
            None
        };
        let now = now_ms();
        let busy = is_busy(&state);
        let (changed, deadline) = {
            let mut inner = coordinator.0.state.lock_or_recover();
            let prior = inner.prompt();
            inner.evaluate(now, bundle_id.as_deref(), busy);
            (prior != inner.prompt(), inner.next_deadline(now))
        };
        if changed || coordinator.0.publish_dirty.swap(false, Ordering::AcqRel) {
            publish(&app);
        }
        drop(transition);
        match deadline {
            Some(deadline) => tokio::select! {
                _ = notified => {},
                _ = operation_finished => { coordinator.0.state.lock_or_recover().operation_available(); },
                _ = tokio::time::sleep(Duration::from_millis(deadline.saturating_sub(now).max(1))) => {},
            },
            None => notified.await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(key: &str, start_ms: u64, end_ms: u64) -> SuggestionEvent {
        SuggestionEvent {
            occurrence_key: key.into(),
            title: "Synthetic planning call".into(),
            attendees: vec!["Alex".into()],
            start_ms,
            end_ms,
        }
    }

    fn ready(events: Vec<SuggestionEvent>) -> DecisionState {
        let mut state = DecisionState::default();
        state.configure(true);
        let plan = state.query_plan(1_000).unwrap();
        state.complete_query(&plan, Ok(events));
        state
    }

    #[test]
    fn suggestions_off_never_calls_calendar_and_discards_revoked_workers() {
        let mut state = DecisionState::default();
        let mut calendar_reads = 0;
        for now in [1_000, 20_000, HORIZON_MS * 10] {
            if state.query_plan(now).is_some() {
                calendar_reads += 1;
            }
            state.evaluate(now, Some("us.zoom.xos"), false);
            assert!(state.prompt().is_none());
            assert!(state.next_deadline(now).is_none());
        }
        assert_eq!(calendar_reads, 0);
        state.configure(true);
        let plan = state.query_plan(1_000).unwrap();
        assert!(state.query_plan(1_001).is_none());
        state.configure(false);
        assert!(!plan.permit.load(Ordering::Acquire));
        state.complete_query(&plan, Ok(vec![event("event", 1_000, 100_000)]));
        assert!(state.events.is_empty());
        state.configure(true);
        state.complete_query(&plan, Ok(vec![event("late", 1_000, 100_000)]));
        assert!(state.events.is_empty());
    }

    #[test]
    fn suggestions_require_supported_frontmost_and_prompt_once_across_toggles() {
        let synthetic = event("occurrence", 1_000, 100_000);
        let mut state = ready(vec![synthetic.clone()]);
        state.evaluate(2_000, Some("com.apple.TextEdit"), false);
        assert!(state.prompt().is_none());
        state.evaluate(2_000, Some("us.zoom.xos"), false);
        let first = state.prompt().unwrap();
        state.evaluate(3_000, Some("us.zoom.xos"), false);
        assert_eq!(state.prompt().unwrap().token, first.token);
        state.pending = None;
        state.evaluate(4_000, Some("us.zoom.xos"), false);
        assert!(state.prompt().is_none());
        state.configure(false);
        state.configure(true);
        let plan = state.query_plan(40_000).unwrap();
        state.complete_query(&plan, Ok(vec![synthetic]));
        state.evaluate(40_000, Some("com.google.Chrome"), false);
        assert!(state.prompt().is_none());
        state.events.push(event("next-occurrence", 40_000, 100_000));
        state.evaluate(40_000, Some("com.google.Chrome"), false);
        assert!(state.prompt().is_some());
    }

    #[test]
    fn suggestions_acceptance_refuses_busy_stale_expired_and_switched_apps() {
        let mut state = ready(vec![event("occurrence", 1_000, 100_000)]);
        state.evaluate(2_000, Some("us.zoom.xos"), false);
        let token = state.prompt().unwrap().token;
        let mut capture_starts = 0;
        for (token, time, bundle, busy) in [
            ("stale", 2_000, Some("us.zoom.xos"), false),
            (token.as_str(), 2_000, Some("us.zoom.xos"), true),
            (token.as_str(), 2_000, Some("com.apple.TextEdit"), false),
            (token.as_str(), 100_000, Some("us.zoom.xos"), false),
        ] {
            if state.accept(token, time, bundle, busy).is_ok() {
                capture_starts += 1;
            }
        }
        assert_eq!(capture_starts, 0);
        let accepted = state
            .accept(&token, 2_000, Some("us.zoom.xos"), false)
            .unwrap();
        capture_starts += 1;
        assert_eq!(accepted.title, "Synthetic planning call");
        assert_eq!(accepted.attendees, ["Alex"]);
        assert_eq!(capture_starts, 1);
        assert!(state
            .accept(&token, 2_000, Some("us.zoom.xos"), false)
            .is_err());
        state.evaluate(3_000, Some("us.zoom.xos"), false);
        assert!(state.prompt().is_none());
    }

    #[test]
    fn suggestions_follow_event_boundaries_and_retry_operation_contention_once_available() {
        let mut state = ready(vec![event("upcoming", 60_000, 90_000)]);
        assert_eq!(state.next_deadline(2_000), Some(60_000));
        assert!(
            state.query_plan(60_000).is_none(),
            "app activation and event start reuse the snapshot"
        );
        state.evaluate(60_000, Some("us.zoom.xos"), false);
        assert!(state.prompt().is_some());
        assert_eq!(state.next_deadline(60_000), Some(90_000));
        state.evaluate(90_000, Some("us.zoom.xos"), false);
        assert!(state.prompt().is_none());
        state.invalidate();
        let plan = state.query_plan(100_000).unwrap();
        state.complete_query(&plan, Err(calendar::OPERATION_BUSY.into()));
        assert!(state.query_failed);
        assert!(state.query_plan(100_001).is_none());
        state.operation_available();
        assert_eq!(state.next_deadline(100_001), Some(130_000));
        let retry = state.query_plan(130_000).unwrap();
        state.complete_query(&retry, Ok(vec![event("live", 100_000, 150_000)]));
        state.evaluate(130_000, Some("us.zoom.xos"), false);
        assert!(state.prompt().is_some());
        assert!(!state.query_failed);
    }

    #[test]
    fn suggestions_calendar_failure_parks_until_a_new_trigger_and_changes_invalidate_acceptance() {
        let mut state = ready(vec![event("live", 1_000, 100_000)]);
        state.evaluate(2_000, Some("us.zoom.xos"), false);
        let token = state.prompt().unwrap().token;
        state.invalidate();
        assert!(state
            .accept(&token, 2_000, Some("us.zoom.xos"), false)
            .is_err());
        let plan = state.query_plan(40_000).unwrap();
        state.complete_query(&plan, Err(calendar::UNAVAILABLE.into()));
        state.operation_available();
        assert!(state.query_plan(80_000).is_none());
        assert_eq!(state.next_deadline(80_000), Some(plan.window.end_ms));
    }

    #[test]
    fn suggestions_suppress_all_pipeline_phases_and_owner_flags() {
        use crate::query_flow::QueryStatus as Q;
        use crate::state::{DictationStatus as D, TransformStatus as T};
        assert!(!workload_busy(D::Idle, T::Idle, Q::Idle, [false; 10]));
        for phase in [D::Starting, D::Recording, D::Recovering, D::Processing] {
            assert!(workload_busy(phase, T::Idle, Q::Idle, [false; 10]));
        }
        for phase in [
            T::Capturing,
            T::Connecting,
            T::Listening,
            T::Thinking,
            T::ReviewPending,
            T::Applying,
        ] {
            assert!(workload_busy(D::Idle, phase, Q::Idle, [false; 10]));
        }
        for phase in [Q::Connecting, Q::Listening, Q::Transcribing, Q::Running] {
            assert!(workload_busy(D::Idle, T::Idle, phase, [false; 10]));
        }
        for index in 0..10 {
            let mut owners = [false; 10];
            owners[index] = true;
            assert!(workload_busy(D::Idle, T::Idle, Q::Idle, owners));
        }
        let mut state = ready(vec![event("live", 1_000, 100_000)]);
        state.evaluate(2_000, Some("us.zoom.xos"), true);
        assert!(state.prompt().is_none());
        assert!(
            state.seen.is_empty(),
            "suppressed events may prompt after work finishes"
        );
    }

    #[test]
    fn suggestions_model_load_warm_unload_and_contention_are_busy_without_filesystem_probes() {
        use crate::model_runtime::LifecycleState as L;
        assert!(model_work_busy(None));
        for phase in [L::Loading, L::Warming, L::Unloading] {
            assert!(model_work_busy(Some(vec![(
                "synthetic".into(),
                phase,
                false
            )])));
        }
        for phase in [L::Ready, L::Failed, L::Unloaded] {
            assert!(!model_work_busy(Some(vec![(
                "synthetic".into(),
                phase,
                false
            )])));
        }
    }

    #[test]
    fn suggestions_window_gates_and_seen_memory_are_bounded() {
        assert!(require_window("main", true).is_ok());
        assert!(require_window("overlay", true).is_err());
        assert!(require_window("overlay", false).is_ok());
        assert!(require_window("log-viewer", false).is_err());
        let mut state = ready(vec![event("live", 1_000, 100_000)]);
        state.seen = (0..MAX_SEEN_OCCURRENCES)
            .map(|index| (index.to_string(), 100_000))
            .collect();
        state.evaluate(2_000, Some("us.zoom.xos"), false);
        assert!(state.prompt().is_none());
    }
}
