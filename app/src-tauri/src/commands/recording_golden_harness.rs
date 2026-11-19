//! Stage 0b golden-trace harness (plan §6 Stage 0b, C9.3 to C9.5).
//!
//! One [`World`] per scenario run holds a mock Tauri app (the managed `State`
//! and the lifecycle bridge `handle_audio_lifecycle_and_ready_tasks` run on it
//! unchanged), a fake `DictationHost` and a fake audio supervisor. The three
//! dictation commands run against the fake host. The supervisor's
//! notifications go through the production bridge, exactly as
//! `ProductionLifecycleSink` does. Every port call, every emit, every
//! scripted reply and every observed change of shared state is appended to one
//! ordered trace, which a scenario compares byte for byte with its golden file.
//!
//! **Synchronization is causal, never timed.** The harness has no clock and no
//! sleep. A scenario drives each command future by hand:
//!
//! * [`poll_once`] polls a future until it is pending. A command that waits
//!   for the recording transition (the serializer) is pending exactly when
//!   the transition is held, so "queued behind start" is an observation, not
//!   a delay.
//! * [`Gate`]s are barriers inside an awaited port (the Smart Destination
//!   capture, the pipeline). A future that reaches a closed gate is pending
//!   until the scenario opens it.
//! * The fake supervisor never runs on its own. Its notifications are steps
//!   the scenario takes, and its hold points (`hold_before_ready`,
//!   `hold_before_owner_clear`) are pending steps the scenario releases, so a
//!   window production has between two supervisor statements is a place the
//!   scenario can stand in.
//! * A hook is a closure a port runs at the point production reaches it, so a
//!   scenario can change shared state "between" two statements of one function
//!   with no timing at all.
//!
//! Process-global state the commands touch (the keyboard processing flag, the
//! partial ticker's hide ownership, the interrupted-capture store, the
//! performance begin ledger) is reset before and after every run, and runs are
//! serialized by [`GOLDEN_LOCK`].
#![cfg(test)]

use super::dictation_host::{
    HostEmitError, HostFuture, KeyboardPort, PerformancePort, SmartDestinationPort, SupervisorPort,
    TargetSnapshotPort,
};
use super::tests::test_state;
use super::*;
#[cfg(feature = "dictation-shadow")]
use crate::dictation_shadow::replay as shadow_replay;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex as StdMutex;
use std::task::{Context, Poll, Wake, Waker};
use tauri::Listener;

pub(super) type Fut<T> = Pin<Box<dyn Future<Output = T> + Send>>;
type Hook = Box<dyn FnOnce() + Send>;
type MockHandle = tauri::AppHandle<tauri::test::MockRuntime>;

/// Serializes runs: the keyboard flag, the hide ownership and the begin ledger
/// are process-global.
static GOLDEN_LOCK: StdMutex<()> = StdMutex::new(());

/// The ids a scenario may use. Global stores are cleared for all of them.
const MAX_ID: u64 = 16;

thread_local! {
    /// The one hook the bridge's S12 tail runs (see [`World::on_s12_tail`]).
    static S12_TAIL: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

/// Called by `handle_audio_lifecycle_with` (test builds only) after
/// S12 released the dictation lock and before its unscoped keyboard/emit tail.
pub(super) fn run_s12_tail_hook() {
    let hook = S12_TAIL.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

/// Every event the lifecycle bridge emits through the app handle.
const BRIDGE_EVENTS: [&str; 8] = [
    "recording-status-changed",
    "audio-initialization-stalled",
    "audio-recovery-started",
    "recording-cancelled",
    "recording-initialization-failed",
    "recording-cleanup-finished",
    "recording-recovery-stalled",
    "recording-interrupted",
];

// ------------------------------------------------------------- drivers

struct Unpark(std::thread::Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Poll `future` once. `Pending` means it is parked on a barrier or on the
/// recording transition.
pub(super) fn poll_once<T>(future: &mut Fut<T>) -> Poll<T> {
    future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
}

/// Run `future` to completion with no barrier left to open: pending here is a
/// scenario bug, reported as such rather than waited on.
pub(super) fn complete<T>(future: &mut Fut<T>) -> T {
    match poll_once(future) {
        Poll::Ready(value) => value,
        Poll::Pending => {
            panic!("scenario deadlock: the future is parked on a barrier nobody opens")
        }
    }
}

/// [`complete`] for a future that joins a `spawn_blocking` task: the join
/// wakes this thread, so parking is a wait on that join and nothing else.
pub(super) fn complete_joining<T>(future: &mut Fut<T>) -> T {
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        std::thread::park();
    }
}

// The failure bound on helper threads lives in its own file: it never orders a
// scenario (G3: no scenario waits on a clock), it only turns a hang into a failure.
#[path = "recording_helper_wait.rs"]
mod helper_wait;
pub(super) use helper_wait::{
    await_arrival, await_release, helper_failed, join_bounded, spin_bounded,
};

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("golden runtime")
    })
}

/// A barrier inside an awaited port.
#[derive(Default)]
struct Gate {
    open: std::sync::atomic::AtomicBool,
    waker: StdMutex<Option<Waker>>,
}

impl Gate {
    async fn pass(&self) {
        std::future::poll_fn(|context| {
            if self.open.load(Ordering::SeqCst) {
                return Poll::Ready(());
            }
            *self
                .waker
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(context.waker().clone());
            if self.open.load(Ordering::SeqCst) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await
    }

    fn release(&self) {
        self.open.store(true, Ordering::SeqCst);
        let waker = self
            .waker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

// --------------------------------------------------------------- script

/// When the fake supervisor's cold start reaches `Ready`, relative to the
/// start command's context installation (C9.4 scenario 10, C9.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ColdStart {
    /// `set_phase(Recording)` and `notify(Ready)` both happen inside the
    /// supervisor start call, before the command installs a context.
    ReadyBeforeContext,
    /// Both happen later, when the scenario calls `release_ready`: the command
    /// installs its context and finds the phase not yet `Recording`.
    ReadyAfterContext,
    /// `set_phase(Recording)` happens inside the start call, then the fake
    /// blocks at `hold_before_ready` until the scenario releases it (C9.5).
    HoldBeforeReady,
}

pub(super) struct Knobs {
    pub(super) app_disabled: bool,
    pub(super) cold_start: ColdStart,
    /// An error the supervisor start returns instead of an ack.
    pub(super) start_ack: Option<AudioStartError>,
    /// Whether the supervisor accepts the attempt before it fails the start.
    pub(super) accept_before_failing: bool,
    /// Notifications the supervisor delivers inside the start call, before it
    /// answers (a failure terminal or a cancel that beats the ack).
    pub(super) before_ack: Vec<AudioLifecycleEventScript>,
    /// Samples a supervisor stop returns (`Err` is a stop timeout).
    pub(super) stop_result: Result<usize, &'static str>,
    /// A stop that returns `Ok` without sealing its delivery: the supervisor
    /// still owns the capture (phase `Stopping`) and parks at
    /// `hold_before_owner_clear` until the scenario releases it (PD-18 (c)).
    pub(super) stop_unsealed: bool,
    pub(super) cancel_result: Result<bool, &'static str>,
    /// The pipeline's delivery step takes the ownership fence D through the
    /// production `with_current_delivery_owner` and holds it until the
    /// scenario calls [`World::finish_delivery`] (C9.4 scenario 2).
    pub(super) hold_delivery: bool,
    /// The pipeline fails: it returns early, so its guard's drop (not a `finish`)
    /// leaves, and its caller gets this error (Stage 2a-2 review).
    pub(super) pipeline_error: Option<&'static str>,
    /// The pipeline guard's `finish` closure (the `In` effect `EM(idle)`, run under
    /// D) panics.
    pub(super) pipeline_finish_panics: bool,
    /// What the Smart Auto resolve port answers (`None`: an error, the scenario did
    /// not script it).
    pub(super) smart_auto: Option<Result<crate::microphone_auto::SmartAutoSelection, String>>,
    /// The `smart-destination-selected` emit fails.
    pub(super) smart_destination_emit_error: bool,
    /// The scheduler keeps the stop task [`auto_stop_dispatch`] built around the
    /// registered `AutoStop` owner, for [`World::owned_auto_stop`] to hand to the
    /// test (R215.2 `RS`; production's scheduler spawns the same task). Off, it
    /// drops the task, and so the owner, as an unrun spawn would.
    pub(super) carry_auto_stop_owner: bool,
    /// A stop whose reply never comes from the supervisor: the receive timed out or
    /// the channel closed. Production returns it whatever the owner is, so it is
    /// separate from `stop_result`, which is the supervisor's own reply and is applied
    /// only after the supervisor's ownership check.
    pub(super) stop_transport_failure: Option<&'static str>,
}

impl Default for Knobs {
    fn default() -> Self {
        Self {
            app_disabled: false,
            cold_start: ColdStart::ReadyBeforeContext,
            start_ack: None,
            accept_before_failing: true,
            before_ack: Vec::new(),
            stop_result: Ok(0),
            stop_unsealed: false,
            cancel_result: Ok(true),
            hold_delivery: false,
            pipeline_error: None,
            pipeline_finish_panics: false,
            smart_auto: None,
            smart_destination_emit_error: false,
            carry_auto_stop_owner: false,
            stop_transport_failure: None,
        }
    }
}

/// The supervisor notifications a scenario can script, as data so a knob
/// stays `Send` without cloning an `AudioLifecycleEvent`.
#[derive(Clone, Copy)]
pub(super) enum AudioLifecycleEventScript {
    Recovering(AudioCancelReason),
    InitializationFailed,
    Idle,
}

impl AudioLifecycleEventScript {
    fn event(self) -> AudioLifecycleEvent {
        match self {
            Self::Recovering(reason) => AudioLifecycleEvent::Recovering { reason },
            Self::InitializationFailed => AudioLifecycleEvent::InitializationFailed {
                error: "scripted initialization failure".to_string(),
                kind: audio::AudioFailureKind::BackendError,
                recovery_reason: None,
            },
            Self::Idle => AudioLifecycleEvent::Idle,
        }
    }
}

// ---------------------------------------------------------- fake audio

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Idle,
    Starting,
    Recording,
    Stopping,
    Recovering,
}

#[derive(Default)]
struct Supervisor {
    owner: Option<u64>,
    phase: Option<Phase>,
    /// `Ready` held back by `hold_before_ready` or `ReadyAfterContext`:
    /// (recording id, whether `set_phase(Recording)` is still to happen).
    held_ready: Option<(u64, bool)>,
    /// The cleanup tail held back by `hold_before_owner_clear`.
    held_owner_clear: Option<u64>,
    /// A cancelled attempt's own `Idle`, held until the scenario releases it.
    held_cancel_finish: Option<u64>,
    /// A sealed stop's cleanup tail (`audio_lifecycle.rs:2280-2290`): the
    /// samples left the supervisor, the owner stays `Dictation(id)` in
    /// `Stopping` until `finish_attempt` clears it and sends `CleanupFinished`.
    sealed_pending: Option<u64>,
}

impl Supervisor {
    fn phase(&self) -> Phase {
        self.phase.unwrap_or(Phase::Idle)
    }
}

#[derive(Default)]
struct Seen {
    status: Option<DictationStatus>,
    id: u64,
    processing: bool,
    /// The id of the installed recording context (M4), 0 for none.
    context: u64,
    /// `AppState::cancelled_id` (M3).
    cancelled_through: u64,
    terminals: BTreeMap<u64, String>,
    perf: BTreeMap<u64, String>,
}

// --------------------------------------------------------------- world

pub(super) struct World {
    /// This world's own `Arc`, for an entry that needs a host from `&self`.
    me: std::sync::Weak<World>,
    app: MockHandle,
    log: StdMutex<Vec<String>>,
    seen: StdMutex<Seen>,
    hooks: StdMutex<HashMap<String, Vec<Hook>>>,
    gates: StdMutex<HashMap<String, Arc<Gate>>>,
    knobs: StdMutex<Knobs>,
    supervisor: StdMutex<Supervisor>,
    auto_stops: StdMutex<Vec<u64>>,
    /// Stop tasks (each holding its `AutoStop` owner) the bridge scheduled while
    /// `carry_auto_stop_owner` is set.
    auto_stop_tasks: StdMutex<
        Vec<(
            u64,
            murmur_dictation_core::decision::Continuation,
            AutoStopTask,
        )>,
    >,
    delivery: StdMutex<Option<Delivery>>,
    /// Smart Auto resolves the start asked its host for.
    smart_auto_resolves: std::sync::atomic::AtomicUsize,
    /// How many times the publish asked for its side tasks (`SD`): a counter, not
    /// a trace line, so no recorded golden changes.
    side_task_spawns: std::sync::atomic::AtomicUsize,
    /// `core_divergences()` when this run began. The persisted-core invariant
    /// (#1160, Opus seat) is that it never grows during a run: a path that writes
    /// `dictation.status` or the recording id without the `FieldAdapter` would
    /// otherwise stay green until the lifecycle cutover starts reading M1 from
    /// `PersistedCore`.
    divergence_baseline: u64,
}

/// A delivery operation parked inside the production ownership fence.
struct Delivery {
    release: std::sync::mpsc::Sender<()>,
    thread: std::thread::JoinHandle<Option<()>>,
}

fn locked<T>(mutex: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl World {
    fn new(app: MockHandle) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            app,
            log: StdMutex::new(Vec::new()),
            seen: StdMutex::new(Seen::default()),
            hooks: StdMutex::new(HashMap::new()),
            gates: StdMutex::new(HashMap::new()),
            knobs: StdMutex::new(Knobs::default()),
            supervisor: StdMutex::new(Supervisor::default()),
            auto_stops: StdMutex::new(Vec::new()),
            auto_stop_tasks: StdMutex::new(Vec::new()),
            delivery: StdMutex::new(None),
            smart_auto_resolves: std::sync::atomic::AtomicUsize::new(0),
            side_task_spawns: std::sync::atomic::AtomicUsize::new(0),
            divergence_baseline: crate::dictation_executor::adapter::core_divergences(),
        })
    }

    /// The persisted-core invariant, checked at every trace line and at the end of
    /// a run: `FieldAdapter` has counted no new divergence since the run began, and
    /// whenever D is free the persisted M1/M2 equal the live `status` and
    /// `recording_id`. (With D held, the live and persisted writes of one effect
    /// are mid-commit, so the equality is read only when D is free, the same rule
    /// `sample` uses; the counter is checked regardless.)
    fn assert_persisted_core(&self, at: &str) {
        self.assert_no_new_divergence(at);
        let state = self.state();
        let Some(dictation) = state.app_state.dictation.try_lock_or_recover() else {
            return;
        };
        self.assert_core_equals_live(&state.app_state, &dictation, at);
    }

    /// `FieldAdapter` has counted no divergence since this run began.
    fn assert_no_new_divergence(&self, at: &str) {
        let divergences = crate::dictation_executor::adapter::core_divergences();
        assert!(
            divergences <= self.divergence_baseline,
            "persisted-core invariant: FieldAdapter counted {} new divergence(s) at `{at}` \
             (a write to dictation.status or the recording id bypassed the adapter)",
            divergences - self.divergence_baseline
        );
    }

    /// The persisted M1/M2 equal the live status and recording id, read under the
    /// dictation lock the caller holds.
    fn assert_core_equals_live(
        &self,
        app_state: &AppState,
        dictation: &crate::state::DictationState,
        at: &str,
    ) {
        let live_status = crate::dictation_admission::core_status(dictation.status);
        let live_id =
            murmur_dictation_core::ids::RecordingId(app_state.recording_id.load(Ordering::SeqCst));
        assert!(
            dictation.core.status() == live_status && dictation.core.current_id() == live_id,
            "persisted-core invariant: persisted M1/M2 ({:?}, {:?}) differ from live ({:?}, {:?}) at `{at}`",
            dictation.core.status(),
            dictation.core.current_id(),
            live_status,
            live_id
        );
    }

    fn listen_to_the_bridge(self: &Arc<Self>) {
        for event in BRIDGE_EVENTS {
            let world = Arc::downgrade(self);
            self.app.listen_any(event, move |received| {
                if let Some(world) = world.upgrade() {
                    world.push(format!("emit {event} {}", received.payload()));
                    world.fire(&format!("emit:{event}"));
                }
            });
        }
    }

    /// The mock app handle the world runs on (additive: the #117 replay hands it to
    /// the real `ensure_vad_model`).
    pub(super) fn app_handle(&self) -> MockHandle {
        self.app.clone()
    }

    pub(super) fn state(&self) -> tauri::State<'_, State> {
        self.app.state::<State>()
    }

    // ---- the trace

    /// Append one line, preceded by any change of shared state seen since the
    /// last line. A change is sampled at the next boundary the trace has, so
    /// its position relative to the lines around it is a causal fact.
    pub(super) fn push(&self, entry: impl Into<String>) {
        let entry = entry.into();
        self.assert_persisted_core(&entry);
        self.sample();
        locked(&self.log).push(entry);
    }

    pub(super) fn note(&self, text: &str) {
        self.push(format!("# {text}"));
    }

    fn sample(&self) {
        let state = self.state();
        let mut seen = locked(&self.seen);
        let mut lines = Vec::new();
        // `try_lock`: a port can be reached with the dictation lock held (the
        // refusal path emits under it). The skip is decided by this call
        // stack alone, so it is the same on every run.
        if let Some(dictation) = state.app_state.dictation.try_lock_or_recover() {
            if seen.status != Some(dictation.status) {
                lines.push(format!(
                    "~ status {} -> {:?}",
                    seen.status
                        .map_or_else(|| "?".to_string(), |status| format!("{status:?}")),
                    dictation.status
                ));
                seen.status = Some(dictation.status);
            }
        }
        let id = state.app_state.recording_id.load(Ordering::SeqCst);
        if id != seen.id {
            lines.push(format!("~ recording_id {} -> {id}", seen.id));
            seen.id = id;
        }
        let processing = keyboard::is_processing();
        if processing != seen.processing {
            lines.push(format!("~ processing {processing}"));
            seen.processing = processing;
        }
        let context = (1..=MAX_ID)
            .find(|recording_id| state.app_state.active_context(*recording_id).is_some())
            .unwrap_or(0);
        if context != seen.context {
            lines.push(if context == 0 {
                format!("~ context {} cleared", seen.context)
            } else {
                format!("~ context {context} installed")
            });
            seen.context = context;
        }
        let cancelled_through = state.app_state.cancelled_id.load(Ordering::SeqCst);
        if cancelled_through != seen.cancelled_through {
            lines.push(format!("~ cancelled_through {cancelled_through}"));
            seen.cancelled_through = cancelled_through;
        }
        for recording_id in 1..=id {
            let terminal = state
                .app_state
                .dictation_telemetry
                .terminal_for_test(recording_id)
                .map(|(outcome, code)| format!("{outcome:?}/{code:?}"));
            if let Some(terminal) = terminal {
                if seen.terminals.get(&recording_id) != Some(&terminal) {
                    lines.push(format!("~ terminal {recording_id} {terminal}"));
                    seen.terminals.insert(recording_id, terminal);
                }
            }
            let claimed = dictation_performance_begins()
                .claimed_outcome(recording_id)
                .map(|outcome| serde_json::to_string(&outcome).expect("outcome serializes"));
            if let Some(claimed) = claimed {
                if seen.perf.get(&recording_id) != Some(&claimed) {
                    lines.push(format!("~ perf-claim {recording_id} {claimed}"));
                    seen.perf.insert(recording_id, claimed);
                }
            }
        }
        drop(seen);
        locked(&self.log).extend(lines);
    }

    fn finish(&self) -> Vec<String> {
        // A sealed stop's cleanup tail always completes: a scenario that does
        // not need it mid-run ends with it, so every golden shows the real tail.
        let sealed = locked(&self.supervisor).sealed_pending;
        if let Some(recording_id) = sealed {
            self.note("end of run: the supervisor finishes the sealed stop");
            self.finish_sealed_stop(recording_id);
        }
        assert!(
            S12_TAIL.with(|slot| slot.borrow_mut().take()).is_none(),
            "the S12 tail hook never fired"
        );
        assert!(
            locked(&self.delivery).is_none(),
            "a delivery is still holding the ownership fence"
        );
        self.sample();
        self.assert_persisted_core("end of run");
        let state = self.state();
        let status = state.app_state.dictation.lock_or_recover().status;
        let context = (1..=MAX_ID)
            .find(|recording_id| state.app_state.active_context(*recording_id).is_some())
            .map_or_else(
                || "none".to_string(),
                |recording_id| recording_id.to_string(),
            );
        locked(&self.log).push(format!(
            "= final status={status:?} processing={} context={context} cancelled_through={}",
            keyboard::is_processing(),
            state.app_state.cancelled_id.load(Ordering::SeqCst)
        ));
        let leftover: Vec<String> = locked(&self.hooks)
            .iter()
            .filter(|(_, hooks)| !hooks.is_empty())
            .map(|(label, _)| label.clone())
            .collect();
        assert!(leftover.is_empty(), "hooks never fired: {leftover:?}");
        let supervisor = locked(&self.supervisor);
        assert!(
            supervisor.held_ready.is_none()
                && supervisor.held_owner_clear.is_none()
                && supervisor.held_cancel_finish.is_none(),
            "a supervisor hold point was never released"
        );
        drop(supervisor);
        locked(&self.hooks).clear();
        locked(&self.log).clone()
    }

    // ---- hooks and gates

    /// Run `hook` once, the next time a port reaches `label`.
    pub(super) fn on(&self, label: &str, hook: impl FnOnce() + Send + 'static) {
        locked(&self.hooks)
            .entry(label.to_string())
            .or_default()
            .push(Box::new(hook));
    }

    /// Run `hook` once, the next time a port reaches `label`, **on the thread of
    /// the port call**, even inside a core span: an inline listener of the emit.
    /// The core must refuse whatever re-enters it from there (C4.2).
    pub(super) fn on_inline(&self, label: &str, hook: impl FnOnce() + Send + 'static) {
        self.on(&format!("inline:{label}"), hook);
    }

    /// Deliver a supervisor notification through the real bridge on the calling
    /// thread, whatever that thread is inside: what a listener that an emit
    /// invokes inline does.
    pub(super) fn bridge_inline(&self, recording_id: u64, event: AudioLifecycleEvent) {
        handle_audio_lifecycle_and_ready_tasks(
            self.app.clone(),
            recording_id,
            event,
            |_, _| {},
            |_, _, _| {},
        );
    }

    fn fire(&self, label: &str) {
        #[cfg(feature = "dictation-shadow")]
        if label == "supervisor.start" {
            shadow_replay::blocking_point("supervisor.start");
        }
        let (inline, hook) = {
            let mut hooks = locked(&self.hooks);
            let mut take = |key: &str| {
                hooks
                    .get_mut(key)
                    .filter(|queue| !queue.is_empty())
                    .map(|queue| queue.remove(0))
            };
            match take(&format!("inline:{label}")) {
                Some(hook) => (true, Some(hook)),
                None => (false, take(label)),
            }
        };
        let Some(hook) = hook else {
            return;
        };
        if inline || !crate::dictation_executor::executor::in_core() {
            hook();
        } else {
            // A hook stands for another thread's event landing exactly at this
            // port (PD-1's user stop, a supervisor notification). Fired from
            // inside a core span it is an *independent* input (D-S2-2): it runs
            // on its own thread, joined at once so the causal order is the one the
            // traces record, exactly as `notify` does, and the production
            // re-entrancy check stays unrelaxed.
            //
            // The join is bounded: a hook that blocks on a lock its caller holds (D
            // held through a cancel's tail, say) fails here at the bound, not never.
            join_bounded(std::thread::spawn(hook), "the independent hook");
        }
    }

    /// Run `hook` once on the calling thread when the bridge's S12 tail reaches
    /// the point after it released D and before its keyboard/emit statements
    /// (`recording.rs`, `run_s12_tail_hook`; compiled into test builds only).
    pub(super) fn on_s12_tail(&self, hook: impl FnOnce() + 'static) {
        S12_TAIL.with(|slot| {
            let previous = slot.borrow_mut().replace(Box::new(hook));
            assert!(previous.is_none(), "an S12 tail hook is already set");
        });
    }

    pub(super) fn close_gate(&self, label: &str) {
        locked(&self.gates).insert(label.to_string(), Arc::new(Gate::default()));
    }

    pub(super) fn open_gate(&self, label: &str) {
        let gate = locked(&self.gates).get(label).cloned();
        gate.unwrap_or_else(|| panic!("no gate `{label}`"))
            .release();
        self.push(format!("gate {label} opened"));
    }

    async fn pass_gate(&self, label: &str) {
        let gate = locked(&self.gates).get(label).cloned();
        if let Some(gate) = gate {
            self.push(format!("gate {label} wait"));
            gate.pass().await;
            self.push(format!("gate {label} pass"));
        }
    }

    pub(super) fn knobs(&self) -> std::sync::MutexGuard<'_, Knobs> {
        locked(&self.knobs)
    }

    // ---- the three commands, as futures the scenario drives

    pub(super) fn start(self: &Arc<Self>) -> Fut<Result<serde_json::Value, String>> {
        let world = Arc::clone(self);
        Box::pin(async move {
            let host = Host(Arc::clone(&world));
            let state = world.state();
            commanded_reply(start_native_recording_inner(
                &host,
                state.inner(),
                None,
                None,
                None,
                None,
            ))
            .await
        })
    }

    /// A start that asks for Smart Auto (with `device_name` a fixed microphone too,
    /// which the start refuses before D).
    pub(super) fn start_smart_auto(
        self: &Arc<Self>,
        device_name: Option<String>,
    ) -> Fut<Result<serde_json::Value, String>> {
        let world = Arc::clone(self);
        Box::pin(async move {
            let host = Host(Arc::clone(&world));
            let state = world.state();
            commanded_reply(start_native_recording_inner(
                &host,
                state.inner(),
                device_name,
                Some(SmartAutoRequest {
                    approved_device_ids: vec!["mic-7".to_string()],
                    preferred_device_ids: Vec::new(),
                    allow_continuity: false,
                    require_recent_signal: false,
                }),
                None,
                None,
            ))
            .await
        })
    }

    /// How many Smart Auto resolves the start asked its host for.
    /// How many times the ready publish started its side tasks (`SD`).
    pub(super) fn side_task_spawns(&self) -> usize {
        self.side_task_spawns.load(Ordering::SeqCst)
    }

    pub(super) fn smart_auto_resolves(&self) -> usize {
        self.smart_auto_resolves.load(Ordering::SeqCst)
    }

    pub(super) fn stop(
        self: &Arc<Self>,
        expected: Option<u64>,
    ) -> Fut<Result<serde_json::Value, String>> {
        let world = Arc::clone(self);
        Box::pin(async move {
            let host = Host(Arc::clone(&world));
            let state = world.state();
            commanded_reply(stop_native_recording_inner(
                &host,
                state.inner(),
                expected,
                audio_lifecycle::DictationStopTrigger::Other,
            ))
            .await
        })
    }

    /// A start that carries a Fn/Globe gesture token, so it claims the speculative
    /// capture bound to that token (`audio_lifecycle::speculative::bind`).
    pub(super) fn start_with_gesture(
        self: &Arc<Self>,
        token: &str,
    ) -> Fut<Result<serde_json::Value, String>> {
        let world = Arc::clone(self);
        let token = token.to_string();
        Box::pin(async move {
            let host = Host(Arc::clone(&world));
            let state = world.state();
            commanded_reply(start_native_recording_inner(
                &host,
                state.inner(),
                None,
                None,
                None,
                Some(token),
            ))
            .await
        })
    }

    /// The self-dispatched stop of an interrupted capture, carrying the real `AutoStop`
    /// owner the bridge registered (R215.2 `RS`), as production's spawned task does.
    /// Returns the owner's token, for its generation and retirement, with the stop.
    /// Needs `carry_auto_stop_owner`.
    pub(super) fn owned_auto_stop(
        self: &Arc<Self>,
        recording_id: u64,
    ) -> (
        murmur_dictation_core::decision::Continuation,
        Fut<Result<serde_json::Value, String>>,
    ) {
        let (token, task) = {
            let mut tasks = locked(&self.auto_stop_tasks);
            let at = tasks
                .iter()
                .position(|(id, _, _)| *id == recording_id)
                .expect("the bridge scheduled an auto-stop task for this recording");
            let (_, token, task) = tasks.remove(at);
            (token, task)
        };
        // The task is the one production spawns, built by the same
        // `auto_stop_dispatch`; this only unwraps its re-entrancy refusal.
        // With the shadow, through the one function production runs it through, so its
        // flights conclude with the result it returns.
        #[cfg(feature = "dictation-shadow")]
        let stop: Fut<Result<serde_json::Value, String>> = Box::pin(async move {
            crate::commands::recording::witness_detached(task)
                .await
                .expect("the auto-stop was not refused as re-entrant")
        });
        #[cfg(not(feature = "dictation-shadow"))]
        let stop: Fut<Result<serde_json::Value, String>> = Box::pin(async move {
            task.await
                .expect("the auto-stop was not refused as re-entrant")
        });
        (token, stop)
    }

    pub(super) fn cancel(self: &Arc<Self>, expected: Option<u64>) -> Fut<Result<(), String>> {
        let world = Arc::clone(self);
        Box::pin(async move {
            let host = Host(Arc::clone(&world));
            let state = world.state();
            commanded_reply(cancel_native_recording_inner(
                &host,
                state.inner(),
                expected,
            ))
            .await
        })
    }

    /// Log what a command replied.
    pub(super) fn reply(&self, label: &str, result: &Result<serde_json::Value, String>) {
        match result {
            Ok(value) => self.push(format!("reply {label} Ok {value}")),
            Err(error) => self.push(format!("reply {label} Err {error}")),
        }
    }

    pub(super) fn reply_unit(&self, label: &str, result: &Result<(), String>) {
        match result {
            Ok(()) => self.push(format!("reply {label} Ok")),
            Err(error) => self.push(format!("reply {label} Err {error}")),
        }
    }

    // ---- fixtures

    /// Install the immutable recording context a start would have installed.
    pub(super) fn install_context(&self, recording_id: u64) {
        let state = self.state();
        let destination = select_recording_destination(
            &crate::frontmost::DeliveryTargetSnapshot::Incomplete,
            crate::smart_destination::Snapshot::Disabled,
        );
        state.app_state.set_active_context(
            recording_id,
            resolve_recording_context(&state.app_state, destination),
        );
    }

    /// A fixture poke: a scenario writes the live status itself, to put an end state in
    /// order or to stand in for another entry's write. It is not a production path, so
    /// the persisted M1 is updated with it. It never forgives an earlier violation: the
    /// divergence counter and both persisted fields are checked against live, under the
    /// dictation lock, *before* the poke changes anything, and only the field the
    /// fixture means to change is updated (M2 is left exactly as it was).
    pub(super) fn poke_status(&self, status: DictationStatus) {
        let state = self.state();
        let mut dictation = state.app_state.dictation.lock_or_recover();
        self.assert_no_new_divergence("poke_status");
        self.assert_core_equals_live(&state.app_state, &dictation, "poke_status");
        dictation.status = status;
        dictation
            .core
            .set_status(crate::dictation_admission::core_status(status));
    }

    /// [`World::poke_status`] for the recording id (M2). The only way to set the
    /// persisted id is the adapter's seeder, which also re-reads M1: M1 has just been
    /// checked equal to live, so that changes nothing but M2.
    pub(super) fn poke_recording_id(&self, id: u64) {
        let state = self.state();
        let mut dictation = state.app_state.dictation.lock_or_recover();
        self.assert_no_new_divergence("poke_recording_id");
        self.assert_core_equals_live(&state.app_state, &dictation, "poke_recording_id");
        state.app_state.recording_id.store(id, Ordering::SeqCst);
        crate::dictation_executor::FieldAdapter::seed_core(&state.app_state, &mut dictation);
    }

    /// An attempt the telemetry ledger accepted earlier, with no terminal yet.
    pub(super) fn accept_silently(&self, recording_id: u64) {
        self.state()
            .app_state
            .dictation_telemetry
            .accepted(recording_id);
    }

    /// The trace so far.
    pub(super) fn trace(&self) -> Vec<String> {
        self.sample();
        locked(&self.log).clone()
    }

    /// What `start_meeting` would be refused with right now (admission only).
    pub(super) fn probe_meeting_conflict(&self) {
        let conflict = crate::commands::meeting::meeting_conflict(&self.state());
        self.push(format!("meeting.conflict -> {conflict:?}"));
    }

    pub(super) fn auto_stop_requests(&self) -> Vec<u64> {
        std::mem::take(&mut *locked(&self.auto_stops))
    }

    // ---- the fake supervisor

    fn set_phase(&self, phase: Phase) {
        locked(&self.supervisor).phase = Some(phase);
        self.push(format!("supervisor.set_phase {phase:?}"));
    }

    fn set_owner(&self, recording_id: Option<u64>) {
        locked(&self.supervisor).owner = recording_id;
    }

    /// A supervisor notification, delivered through the production bridge on
    /// the caller's thread, as `ProductionLifecycleSink::notify` does.
    pub(super) fn notify(&self, recording_id: u64, event: AudioLifecycleEvent) {
        let this = self
            .me
            .upgrade()
            .expect("the world is alive while it notifies");
        self.push(format!(
            "supervisor.notify {recording_id} {}",
            event_name(&event)
        ));
        let ends_the_attempt = matches!(event, AudioLifecycleEvent::Idle);
        let world = this;
        let deliver = move || {
            // A weak handle: the queued task outlives a scenario that panics before
            // it fetches the task, and an `Arc` here would keep the world, which owns
            // the task, alive for ever (and the task's `AutoStop` owner with it).
            let host_world = Arc::downgrade(&world);
            handle_audio_lifecycle_events(
                world.app.clone(),
                recording_id,
                event,
                // Production's `auto_stop_dispatch`, with this world's host and a
                // scheduler that keeps the task instead of spawning it.
                auto_stop_dispatch(
                    move |_, _| {
                        Host(
                            host_world
                                .upgrade()
                                .expect("the world is alive when its task is polled"),
                        )
                    },
                    |recording_id, token, stop| {
                        world.push(format!("bridge.auto_stop_scheduled {recording_id}"));
                        locked(&world.auto_stops).push(recording_id);
                        // Unless a scenario carries it, the task drops here, and its
                        // `AutoStop` owner is retired, as an unrun spawn's would be.
                        let carry = world.knobs().carry_auto_stop_owner;
                        if carry {
                            locked(&world.auto_stop_tasks).push((recording_id, token, stop));
                        }
                    },
                ),
                |app, recording_id, register| {
                    // The two spawns `handle_audio_lifecycle` makes for a `Ready`.
                    // The ticker's own preconditions are production's: a recorded
                    // context and a model that supports a live preview.
                    let state = app.state::<State>();
                    if !ticker_can_start(&state.app_state, recording_id) {
                        return;
                    }
                    let registered = register();
                    world.push(format!("SD ticker {recording_id} registered={registered}"));
                },
            );
        };
        if crate::dictation_executor::executor::in_core() {
            // The fake supervisor calls the bridge from inside a family's port
            // call, on the thread that is running the family's executor. In
            // production the supervisor's own thread delivers its events, so this
            // is an *independent* input (decision D-S2-2): it runs on its own
            // thread, joined at once so the causal order is the one the golden
            // traces record, and the production re-entrancy check stays
            // unrelaxed (the same thread's nested entry is still refused).
            //
            // The delivery takes D. A cancel (or any entry) that is still holding D
            // here would block on the join below for ever, so that is a failure
            // named here, not a hang.
            assert!(
                !matches!(
                    self.state().app_state.dictation.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ),
                "the dictation lock is held while the supervisor notification is delivered: \
                 the delivery takes it, so the join below would never return"
            );
            let delivery = std::thread::spawn(deliver);
            join_bounded(delivery, "the supervisor's delivery thread");
        } else {
            deliver();
        }
        // `finish_attempt` sends `Idle` only when no sealed recording was
        // delivered, and then clears the owner without a `CleanupFinished`. A
        // scenario that sends `Idle` for an attempt whose stop already returned
        // samples (PD-7, PD-2) therefore ends that attempt here instead of
        // leaving a sealed tail the supervisor could never also run.
        let sealed = {
            let mut supervisor = locked(&self.supervisor);
            if ends_the_attempt && supervisor.sealed_pending == Some(recording_id) {
                supervisor.sealed_pending = None;
                true
            } else {
                false
            }
        };
        if sealed {
            self.set_owner(None);
            self.push(format!("supervisor.clear_owner {recording_id}"));
            self.set_phase(Phase::Idle);
        }
    }

    /// Cold start: the supervisor accepts the attempt and brings the worker up
    /// (C9.5 step 1). Runs inside the start port, on the supervisor's side.
    fn cold_start(&self, recording_id: u64) {
        self.set_owner(Some(recording_id));
        self.set_phase(Phase::Starting);
        self.notify(recording_id, AudioLifecycleEvent::Accepted);
        let policy = self.knobs().cold_start;
        match policy {
            ColdStart::ReadyBeforeContext => {
                self.set_phase(Phase::Recording);
                self.notify(recording_id, AudioLifecycleEvent::Ready);
            }
            ColdStart::ReadyAfterContext => {
                locked(&self.supervisor).held_ready = Some((recording_id, true));
                self.push(format!("supervisor.hold_before_ready {recording_id}"));
            }
            ColdStart::HoldBeforeReady => {
                self.set_phase(Phase::Recording);
                locked(&self.supervisor).held_ready = Some((recording_id, false));
                self.push(format!("supervisor.hold_before_ready {recording_id}"));
            }
        }
    }

    /// Release `hold_before_ready`: the supervisor goes on to notify `Ready`.
    pub(super) fn release_ready(&self) {
        let (recording_id, set_phase) = locked(&self.supervisor)
            .held_ready
            .take()
            .expect("no Ready is held");
        self.push(format!("supervisor.release_ready {recording_id}"));
        if set_phase {
            self.set_phase(Phase::Recording);
        }
        self.notify(recording_id, AudioLifecycleEvent::Ready);
    }

    /// A start the supervisor accepted whose response it has not sent yet:
    /// `handle_start` installed the attempt, `set_owner`, `set_phase(Starting)`, then
    /// `notify(Accepted)` (`audio_lifecycle.rs:1481-1588`).
    pub(super) fn accepted_start(&self, recording_id: u64) {
        self.set_owner(Some(recording_id));
        self.set_phase(Phase::Starting);
        self.notify(recording_id, AudioLifecycleEvent::Accepted);
    }

    /// `begin_recovery(reason)` of an attempt: `set_phase(Recovering)`, then
    /// `notify(Recovering{reason})` (`audio_lifecycle.rs:2672-2689`).
    pub(super) fn recovering(&self, recording_id: u64, reason: AudioCancelReason) {
        self.set_phase(Phase::Recovering);
        self.notify(recording_id, AudioLifecycleEvent::Recovering { reason });
    }

    /// The worker failed at runtime: `Recovering{RuntimeFailure}`.
    pub(super) fn runtime_failure(&self, recording_id: u64) {
        self.set_phase(Phase::Recovering);
        self.notify(
            recording_id,
            AudioLifecycleEvent::Recovering {
                reason: AudioCancelReason::RuntimeFailure,
            },
        );
    }

    /// `finish_attempt` for a recovered runtime failure with preserved audio
    /// (`audio_lifecycle.rs` `AttemptPhase::Recovering`): park the capture,
    /// notify `Interrupted`, then stop at `hold_before_owner_clear`. No `Idle`
    /// follows an `Interrupted` end.
    pub(super) fn finish_interrupted(&self, recording_id: u64, samples: usize) {
        audio_lifecycle::insert_interrupted_dictation_for_test(
            recording_id,
            audio_lifecycle::InterruptedCapture {
                samples: vec![0.0; samples],
                reason: "stream_invalidated".to_string(),
                duration_ms: samples as u64 / 16,
            },
        );
        self.notify(
            recording_id,
            AudioLifecycleEvent::Interrupted {
                reason: AudioCancelReason::RuntimeFailure,
                delivered_samples: samples as u64,
                duration_ms: samples as u64 / 16,
            },
        );
        locked(&self.supervisor).held_owner_clear = Some(recording_id);
        self.push(format!("supervisor.hold_before_owner_clear {recording_id}"));
    }

    /// Park the supervisor just before `clear_owner` (`audio_lifecycle.rs:2460`)
    /// with the owner and its phase untouched: the gap `finish_attempt` has
    /// between its last notification and the owner clear (PD-18 (b), (c)).
    pub(super) fn hold_owner_clear(&self, recording_id: u64) {
        locked(&self.supervisor).held_owner_clear = Some(recording_id);
        self.push(format!("supervisor.hold_before_owner_clear {recording_id}"));
    }

    /// Another recording's attempt replaced the owner while a stop for `recording_id` was
    /// queued: the supervisor installed a new attempt (`handle_start`, `set_owner`).
    pub(super) fn replace_owner(&self, recording_id: u64) {
        self.set_owner(Some(recording_id));
        // Production installs the new attempt in `Starting` (`handle_start`); the
        // replaced attempt's phase does not carry over to it.
        locked(&self.supervisor).phase = Some(Phase::Starting);
        self.push(format!("supervisor.replace_owner {recording_id}"));
    }

    /// Release `hold_before_owner_clear` (`audio_lifecycle.rs:2460-2461`).
    pub(super) fn release_owner_clear(&self) {
        let recording_id = locked(&self.supervisor)
            .held_owner_clear
            .take()
            .expect("no owner clear is held");
        self.set_owner(None);
        self.push(format!("supervisor.clear_owner {recording_id}"));
        self.set_phase(Phase::Idle);
    }

    /// A cancelled attempt's own `Idle` (`finish_attempt`, `Recovering` arm,
    /// no standby): notify `Idle`, then clear the owner.
    pub(super) fn release_cancel_finish(&self) {
        let recording_id = locked(&self.supervisor)
            .held_cancel_finish
            .take()
            .expect("no cancel finish is held");
        self.push(format!("supervisor.release_cancel_finish {recording_id}"));
        self.notify(recording_id, AudioLifecycleEvent::Idle);
        self.set_owner(None);
        self.push(format!("supervisor.clear_owner {recording_id}"));
        self.set_phase(Phase::Idle);
    }

    /// A sealed stop's cleanup tail (`finish_attempt`, `audio_lifecycle.rs:
    /// 2457-2467`): clear the owner, publish `Idle`, then send `CleanupFinished`.
    /// A scenario runs it mid-run where production's order matters; otherwise
    /// [`World::finish`] runs it at the end.
    pub(super) fn finish_sealed_stop(&self, recording_id: u64) {
        let pending = locked(&self.supervisor).sealed_pending.take();
        assert_eq!(pending, Some(recording_id), "no sealed stop for this id");
        self.set_owner(None);
        self.push(format!("supervisor.clear_owner {recording_id}"));
        self.set_phase(Phase::Idle);
        self.notify(recording_id, AudioLifecycleEvent::CleanupFinished);
    }

    // ---- the delivery fence (C9.4 scenario 2)

    /// The pipeline's delivery step: a thread takes D through the production
    /// `with_current_delivery_owner` (the helper that makes cancellation atomic
    /// with the clipboard write and paste) and parks inside it until
    /// [`World::finish_delivery`]. Returns once D is held (`true`), or `false`
    /// when the owner check refused and the operation never ran.
    fn begin_delivery(self: &Arc<Self>, recording_id: u64) -> bool {
        let (entered, entered_rx) = std::sync::mpsc::channel::<()>();
        let (release, release_rx) = std::sync::mpsc::channel::<()>();
        let world = Arc::clone(self);
        let thread = std::thread::spawn(move || {
            let state = world.state();
            with_current_delivery_owner(&state.app_state, recording_id, || {
                world.push(format!(
                    "delivery {recording_id}: holds the ownership fence D"
                ));
                let _ = entered.send(());
                await_release(&release_rx);
                world.push(format!(
                    "delivery {recording_id}: operation done, D released next"
                ));
            })
        });
        if await_arrival(&entered_rx, &thread, "delivery").is_err() {
            // The owner check refused: the operation never ran (a panic is re-raised).
            join_bounded(thread, "delivery");
            self.push(format!("delivery {recording_id}: refused, not the owner"));
            return false;
        }
        *locked(&self.delivery) = Some(Delivery { release, thread });
        true
    }

    /// Let a parked delivery return and release D, without waiting for anything.
    fn release_delivery(&self) {
        if let Some(delivery) = locked(&self.delivery).take() {
            let _ = delivery.release.send(());
        }
    }

    /// Let the parked delivery operation return and release D, then wait for
    /// `cancel` (blocked on D) to finish. Nothing is logged between the release
    /// and the joins: the two threads' lines are ordered by D alone.
    pub(super) fn finish_delivery(&self, mut cancel: ConcurrentCancel) -> Result<(), String> {
        let delivery = locked(&self.delivery).take().expect("a parked delivery");
        let _ = delivery.release.send(());
        let reply = join_bounded(
            cancel.thread.take().expect("a live cancel"),
            "the concurrent cancel",
        );
        let delivered = join_bounded(delivery.thread, "the parked delivery");
        self.push(format!(
            "delivery returned {}",
            if delivered.is_some() { "Some" } else { "None" }
        ));
        reply
    }

    /// Run a cancel on its own thread: it blocks in the production `D` lock
    /// while a delivery holds D.
    pub(super) fn cancel_on_thread(self: &Arc<Self>) -> ConcurrentCancel {
        let world = Arc::clone(self);
        ConcurrentCancel {
            world: Arc::clone(&world),
            thread: Some(std::thread::spawn(move || {
                let _runtime = runtime().enter();
                let mut cancel = world.cancel(None);
                complete_joining(&mut cancel)
            })),
        }
    }

    /// The fake supervisor's phase when `recording_id` owns it.
    pub(super) fn dictation_owner_phase(&self, recording_id: u64) -> Option<Phase> {
        let supervisor = locked(&self.supervisor);
        (supervisor.owner == Some(recording_id)).then(|| supervisor.phase())
    }
}

impl World {
    /// Run a cancel on its own thread and park it just before the executor runs the
    /// effect whose legend code is `code` (`EM`, `TERM`, ...): the thread blocks at that
    /// barrier, holding whatever locks the cancel holds there, until
    /// [`ParkedCancel::release_and_join`].
    pub(super) fn cancel_parked_before(self: &Arc<Self>, code: &'static str) -> ParkedCancel {
        let (parked, parked_rx) = std::sync::mpsc::channel::<()>();
        let (release, release_rx) = std::sync::mpsc::channel::<()>();
        let world = Arc::clone(self);
        let thread = std::thread::spawn(move || {
            let _runtime = runtime().enter();
            let _barrier = crate::barrier_hook::install(move |point| {
                if point == code {
                    let _ = parked.send(());
                    // A scenario that panics first drops `release`: the cancel then
                    // runs on instead of hanging the test.
                    await_release(&release_rx);
                }
            });
            let mut cancel = world.cancel(None);
            complete_joining(&mut cancel)
        });
        ParkedCancel {
            thread: Some(thread),
            parked: parked_rx,
            release,
        }
    }
}

/// A cancel parked at an executor barrier (see [`World::cancel_parked_before`]).
pub(super) struct ParkedCancel {
    /// `None` once joined. A cancel still parked when the scenario unwinds is released
    /// by the sender dropping with this struct, so it never outlives the test blocked.
    thread: Option<std::thread::JoinHandle<Result<(), String>>>,
    parked: std::sync::mpsc::Receiver<()>,
    release: std::sync::mpsc::Sender<()>,
}

impl ParkedCancel {
    /// Block until the cancel has reached its barrier.
    pub(super) fn wait_parked(&mut self) {
        if await_arrival(
            &self.parked,
            self.thread.as_ref().expect("a live thread"),
            "cancel",
        )
        .is_err()
        {
            helper_failed(self.thread.take().expect("a live thread"), "cancel");
        }
    }

    /// Let the cancel run on and return its reply.
    pub(super) fn release_and_join(mut self) -> Result<(), String> {
        let _ = self.release.send(());
        join_bounded(self.thread.take().expect("a live thread"), "cancel")
    }
}

/// A cancel running on its own thread.
pub(super) struct ConcurrentCancel {
    /// `None` once joined.
    thread: Option<std::thread::JoinHandle<Result<(), String>>>,
    world: Arc<World>,
}

impl Drop for ConcurrentCancel {
    /// A scenario that fails while this cancel is blocked on D unwinds through the
    /// locals it declared before the cancel, and some of their drops take D. The
    /// delivery that holds D is released first, here, as soon as the cancel drops.
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.world.release_delivery();
        }
    }
}

impl ConcurrentCancel {
    fn live(&self) -> &std::thread::JoinHandle<Result<(), String>> {
        self.thread.as_ref().expect("a live cancel")
    }

    /// Wait until the cancel holds the recording transition and so is at (or a
    /// statement from) its `D` lock: the transition is released only at the
    /// end of the command, so a held transition with an unfinished command is
    /// a cancel that has not got past D. No clock: this spins on the lock
    /// state, and gives up if the command finished.
    pub(super) fn wait_until_blocked_on_d(&self, world: &World) {
        spin_bounded("the concurrent cancel", || {
            !self.live().is_finished()
                && world
                    .state()
                    .app_state
                    .recording_transition
                    .try_lock()
                    .is_ok()
        });
        assert!(
            !self.live().is_finished(),
            "the cancel finished while a delivery held the ownership fence"
        );
    }
}

fn event_name(event: &AudioLifecycleEvent) -> String {
    match event {
        AudioLifecycleEvent::StartupDiagnostic(_) => "StartupDiagnostic".to_string(),
        AudioLifecycleEvent::Accepted => "Accepted".to_string(),
        AudioLifecycleEvent::Ready => "Ready".to_string(),
        AudioLifecycleEvent::StillConnecting => "StillConnecting".to_string(),
        AudioLifecycleEvent::Recovering { reason } => format!("Recovering({})", reason.as_str()),
        AudioLifecycleEvent::InitializationFailed { kind, .. } => {
            format!("InitializationFailed({})", kind.as_str())
        }
        AudioLifecycleEvent::CleanupFinished => "CleanupFinished".to_string(),
        AudioLifecycleEvent::RecoveryStalled => "RecoveryStalled".to_string(),
        AudioLifecycleEvent::Interrupted {
            reason,
            delivered_samples,
            ..
        } => format!(
            "Interrupted({}, {delivered_samples} samples)",
            reason.as_str()
        ),
        AudioLifecycleEvent::Idle => "Idle".to_string(),
    }
}

// ------------------------------------------------------------ the ports

impl SupervisorPort for World {
    fn start_dictation_recording(
        &self,
        _device_id: Option<String>,
        recording_id: u64,
        origin: &str,
        _gesture: audio_lifecycle::gesture::StartGesture,
    ) -> Result<(), AudioStartError> {
        self.push(format!("supervisor.start {recording_id} {origin}"));
        self.fire("supervisor.start");
        self.supervisor_start(recording_id)
    }

    fn start_dictation_recording_after_preview(
        &self,
        _device_id: Option<String>,
        recording_id: u64,
        origin: &str,
        _gesture: audio_lifecycle::gesture::StartGesture,
    ) -> Result<(), AudioStartError> {
        self.push(format!(
            "supervisor.start_after_preview {recording_id} {origin}"
        ));
        self.fire("supervisor.start");
        self.supervisor_start(recording_id)
    }

    fn stop_dictation_recording(
        &self,
        recording_id: u64,
        trigger: audio_lifecycle::DictationStopTrigger,
        _released_at: Option<std::time::Instant>,
    ) -> Result<Vec<f32>, String> {
        self.push(format!("supervisor.stop {recording_id} {trigger:?}"));
        // The supervisor handles the `Stop` message against whatever attempt exists
        // then, and moves its own attempt to `Stopping` only when that attempt is the
        // one being stopped. A stop that finds another recording's attempt is refused
        // and leaves that attempt's phase alone.
        let replaced = matches!(
            locked(&self.supervisor).owner,
            Some(current) if current != recording_id
        );
        if !replaced {
            self.set_phase(Phase::Stopping);
        }
        self.fire("supervisor.stop");
        let (result, unsealed, transport_failure) = {
            let knobs = self.knobs();
            (
                knobs.stop_result,
                knobs.stop_unsealed,
                knobs.stop_transport_failure,
            )
        };
        // A transport failure (the 12 s receive expired, the channel closed) is the
        // caller's own error: the supervisor's ownership check and scripted replies
        // are not consulted, and it is returned whatever the owner is.
        if let Some(error) = transport_failure {
            return Err(error.to_string());
        }
        // Reply precedence (production, `audio_lifecycle.rs:1235-1245`): the owner is
        // checked first, so a scripted supervisor reply (a recovery refusal, a stop
        // timeout the supervisor itself reports) never masks "Audio owner changed".
        // With no attempt (the owner was cleared) it answers `Ok(Vec::new())` at once:
        // nothing is sealed, so no `CleanupFinished` follows and no owner clear is
        // pending. With another recording's attempt (the owner was replaced) it refuses:
        // `Audio owner changed before stop`.
        let owner = locked(&self.supervisor).owner;
        match owner {
            Some(current) if current == recording_id => {}
            None => {
                self.push(format!(
                    "supervisor.stop_after_owner_clear {recording_id} samples=0"
                ));
                return Ok(Vec::new());
            }
            Some(other) => {
                self.push(format!(
                    "supervisor.stop_owner_changed {recording_id} owner={other}"
                ));
                return Err("Audio owner changed before stop".to_string());
            }
        }
        match result {
            Ok(samples) => {
                if unsealed {
                    // The reply is delivered without `RecordingSealed`
                    // (`audio_lifecycle.rs:2361-2362`): no `CleanupFinished`
                    // follows, and the supervisor parks before `clear_owner`
                    // (PD-18 (c)).
                    self.hold_owner_clear(recording_id);
                } else {
                    // `RecordingSealed`: the samples leave the supervisor, which
                    // keeps the owner in `Stopping` until the worker exit is
                    // confirmed (`finish_sealed_stop`).
                    locked(&self.supervisor).sealed_pending = Some(recording_id);
                    self.push(format!(
                        "supervisor.sealed_delivery {recording_id} samples={samples}"
                    ));
                }
                Ok(vec![0.0; samples])
            }
            Err(error) => Err(error.to_string()),
        }
    }

    fn cancel_dictation_capture(
        &self,
        recording_id: u64,
        reason: AudioCancelReason,
    ) -> Result<bool, String> {
        self.push(format!(
            "supervisor.cancel {recording_id} {}",
            reason.as_str()
        ));
        self.fire("supervisor.cancel");
        let result = self.knobs().cancel_result;
        if let Ok(true) = result {
            // The supervisor handles `Cancel{reason}`: Recovering is published
            // at once, the attempt's own `Idle` follows when the worker joins.
            locked(&self.supervisor).held_ready = None;
            self.set_phase(Phase::Recovering);
            self.notify(recording_id, AudioLifecycleEvent::Recovering { reason });
            locked(&self.supervisor).held_cancel_finish = Some(recording_id);
            self.push(format!("supervisor.hold_cancel_finish {recording_id}"));
        }
        result.map_err(str::to_string)
    }

    fn take_interrupted_dictation(
        &self,
        recording_id: u64,
    ) -> Option<audio_lifecycle::InterruptedCapture> {
        let capture = audio_lifecycle::take_interrupted_dictation(recording_id);
        self.push(format!(
            "supervisor.take_interrupted {recording_id} -> {}",
            capture.as_ref().map_or_else(
                || "none".to_string(),
                |capture| format!("{} samples", capture.samples.len())
            )
        ));
        capture
    }

    fn is_dictation_cleanup_pending(&self, recording_id: u64) -> bool {
        let pending = matches!(
            self.dictation_owner_phase(recording_id),
            Some(Phase::Stopping | Phase::Recovering)
        );
        self.push(format!(
            "supervisor.is_dictation_cleanup_pending {recording_id} -> {pending}"
        ));
        pending
    }
}

impl World {
    /// `audio_lifecycle::is_dictation_recording` on the fake's phase.
    fn is_dictation_recording(&self, recording_id: u64) -> bool {
        let recording = self.dictation_owner_phase(recording_id) == Some(Phase::Recording);
        self.push(format!(
            "supervisor.is_dictation_recording {recording_id} -> {recording}"
        ));
        // A point where a scenario lands an event after the read and before D
        // (R104.5's mutation witness).
        self.fire("supervisor.is_dictation_recording");
        recording
    }
}

impl World {
    fn supervisor_start(&self, recording_id: u64) -> Result<(), AudioStartError> {
        let failure = self.knobs().start_ack.take();
        if let Some(error) = failure {
            if self.knobs().accept_before_failing {
                self.set_owner(Some(recording_id));
                self.notify(recording_id, AudioLifecycleEvent::Accepted);
            }
            let before_ack = std::mem::take(&mut self.knobs().before_ack);
            for script in before_ack {
                self.notify(recording_id, script.event());
            }
            self.set_owner(None);
            self.push(format!("supervisor.start_ack {recording_id} Err({error})"));
            return Err(error);
        }
        self.cold_start(recording_id);
        self.push(format!("supervisor.start_ack {recording_id} Ok"));
        Ok(())
    }
}

impl KeyboardPort for World {
    fn is_app_disabled(&self) -> bool {
        self.push("keyboard.is_app_disabled");
        self.knobs().app_disabled
    }

    fn listener_generation(&self) -> u64 {
        self.push("keyboard.listener_generation");
        0
    }

    fn set_processing(&self, processing: bool) {
        self.push(format!("keyboard.set_processing {processing}"));
        keyboard::set_processing(processing);
        self.fire(&format!("keyboard.set_processing:{processing}"));
    }

    fn hotkey_release_take_for(
        &self,
        recording_id: u64,
        _stop_entered_at: std::time::Instant,
    ) -> Option<crate::hotkey_release::HotkeyRelease> {
        self.push(format!("keyboard.hotkey_release_take_for {recording_id}"));
        None
    }

    fn stop_gesture_take_for(
        &self,
        recording_id: u64,
        _entered: std::time::Instant,
    ) -> Option<crate::stop_gesture::StopStamp> {
        self.push(format!("keyboard.stop_gesture_take_for {recording_id}"));
        // The last port before the stop reads the recording context (S7).
        self.fire("keyboard.stop_gesture_take_for");
        None
    }
}

impl TargetSnapshotPort for World {
    fn capture_delivery_target_snapshot(&self) -> crate::frontmost::DeliveryTargetSnapshot {
        self.push("ax.capture_delivery_target_snapshot");
        crate::frontmost::DeliveryTargetSnapshot::Incomplete
    }
}

impl SmartDestinationPort for World {
    fn capture<'a>(
        &'a self,
        _expected: &'a crate::frontmost::DeliveryTargetSnapshot,
        enabled: bool,
    ) -> HostFuture<'a, crate::smart_destination::Snapshot> {
        self.push(format!("smart_destination.capture enabled={enabled}"));
        Box::pin(async move {
            self.pass_gate("smart_capture").await;
            if enabled {
                crate::smart_destination::Snapshot::Unavailable
            } else {
                crate::smart_destination::Snapshot::Disabled
            }
        })
    }
}

impl PerformancePort for World {
    fn register_production(
        &self,
        recording_id: u64,
        selection: crate::performance_metrics::production::MicrophoneSelectionV1,
        _kind: Option<crate::performance_metrics::production::MicrophoneKindV1>,
    ) {
        self.push(format!(
            "performance.register_production {recording_id} {selection:?}"
        ));
    }

    fn production_context(&self, recording_id: u64, _context: &DictationContextSnapshot) {
        self.push(format!("performance.production_context {recording_id}"));
    }

    fn production_sample_count(&self, recording_id: u64, count: usize) {
        self.push(format!(
            "performance.production_sample_count {recording_id} {count}"
        ));
    }

    // The begin ledger is the production one: `perf-claim` lines in the trace
    // are what it holds, so a completion that loses a claim is visible.
    fn begin_register(&self, recording_id: u64) {
        self.push(format!("performance.begin_register {recording_id}"));
        dictation_performance_begins().register(recording_id);
    }

    fn begin_abort(&self, recording_id: u64) {
        self.push(format!("performance.begin_abort {recording_id}"));
        dictation_performance_begins().abort(recording_id);
    }

    fn spawn_begin(
        &self,
        recording_id: u64,
        _runtimes: Vec<RuntimeIdentityV1>,
        _started_at_ms: i64,
    ) {
        // The begin succeeds at once (no store), exactly as the spawned task
        // resolves it: a completion that arrived while the row was `Beginning`
        // is claimed now, and every later one is `AlreadyCompleting`. The
        // store is uninitialized, so persisting a claimed completion fails
        // and the claim stays visible as a `perf-claim` line.
        self.push(format!("performance.spawn_begin {recording_id}"));
        let coordinator = dictation_performance_begins();
        if let Some(resolution) = coordinator.resolve_begin(recording_id, true) {
            flush_dictation_performance_begin(
                coordinator,
                &self.state().performance,
                recording_id,
                resolution,
            );
        }
    }

    fn complete(
        &self,
        recording_id: u64,
        outcome: RunOutcomeV1,
        stages: Vec<StageTimingV1>,
        input: Option<ContentFreeInputSummaryV1>,
        runtimes: Option<Vec<RuntimeIdentityV1>>,
    ) {
        self.push(format!(
            "performance.complete {recording_id} {}",
            serde_json::to_string(&outcome).expect("outcome serializes")
        ));
        complete_dictation_performance(
            &self.state().performance,
            recording_id,
            outcome,
            stages,
            input,
            runtimes,
        );
    }
}

/// The `DictationHost` the commands run against.
pub(super) struct Host(pub(super) Arc<World>);

impl DictationHost for Host {
    fn emit_json(&self, event: &str, payload: serde_json::Value) -> Result<(), HostEmitError> {
        self.0.push(format!("emit {event} {payload}"));
        self.0.fire(&format!("emit:{event}"));
        Ok(())
    }

    fn supervisor(&self) -> Arc<dyn SupervisorPort> {
        Arc::clone(&self.0) as Arc<dyn SupervisorPort>
    }

    fn keyboard(&self) -> &dyn KeyboardPort {
        &*self.0
    }

    fn ax(&self) -> &dyn TargetSnapshotPort {
        &*self.0
    }

    fn smart_destination(&self) -> &dyn SmartDestinationPort {
        &*self.0
    }

    fn performance(&self) -> &dyn PerformancePort {
        &*self.0
    }

    fn microphone_transition<'a>(
        &'a self,
        state: &'a State,
    ) -> HostFuture<'a, Result<tokio::sync::MutexGuard<'a, ()>, String>> {
        self.0.push("mic_transition");
        Box::pin(async move { Ok(state.app_state.recording_transition.lock().await) })
    }

    fn resolve_smart_auto(
        &self,
        _request: &SmartAutoRequest,
    ) -> Result<crate::microphone_auto::SmartAutoSelection, String> {
        // Counted outside the trace so no golden line changes: no scenario asks for
        // Smart Auto, and only the contract tests read the count.
        self.0.smart_auto_resolves.fetch_add(1, Ordering::SeqCst);
        let result = locked(&self.0.knobs)
            .smart_auto
            .clone()
            .unwrap_or_else(|| Err("smart auto is not scripted".to_string()));
        if result.is_ok() {
            // Production's success emits `smart-auto-microphone-changed` inline,
            // on this thread, after the commit.
            self.0.fire("smart_auto.routing_changed");
        }
        result
    }

    fn refresh_expired_ide_context(&self, _app_state: &AppState, _bundle_id: Option<&str>) {
        self.0.push("refresh_expired_ide_context");
        // A point between query 402's read and the emit's own D section.
        self.0.fire("refresh_expired_ide_context");
    }

    fn emit_smart_destination(
        &self,
        recording_id: u64,
        _context: &DictationContextSnapshot,
        _accepted_target: &crate::frontmost::DeliveryTargetSnapshot,
    ) -> Result<(), String> {
        self.0
            .push(format!("emit_smart_destination {recording_id}"));
        // An inline listener of the emit (on this thread, under the shell's D hold).
        self.0.fire("smart_destination.emit");
        if locked(&self.0.knobs).smart_destination_emit_error {
            Err("the emit failed".to_string())
        } else {
            Ok(())
        }
    }

    fn publish_recording_ready_if_audio_ready(
        &self,
        state: &State,
        recording_id: u64,
        context_ready_at: std::time::Instant,
        ready_op: crate::core_history::ContinuationGuard,
    ) -> bool {
        let world = &self.0;
        world.push(format!("publish_recording_ready {recording_id}"));
        let published = publish_recording_ready_with(
            &world.app,
            state,
            recording_id,
            context_ready_at,
            ready_op,
            |recording_id| world.is_dictation_recording(recording_id),
            |app, recording_id, register| {
                world.side_task_spawns.fetch_add(1, Ordering::SeqCst);
                let state = app.state::<State>();
                if !ticker_can_start(&state.app_state, recording_id) {
                    return;
                }
                let registered = register();
                world.push(format!("SD ticker {recording_id} registered={registered}"));
            },
        );
        world.push(format!(
            "publish_recording_ready {recording_id} -> {published}"
        ));
        published
    }

    fn spawn_model_preparation(&self, _model_name: String, recording_id: u64) {
        self.0
            .push(format!("spawn_model_preparation {recording_id}"));
    }

    fn claim_preview_hide(
        &self,
        recording_id: u64,
    ) -> Option<crate::commands::dictation_preview::PreviewHideOwner> {
        self.0.push(format!("claim_preview_hide {recording_id}"));
        None
    }

    fn run_transcription_pipeline<'a>(
        &'a self,
        samples: &'a [f32],
        app_state: &'a AppState,
        recording_id: u64,
        _performance_guard: &'a mut NativeDictationPerformanceGuard,
        _context: Arc<DictationContextSnapshot>,
        stop: PipelineStopContext,
    ) -> HostFuture<'a, Result<PipelineResult, String>> {
        let world = &self.0;
        world.push(format!("pipeline {recording_id} samples={}", samples.len()));
        Box::pin(async move {
            let PipelineStopContext { pipeline_op, .. } = stop;
            // `run_transcription_pipeline` builds its `IdleGuard` first and
            // drops it on return (`recording.rs`, the `_guard` at the pipeline's
            // entry): the real pipeline guard, with the pipeline's history
            // entries, so what a return does to status, context and history is
            // production's.
            let mut guard = IdleGuard::for_pipeline(app_state, recording_id, pipeline_op);
            world.fire("pipeline");
            let hold_delivery = world.knobs().hold_delivery;
            let cancelled = |stage| PipelineResult {
                text: String::new(),
                timings: PipelineTimings::default(),
                terminal: PipelineTerminal::Cancelled(stage),
                history: None,
            };
            let succeeded = || PipelineResult {
                text: "scripted transcript".to_string(),
                timings: PipelineTimings::default(),
                terminal: PipelineTerminal::Success,
                history: Some(PipelineHistory {
                    raw_text: "scripted transcript".to_string(),
                    stages: Vec::new(),
                }),
            };
            // Production reads `cancelled_id` only before delivery (the
            // checkpoints at `recording.rs` 2923, 3029 and 3185, the last one
            // just before injection) and again when the delivery owner check
            // refuses (3557, `Cancelled(ClipboardPaste)`). Once a delivery ran
            // under D the pipeline returns Success whatever happened to the
            // cancellation meanwhile (3666-3680).
            let result = if hold_delivery {
                if app_state.is_cancelled(recording_id) {
                    world.push(format!("pipeline {recording_id} cancelled before delivery"));
                    // Pass the gate anyway so a scenario that opens it cannot
                    // wait on a pipeline that already returned.
                    world.pass_gate("pipeline").await;
                    cancelled(PerformanceStageV1::ClipboardPaste)
                } else if world.begin_delivery(recording_id) {
                    // Parked on the delivery's reply.
                    world.pass_gate("pipeline").await;
                    world.push(format!("pipeline {recording_id} returns"));
                    succeeded()
                } else {
                    // Refused by the owner check (3557); same gate rule.
                    world.pass_gate("pipeline").await;
                    world.push(format!("pipeline {recording_id} returns"));
                    cancelled(PerformanceStageV1::ClipboardPaste)
                }
            } else {
                // Modelled as parked in decode, so the next checkpoint is 3185
                // (`ClipboardPaste`). A park in VAD would instead meet 3029 and
                // return `Cancelled(InferenceDecode)`; no scenario traces the
                // pipeline's own cancel stage, so the difference is not shown.
                world.pass_gate("pipeline").await;
                world.push(format!("pipeline {recording_id} returns"));
                if app_state.is_cancelled(recording_id) {
                    cancelled(PerformanceStageV1::ClipboardPaste)
                } else {
                    succeeded()
                }
            };
            let (error, finish_panics) = {
                let knobs = world.knobs();
                (knobs.pipeline_error, knobs.pipeline_finish_panics)
            };
            if let Some(error) = error {
                // An early return: the guard's drop leaves, as on a `?`.
                drop(guard);
                world.push(format!("pipeline {recording_id} failed"));
                return Err(error.to_string());
            }
            // The guard's drop is `finish`: it clears the context, then writes
            // Idle on an id match alone, whatever the status (PD-6).
            let mut wrote_idle = false;
            guard.finish(|_| {
                if finish_panics {
                    // The closure is the guard's `In` effect `EM(idle)`: by now the
                    // decision is made and both history entries are retired.
                    let history = &app_state.core_history;
                    world.push(format!(
                        "pipeline {recording_id} history before In: h2={} h1={}",
                        history.pipeline_count(),
                        history.outstanding_ops()
                    ));
                    panic!("the pipeline's idle closure panics");
                }
                wrote_idle = true;
            });
            world.push(format!(
                "pipeline {recording_id} idle_guard wrote_idle={wrote_idle}"
            ));
            Ok(result)
        })
    }
}

// --------------------------------------------------------------- runner

/// What a sub-run starts from.
pub(super) struct Setup {
    pub(super) status: DictationStatus,
    pub(super) recording_id: u64,
}

impl Setup {
    pub(super) fn idle() -> Self {
        Self {
            status: DictationStatus::Idle,
            recording_id: 0,
        }
    }
}

/// The partial ticker spawner's own preconditions, as the harness's two spawners apply
/// them: with the shadow compiled in they are the production spawner's check
/// (`ticker_start_check`, the one lookup that also hands its skip to the port); without it
/// they are the same two conditions, read here.
fn ticker_can_start(app_state: &AppState, recording_id: u64) -> bool {
    #[cfg(feature = "dictation-shadow")]
    {
        ticker_start_check(app_state, recording_id).is_ok()
    }
    #[cfg(not(feature = "dictation-shadow"))]
    {
        let Some(context) = app_state.active_context(recording_id) else {
            return false;
        };
        let model = &context.transcription.model_name;
        transcriber::whisper::supports_live_preview(model) || transcriber::is_coreml_model(model)
    }
}

/// A command's entry as its IPC wrapper runs it: the reply the command returns is the
/// reply the shadow witnesses (CP4, D-S2c-8 f). Without the shadow it is the command.
pub(super) async fn commanded_reply<T, E, F>(command: F) -> Result<T, E>
where
    F: Future<Output = Result<T, E>>,
    T: ReplyValue,
{
    #[cfg(feature = "dictation-shadow")]
    {
        crate::dictation_executor::shadow::commanded(command).await
    }
    #[cfg(not(feature = "dictation-shadow"))]
    {
        command.await
    }
}

/// What the shadow can read a returned reply as (the replies of the four commands).
#[cfg(feature = "dictation-shadow")]
pub(super) use crate::dictation_executor::shadow::RenderedValue as ReplyValue;
#[cfg(not(feature = "dictation-shadow"))]
pub(super) trait ReplyValue {}
#[cfg(not(feature = "dictation-shadow"))]
impl ReplyValue for () {}
#[cfg(not(feature = "dictation-shadow"))]
impl ReplyValue for serde_json::Value {}

/// Test builds with the shadow compiled in: run sub-runs with the shadow **not** armed
/// (the feature on, `MURMUR_SHADOW` unset), the baseline a measurement compares with.
#[cfg(feature = "dictation-shadow")]
pub(super) static REPLAY_DISARMED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// One named sub-run of a scenario.
pub(super) struct SubRun {
    pub(super) title: &'static str,
    pub(super) setup: Setup,
    pub(super) body: fn(&Arc<World>),
}

fn reset_process_globals() {
    keyboard::set_processing(false);
    crate::commands::dictation_preview::reset_ownership_for_test();
    audio_lifecycle::gesture::clear_pending();
    for recording_id in 0..=MAX_ID {
        let _ = audio_lifecycle::take_interrupted_dictation(recording_id);
        dictation_performance_begins().abort(recording_id);
    }
}

/// Run one sub-run on a fresh app and return its trace lines.
pub(super) fn run_sub_run(sub_run: &SubRun) -> Vec<String> {
    run_sub_run_observed(sub_run, &|_| {})
}

/// [`run_sub_run`], then `observe` runs on the finished world (the trace is already
/// taken; `observe` adds no line). Additive: the Stage 4 replays read test-only
/// observers (the successful terminal claims) that the trace does not carry.
pub(super) fn run_sub_run_observed(sub_run: &SubRun, observe: &dyn Fn(&Arc<World>)) -> Vec<String> {
    let _serial = locked(&GOLDEN_LOCK);
    let _runtime = runtime().enter();
    S12_TAIL.with(|slot| slot.borrow_mut().take());
    reset_process_globals();
    // E7: the real shell runs against the real comparator, on this thread.
    // (A test measuring what the armed shadow adds disarms it for the baseline run.)
    #[cfg(feature = "dictation-shadow")]
    let armed = !REPLAY_DISARMED.load(Ordering::SeqCst);
    #[cfg(feature = "dictation-shadow")]
    let replay = armed.then(shadow_replay::Replay::begin);
    #[cfg(feature = "dictation-shadow")]
    let counters_before = crate::dictation_executor::shadow::conservation_now();
    let app_state = AppState::default();
    app_state
        .recording_id
        .store(sub_run.setup.recording_id, Ordering::SeqCst);
    {
        // The persisted core starts equal to live, as the shell seeds it.
        let mut dictation = app_state.dictation.lock_or_recover();
        dictation.status = sub_run.setup.status;
        crate::dictation_executor::FieldAdapter::seed_core(&app_state, &mut dictation);
    }
    let app = tauri::test::mock_builder()
        .manage(test_state(app_state))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("build mock Tauri app");
    let world = World::new(app.handle().clone());
    let _release_on_unwind = ReleaseOnUnwind(Arc::clone(&world));
    world.listen_to_the_bridge();
    // The first sample is the starting state, not a change.
    {
        let state = world.state();
        let mut seen = locked(&world.seen);
        seen.status = Some(sub_run.setup.status);
        seen.id = state.app_state.recording_id.load(Ordering::SeqCst);
    }
    // C8.7 at the site: every header is submitted with D free, and a blocking effect
    // runs only after its header was compared.
    #[cfg(feature = "dictation-shadow")]
    let probe = armed.then(|| {
        let probed = Arc::clone(&world);
        shadow_replay::SiteProbe::install(move || {
            !matches!(
                probed.state().app_state.dictation.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            )
        })
    });
    (sub_run.body)(&world);
    let lines = world.finish();
    observe(&world);
    #[cfg(feature = "dictation-shadow")]
    drop(probe);
    reset_process_globals();
    // Everything the run drove is dropped before the replay ends, so a flight that
    // did not finish has reported its abandonment.
    drop(world);
    drop(app);
    #[cfg(feature = "dictation-shadow")]
    if let Some(replay) = replay {
        let report = replay.end().conserved(
            crate::dictation_executor::shadow::conservation_now().since(counters_before),
        );
        report.assert_clean(sub_run.title);
        let site = shadow_replay::site_totals();
        assert!(
            site.violations.is_empty(),
            "{}: a capture site broke a C8.7 order: {:?}",
            sub_run.title,
            site.violations
        );
    }
    lines
}

/// A parked delivery holds D inside a thread that owns an `Arc<World>`, and the world
/// owns that delivery's release sender, so a scenario that panics (or a bounded wait
/// that gives up) would leave the delivery, and a cancel blocked on D behind it,
/// parked for ever. This releases it while the scenario unwinds. It joins nothing:
/// a panic in a drop during unwinding would abort the process.
struct ReleaseOnUnwind(Arc<World>);

impl Drop for ReleaseOnUnwind {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.release_delivery();
        }
    }
}

/// A scenario's golden text: a header naming the scenario and the preserved
/// defects it witnesses, then each sub-run's trace.
pub(super) fn render(
    file: &str,
    title: &str,
    witnesses: &str,
    known_bad: Option<&str>,
    sub_runs: &[SubRun],
) -> String {
    let mut text = String::new();
    text.push_str(&format!("# golden: {file}\n# scenario: {title}\n"));
    text.push_str(&format!("# witnesses: {witnesses}\n"));
    if let Some(known_bad) = known_bad {
        text.push_str(&format!("# known-bad: {known_bad}\n"));
    }
    text.push_str(
        "# lines: `~` is shared state seen to change since the previous line; `#` is a\n\
         # scenario step; everything else is a port call, an emit or a reply, in order.\n",
    );
    for sub_run in sub_runs {
        text.push_str(&format!("\n=== {} ===\n", sub_run.title));
        for line in run_sub_run(sub_run) {
            text.push_str(&line);
            text.push('\n');
        }
    }
    text
}

fn golden_path(file: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/commands/recording_golden")
        .join(format!("{file}.golden"))
}

/// `MURMUR_GOLDEN_REPEAT=N` runs every scenario N times in one process and
/// requires each run to equal the golden (G1, G4). `MURMUR_GOLDEN_UPDATE=1`
/// rewrites a golden from the first run instead of comparing.
pub(super) fn repeat_count() -> usize {
    std::env::var("MURMUR_GOLDEN_REPEAT")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|count| *count > 0)
        .unwrap_or(1)
}

pub(super) fn check_golden(
    file: &str,
    title: &str,
    witnesses: &str,
    known_bad: Option<&str>,
    sub_runs: &[SubRun],
    verify: &dyn Fn(&str),
) -> String {
    let first = render(file, title, witnesses, known_bad, sub_runs);
    let path = golden_path(file);
    if std::env::var("MURMUR_GOLDEN_UPDATE").is_ok_and(|value| value == "1") {
        std::fs::create_dir_all(path.parent().expect("golden dir")).expect("create golden dir");
        std::fs::write(&path, &first).expect("write golden");
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing golden {}: {error}; record it with MURMUR_GOLDEN_UPDATE=1",
            path.display()
        )
    });
    assert_eq!(first, expected, "golden {file} differs from the recording");
    verify(&first);
    for run in 1..repeat_count() {
        let again = render(file, title, witnesses, known_bad, sub_runs);
        assert_eq!(
            again,
            expected,
            "golden {file}: run {} of {} differs",
            run + 1,
            repeat_count()
        );
        verify(&again);
    }
    eprintln!(
        "golden {file}: {n}/{n} runs identical to the checked-in trace ({} sub-runs each)",
        sub_runs.len(),
        n = repeat_count()
    );
    first
}
