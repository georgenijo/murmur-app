//! Shell-level tests of the legacy `process_audio` command (sites 301 and 302).
//!
//! Each scenario drives the real command body, `process_audio_inner`, over a
//! fake `ImportedAudioHost` (the command's own seam; the `#[tauri::command]`
//! wrapper only builds the production host). The audio is a real base64 WAV and
//! `parse_wav_to_samples`, the idle guard, the post-pipeline tail and the state
//! writes are production's; the fake supplies the pipeline (it takes the real
//! pipeline `IdleGuard`) and records every emit and keyboard write.
//!
//! The behaviour tests (no `history_` in the name) assert only what the command
//! did before sites 301/302 moved behind `decide_*`; they hold on the base. The
//! `history_` tests read `core_history` and need the registration of R301.5.
#![cfg(test)]
use super::*;
use std::sync::Mutex as StdMutex;
use std::task::{Context, Poll, Waker};

/// Serializes runs: the keyboard processing flag is process-global.
static LOCK: StdMutex<()> = StdMutex::new(());

fn locked<T>(mutex: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

type Hook = Box<dyn FnOnce(&AppState) + Send>;
type Listener = Box<dyn FnOnce(&FakeHost, &State) + Send>;

/// What the fake pipeline does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pipeline {
    /// Return success after creating the real pipeline guard.
    Succeed,
    /// A newer recording owns the app when the pipeline's guard finishes.
    NewerRecording,
    /// The user cancels while the pipeline runs: it returns `Cancelled`.
    Cancelled,
    /// The pipeline panics after taking its guard.
    Panic,
    /// The pipeline's guard finishes and it returns `Err`.
    Fail,
}

struct FakeHost {
    log: Arc<StdMutex<Vec<String>>>,
    pipeline: StdMutex<Pipeline>,
    /// Runs inside the smart-destination capture await (a cancel or a newer
    /// start landing while the destination is captured).
    during_capture: StdMutex<Option<Hook>>,
    /// Runs in the Smart Destination emit: after the stale check, before the
    /// parse guard exists (a newer start landing in that window).
    after_accept: Arc<StdMutex<Option<Hook>>>,
    /// Runs inline in the Smart Destination emit, as a listener would (`IN_CORE`
    /// set, the helper's D section held in production).
    in_destination_emit: StdMutex<Option<Hook>>,
    /// The dictation status when the pipeline was entered.
    status_at_pipeline: StdMutex<Option<DictationStatus>>,
    /// While set, the destination capture is pending (an import suspended there).
    hold_capture: std::sync::atomic::AtomicBool,
    /// While set, the Smart Destination emit reports a failure.
    destination_emit_fails: std::sync::atomic::AtomicBool,
    /// While set, the transition is pending *before* it asks for T (an import
    /// parked ahead of the lock, with no queued waiter on T).
    hold_transition: std::sync::atomic::AtomicBool,
    panic_in_capture: StdMutex<bool>,
    /// Runs inline in the `dictation-generation-started` emit (a listener).
    on_generation_started: StdMutex<Option<Listener>>,
    history: crate::core_history::CoreHistoryLock,
    state: std::sync::Weak<State>,
}

impl FakeHost {
    fn new(state: &Arc<State>) -> Self {
        Self::with_history(Arc::downgrade(state), state.app_state.core_history.clone())
    }

    /// For a state the test does not own as an `Arc` (the golden harness's app).
    fn with_history(
        state: std::sync::Weak<State>,
        history: crate::core_history::CoreHistoryLock,
    ) -> Self {
        Self {
            state,
            log: Arc::new(StdMutex::new(Vec::new())),
            pipeline: StdMutex::new(Pipeline::Succeed),
            during_capture: StdMutex::new(None),
            after_accept: Arc::new(StdMutex::new(None)),
            in_destination_emit: StdMutex::new(None),
            status_at_pipeline: StdMutex::new(None),
            hold_capture: std::sync::atomic::AtomicBool::new(false),
            panic_in_capture: StdMutex::new(false),
            on_generation_started: StdMutex::new(None),
            hold_transition: std::sync::atomic::AtomicBool::new(false),
            destination_emit_fails: std::sync::atomic::AtomicBool::new(false),
            history,
        }
    }

    fn push(&self, line: impl Into<String>) {
        locked(&self.log).push(line.into());
    }

    fn counts(&self) -> (usize, usize) {
        (
            self.history.pipeline_count(),
            self.history.outstanding_ops(),
        )
    }

    fn lines(&self) -> Vec<String> {
        locked(&self.log).clone()
    }
}

impl ImportedAudioHost for FakeHost {
    fn emit_import_json(
        &self,
        event: &str,
        payload: serde_json::Value,
    ) -> Result<(), super::dictation_host::HostEmitError> {
        // History counts at each emit: the idle emit of a parse failure runs in
        // the guard's `In` effect, so this line shows what is registered then.
        let (h2, h1) = self.counts();
        self.push(format!("emit {event} {payload} h2={h2} h1={h1}"));
        if event == "dictation-generation-started" {
            let listener = locked(&self.on_generation_started).take();
            if let (Some(listener), Some(state)) = (listener, self.state.upgrade()) {
                listener(self, &state);
            }
        }
        Ok(())
    }

    fn idle_emitter(&self) -> super::process_audio_ports::OwnedIdleEmit {
        let (log, history) = (Arc::clone(&self.log), self.history.clone());
        super::process_audio_ports::OwnedIdleEmit::new(move || {
            let (h2, h1) = (history.pipeline_count(), history.outstanding_ops());
            let (event, payload) = ("recording-status-changed", serde_json::json!("idle"));
            locked(&log).push(format!("emit {event} {payload} h2={h2} h1={h1}"));
            Ok(())
        })
    }

    fn transition<'a>(
        &'a self,
        state: &'a State,
    ) -> super::dictation_host::HostFuture<'a, Result<tokio::sync::MutexGuard<'a, ()>, String>>
    {
        Box::pin(async move {
            std::future::poll_fn(|cx| {
                if self.hold_transition.load(Ordering::SeqCst) {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            })
            .await;
            Ok(state.app_state.recording_transition.lock().await)
        })
    }

    fn capture_delivery_target_snapshot(&self) -> crate::frontmost::DeliveryTargetSnapshot {
        self.push("ax");
        crate::frontmost::DeliveryTargetSnapshot::Incomplete
    }

    fn set_processing(&self, processing: bool) {
        self.push(format!("keyboard {processing}"));
        keyboard::set_processing(processing);
    }

    fn capture_smart_destination<'a>(
        &'a self,
        _expected: &'a crate::frontmost::DeliveryTargetSnapshot,
        enabled: bool,
    ) -> super::dictation_host::HostFuture<'a, crate::smart_destination::Snapshot> {
        let (h2, h1) = self.counts();
        self.push(format!("capture_destination h2={h2} h1={h1}"));
        Box::pin(async move {
            std::future::poll_fn(|cx| {
                if self.hold_capture.load(Ordering::SeqCst) {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            })
            .await;
            let hook = locked(&self.during_capture).take();
            if let (Some(hook), Some(state)) = (hook, self.state.upgrade()) {
                hook(&state.app_state);
            }
            if *locked(&self.panic_in_capture) {
                panic!("the destination capture panics");
            }
            if enabled {
                crate::smart_destination::Snapshot::Unavailable
            } else {
                crate::smart_destination::Snapshot::Disabled
            }
        })
    }

    fn emit_smart_destination(
        &self,
        recording_id: u64,
        _context: &DictationContextSnapshot,
        _accepted_target: &crate::frontmost::DeliveryTargetSnapshot,
    ) -> Result<(), String> {
        self.push(format!(
            "emit_smart_destination {recording_id} in_core={}",
            crate::dictation_executor::executor::in_core()
        ));
        let listener = locked(&self.in_destination_emit).take();
        if let (Some(listener), Some(state)) = (listener, self.state.upgrade()) {
            listener(&state.app_state);
        }
        if self.destination_emit_fails.load(Ordering::SeqCst) {
            Err("the emit failed".to_string())
        } else {
            Ok(())
        }
    }

    fn run_transcription_pipeline<'a>(
        &'a self,
        samples: &'a [f32],
        app_state: &'a AppState,
        recording_id: u64,
        _performance_guard: &'a mut PerformanceRunGuard,
        _context: Arc<DictationContextSnapshot>,
        stop: PipelineStopContext,
    ) -> super::dictation_host::HostFuture<'a, Result<PipelineResult, String>> {
        let (h2, h1) = self.counts();
        self.push(format!(
            "pipeline {recording_id} samples={} h2={h2} h1={h1}",
            samples.len()
        ));
        *locked(&self.status_at_pipeline) = Some(app_state.dictation.lock_or_recover().status);
        Box::pin(async move {
            // The real pipeline builds its guard first and drops it on return.
            let mut guard = IdleGuard::for_pipeline(app_state, recording_id, stop.pipeline_op);
            let script = *locked(&self.pipeline);
            let result = |text: &str, terminal| PipelineResult {
                text: text.to_string(),
                timings: PipelineTimings::default(),
                terminal,
                history: None,
            };
            match script {
                Pipeline::Succeed => {
                    guard.finish(|_| {});
                    Ok(result("scripted transcript", PipelineTerminal::Success))
                }
                Pipeline::NewerRecording => {
                    // A cancel then a newer start: another recording owns the
                    // status when this pipeline's guard finishes.
                    FieldAdapter::cancel_through(app_state, recording_id);
                    let newer = FieldAdapter::alloc_id(
                        app_state,
                        &mut app_state.dictation.lock_or_recover(),
                    );
                    FieldAdapter::set_status(
                        newer,
                        &mut app_state.dictation.lock_or_recover(),
                        DictationStatus::Recording,
                    );
                    guard.finish(|_| {});
                    Ok(result("", PipelineTerminal::Success))
                }
                Pipeline::Cancelled => {
                    FieldAdapter::cancel_through(app_state, recording_id);
                    drop(guard);
                    Ok(result(
                        "",
                        PipelineTerminal::Cancelled(PerformanceStageV1::Vad),
                    ))
                }
                Pipeline::Panic => {
                    let _guard = guard;
                    panic!("the pipeline panics");
                }
                Pipeline::Fail => {
                    guard.finish(|_| {});
                    Err("the scripted pipeline failed".to_string())
                }
            }
        })
    }
}

use crate::dictation_executor::FieldAdapter;

fn wav_base64(samples: usize) -> String {
    let mut bytes = std::io::Cursor::new(Vec::new());
    {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::new(&mut bytes, spec).expect("wav writer");
        for index in 0..samples {
            writer
                .write_sample(((index % 100) as i16) * 50)
                .expect("sample");
        }
        writer.finalize().expect("finalize");
    }
    base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        bytes.into_inner(),
    )
}

fn poll_to_end<T>(future: impl std::future::Future<Output = T>) -> T {
    let mut future = Box::pin(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("process_audio parked on a barrier nobody opens"),
    }
}

fn poll_once<F: std::future::Future + ?Sized>(
    future: &mut std::pin::Pin<Box<F>>,
) -> Poll<F::Output> {
    future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
}

struct Run {
    state: Arc<State>,
    host: FakeHost,
}

fn run(body: impl FnOnce(&Run)) {
    let _serial = locked(&LOCK);
    keyboard::set_processing(false);
    // E7 / E8 (appendix C8.5): the import's decisions (sites 301, 302, 204a, 204b) run
    // through the shadow in sync mode; every flight is closed when the run ends.
    #[cfg(feature = "dictation-shadow")]
    let armed = !super::golden_harness::REPLAY_DISARMED.load(Ordering::SeqCst);
    #[cfg(feature = "dictation-shadow")]
    let replay = armed.then(crate::dictation_shadow::replay::Replay::begin);
    #[cfg(feature = "dictation-shadow")]
    let counters_before = crate::dictation_executor::shadow::conservation_now();
    let state = Arc::new(super::tests::test_state(AppState::default()));
    let host = FakeHost::new(&state);
    let run = Run { state, host };
    body(&run);
    keyboard::set_processing(false);
    #[cfg(feature = "dictation-shadow")]
    if let Some(replay) = replay {
        drop(run);
        replay
            .end()
            .conserved(crate::dictation_executor::shadow::conservation_now().since(counters_before))
            .assert_clean(
                std::thread::current()
                    .name()
                    .unwrap_or("process_audio test"),
            );
    }
}

impl Run {
    fn process(&self, audio: &str) -> Result<serde_json::Value, String> {
        // `after_accept` lands where the destination section has released D and the
        // parse guard does not exist yet (the shell owns the emit's D section, so
        // the hook cannot run inside the emit).
        let (hook, state) = (Arc::clone(&self.host.after_accept), Arc::clone(&self.state));
        // Installed only when a hook is set, so a test's own barrier is not replaced.
        let _barrier = locked(&hook).is_some().then(|| {
            let hook = Arc::clone(&hook);
            crate::barrier_hook::install(move |point| {
                if point == "process_audio.accepted" {
                    if let Some(hook) = locked(&hook).take() {
                        hook(&state.app_state);
                    }
                }
            })
        });
        poll_to_end(process_audio_inner(&self.host, &self.state, audio))
    }

    fn status(&self) -> DictationStatus {
        self.state.app_state.dictation.lock_or_recover().status
    }

    fn id(&self) -> u64 {
        self.state.app_state.recording_id.load(Ordering::SeqCst)
    }

    fn set_status(&self, status: DictationStatus, id: u64) {
        self.state
            .app_state
            .recording_id
            .store(id, Ordering::SeqCst);
        self.state.app_state.dictation.lock_or_recover().status = status;
    }

    fn history(&self) -> (usize, usize) {
        self.host.counts()
    }

    fn emits(&self, event: &str) -> Vec<String> {
        self.host
            .lines()
            .into_iter()
            .filter(|line| line.starts_with(&format!("emit {event} ")))
            .collect()
    }
}

fn idle_emits(run: &Run) -> usize {
    run.emits("recording-status-changed")
        .iter()
        .filter(|line| line.contains("\"idle\""))
        .count()
}

// ------------------------------------------------------------- behaviour

#[test]
fn success_replies_with_the_transcript_and_returns_to_idle() {
    run(|run| {
        let reply = run.process(&wav_base64(8_000)).expect("success");
        assert_eq!(
            reply,
            serde_json::json!({"type": "transcription", "text": "scripted transcript"})
        );
        assert_eq!(run.status(), DictationStatus::Idle);
        assert_eq!(run.id(), 1);
        assert!(!keyboard::is_processing());
        let lines = run.host.lines();
        let order: Vec<&str> = lines
            .iter()
            .map(|line| line.split(" h2=").next().unwrap_or(line))
            .collect();
        assert_eq!(
            order,
            [
                "ax",
                "keyboard true",
                "emit dictation-generation-started {\"recordingId\":1}",
                "emit recording-status-changed \"processing\"",
                "capture_destination",
                "pipeline 1 samples=8000",
                // the post-pipeline tail (204b): gate cleared, then `idle` once
                "keyboard false",
                "emit recording-status-changed \"idle\"",
            ]
        );
    });
}

#[test]
fn an_invalid_base64_payload_resets_to_idle_and_never_runs_the_pipeline() {
    run(|run| {
        let error = run.process("not base64 !!").expect_err("parse failure");
        assert!(error.starts_with("Failed to decode base64"), "{error}");
        assert_eq!(run.status(), DictationStatus::Idle);
        assert!(!keyboard::is_processing());
        assert_eq!(idle_emits(run), 1, "{:?}", run.host.lines());
        assert!(!run.host.lines().iter().any(|l| l.starts_with("pipeline")));
    });
}

#[test]
fn a_payload_that_is_not_a_wav_resets_to_idle_and_never_runs_the_pipeline() {
    run(|run| {
        let not_wav = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"junk");
        let error = run.process(&not_wav).expect_err("parse failure");
        assert!(error.starts_with("Failed to parse WAV"), "{error}");
        assert_eq!(run.status(), DictationStatus::Idle);
        assert_eq!(idle_emits(run), 1);
        assert!(!run.host.lines().iter().any(|l| l.starts_with("pipeline")));
    });
}

thread_local! {
    /// The span an import's parse failure runs inside (see
    /// `a_parse_failure_inside_a_span_emits_idle_at_the_drain`).
    static PARSE_SPAN: std::cell::RefCell<Option<crate::dictation_executor::executor::CoreEntry>> =
        const { std::cell::RefCell::new(None) };
}

/// D-S2-7: the import's parse failure finishes with an *owned* idle emit, so a finish
/// reached inside a core span (the import was polled by a listener's thread) queues
/// it as data and makes it at the drain, after D is released. A borrowed emitter
/// would be dropped with only an error log. The span is entered at the barrier that
/// lands after the destination section and before the parse guard exists, and the
/// drain guard stands before it, as an entry's does.
#[test]
fn a_parse_failure_inside_a_span_emits_idle_at_the_drain() {
    let not_wav = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"junk");
    for (payload, expected) in [
        ("not base64 !!", "Failed to decode base64"),
        (not_wav.as_str(), "Failed to parse WAV"),
    ] {
        run(|run| {
            let drain = DrainDeferredFinishes::new(&run.state.app_state);
            *locked(&run.host.after_accept) = Some(Box::new(|_| {
                PARSE_SPAN.with(|slot| {
                    *slot.borrow_mut() = Some(
                        crate::dictation_executor::executor::CoreEntry::enter()
                            .expect("outside any span"),
                    );
                });
            }));
            let error = run.process(payload).expect_err("parse failure");
            assert!(error.starts_with(expected), "{error}");
            assert_eq!(idle_emits(run), 0, "nothing is emitted inside the span");
            assert_eq!(
                run.status(),
                DictationStatus::Processing,
                "the finish is queued, not applied"
            );
            PARSE_SPAN.with(|slot| drop(slot.borrow_mut().take()));
            drop(drain);
            assert_eq!(
                idle_emits(run),
                1,
                "idle is emitted at the drain: {:?}",
                run.host.lines()
            );
            assert_eq!(run.status(), DictationStatus::Idle);
            assert_eq!(run.host.counts(), (0, 0), "H1 and H2 are retired");
        });
    }
}

#[test]
fn a_newer_recording_during_the_destination_capture_is_left_alone() {
    run(|run| {
        // A newer recording takes over while the destination is captured.
        *locked(&run.host.during_capture) = Some(Box::new(|app_state| {
            let newer =
                FieldAdapter::alloc_id(app_state, &mut app_state.dictation.lock_or_recover());
            FieldAdapter::set_status(
                newer,
                &mut app_state.dictation.lock_or_recover(),
                DictationStatus::Recording,
            );
        }));
        // The capture itself reports the stale recording first.
        let reply = run.process("not base64 !!").expect("stale reply");
        assert_eq!(
            reply,
            serde_json::json!({"type": "transcription", "text": "", "state": "idle"})
        );
        assert_eq!(run.status(), DictationStatus::Recording);
        assert_eq!(run.id(), 2);
    });
}

/// R302.1 with a newer id (R204a.5c): the parse guard clears nothing it does not own.
#[test]
fn a_parse_failure_with_a_newer_recording_leaves_that_recording_alone() {
    run(|run| {
        {
            let mut dictation = run.state.app_state.dictation.lock_or_recover();
            dictation.smart_destination_enabled = true;
            dictation.auto_paste = true;
        }
        *locked(&run.host.after_accept) = Some(Box::new(|app_state| {
            let newer =
                FieldAdapter::alloc_id(app_state, &mut app_state.dictation.lock_or_recover());
            FieldAdapter::set_status(
                newer,
                &mut app_state.dictation.lock_or_recover(),
                DictationStatus::Recording,
            );
        }));
        let error = run.process("not base64 !!").expect_err("parse failure");
        assert!(error.starts_with("Failed to decode base64"), "{error}");
        assert_eq!(run.status(), DictationStatus::Recording, "the newer owner");
        assert_eq!(run.id(), 2);
        assert_eq!(idle_emits(run), 0, "a stale guard emits no idle");
    });
}

#[test]
fn a_recording_cancelled_during_the_destination_capture_replies_empty() {
    run(|run| {
        *locked(&run.host.during_capture) = Some(Box::new(|app_state| {
            FieldAdapter::cancel_through(app_state, 1);
        }));
        let reply = run.process(&wav_base64(8_000)).expect("stale reply");
        assert_eq!(
            reply,
            serde_json::json!({"type": "transcription", "text": "", "state": "idle"})
        );
        assert!(!run.host.lines().iter().any(|l| l.starts_with("pipeline")));
    });
}

#[test]
fn a_newer_recording_at_pipeline_exit_is_not_overwritten() {
    run(|run| {
        *locked(&run.host.pipeline) = Pipeline::NewerRecording;
        let reply = run.process(&wav_base64(8_000)).expect("reply");
        assert_eq!(reply["type"], "transcription");
        assert_eq!(run.status(), DictationStatus::Recording, "the newer owner");
        assert_eq!(run.id(), 2);
        assert_eq!(idle_emits(run), 0, "no idle for a stale tail (R204b.4)");
    });
}

#[test]
fn a_cancelled_pipeline_returns_to_idle() {
    run(|run| {
        *locked(&run.host.pipeline) = Pipeline::Cancelled;
        let reply = run.process(&wav_base64(8_000)).expect("reply");
        assert_eq!(reply["type"], "transcription");
        assert_eq!(run.status(), DictationStatus::Idle);
        assert!(!keyboard::is_processing());
    });
}

#[test]
fn a_panicking_pipeline_still_returns_to_idle() {
    run(|run| {
        *locked(&run.host.pipeline) = Pipeline::Panic;
        let audio = wav_base64(8_000);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = run.process(&audio);
        }));
        assert!(panicked.is_err());
        assert_eq!(run.status(), DictationStatus::Idle);
        assert!(!keyboard::is_processing());
    });
}

#[test]
fn live_dictation_refuses_before_any_write() {
    run(|run| {
        run.set_status(DictationStatus::Recording, 5);
        let error = run.process(&wav_base64(8_000)).expect_err("refused");
        assert_eq!(
            error,
            "Cannot process audio while live dictation is active."
        );
        assert_eq!(run.id(), 5, "no id burned");
        assert_eq!(run.status(), DictationStatus::Recording);
        assert!(run.host.lines().is_empty());
    });
}

#[test]
fn a_meeting_refuses_before_the_dictation_lock() {
    run(|run| {
        run.state
            .app_state
            .meeting_active
            .store(true, Ordering::SeqCst);
        // A held D proves the meeting check does not take it.
        let _d = run.state.app_state.dictation.lock_or_recover();
        let error = run.process(&wav_base64(8_000)).expect_err("refused");
        assert_eq!(
            error,
            "Cannot process audio while a meeting transcript is active."
        );
    });
}

#[test]
fn a_file_transcription_refuses() {
    run(|run| {
        run.state
            .app_state
            .file_transcribing
            .store(true, Ordering::SeqCst);
        let error = run.process(&wav_base64(8_000)).expect_err("refused");
        assert_eq!(
            error,
            "Cannot process audio while a file transcription is in progress."
        );
        assert_eq!(run.id(), 0);
        assert!(run.host.lines().is_empty());
    });
}

#[test]
fn a_running_benchmark_refuses() {
    run(|run| {
        assert!(
            run.state.benchmark.try_start(),
            "the benchmark claims the coordinator"
        );
        let error = run.process(&wav_base64(8_000)).expect_err("refused");
        assert_eq!(
            error,
            "Cannot process audio while a benchmark is in progress."
        );
        assert_eq!(run.id(), 0);
    });
}

// --------------------------------------------------------------- history

fn line_with<'a>(lines: &'a [String], prefix: &str) -> &'a String {
    lines
        .iter()
        .find(|line| line.starts_with(prefix))
        .unwrap_or_else(|| panic!("no line starts with {prefix:?}: {lines:?}"))
}

/// `(effect point, H2 count, H1 count)` at each `ID+` / `ST` effect of a run, read
/// at the effect itself (before its write), so a registration or a retirement that
/// moves across an `In` effect shows at that boundary.
type Boundaries = std::rc::Rc<std::cell::RefCell<Vec<(&'static str, usize, usize)>>>;

fn observe_effect_boundaries(run: &Run) -> (Boundaries, crate::barrier_hook::Installed) {
    let seen = Boundaries::default();
    let log = std::rc::Rc::clone(&seen);
    let history = run.state.app_state.core_history.clone();
    let installed = crate::barrier_hook::install(move |point| {
        if point.starts_with("effect.") {
            log.borrow_mut()
                .push((point, history.pipeline_count(), history.outstanding_ops()));
        }
    });
    (seen, installed)
}

/// R301.5: `H2 reg rid` and `H1 reg op_pl` are registered right after the
/// decision, before any effect, and are still there while the pipeline runs.
#[test]
fn history_is_registered_before_the_effects_and_while_the_pipeline_runs() {
    run(|run| {
        assert_eq!(run.history(), (0, 0));
        let (seen, _installed) = observe_effect_boundaries(run);
        run.process(&wav_base64(8_000)).expect("success");
        let lines = run.host.lines();
        // Before any effect: the first `In` effect (`ID+`) already sees both
        // entries, and so does `ST→Processing`. The pipeline's guard writes `Idle`
        // after retiring them (C3.9).
        assert_eq!(
            *seen.borrow(),
            [
                ("effect.alloc_id", 1, 1),
                ("effect.set_status", 1, 1),
                ("effect.set_status", 0, 0)
            ],
            "{lines:?}"
        );
        assert!(
            line_with(&lines, "emit recording-status-changed \"processing\"")
                .ends_with("h2=1 h1=1"),
            "{lines:?}"
        );
        assert!(line_with(&lines, "capture_destination").ends_with("h2=1 h1=1"));
        assert!(line_with(&lines, "pipeline 1").ends_with("h2=1 h1=1"));
        // Success: the pipeline guard (204a `PipelineExit`) retired both.
        assert_eq!(run.history(), (0, 0));
    });
}

/// R302.1: a parse failure retires `H2` and `H1` after the decision and **before**
/// its `In` effects (the idle emit sees neither entry).
#[test]
fn history_is_retired_before_the_in_effects_of_a_parse_failure() {
    for audio in ["not base64 !!".to_string(), {
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"junk")
    }] {
        run(|run| {
            let (seen, _installed) = observe_effect_boundaries(run);
            run.process(&audio).expect_err("parse failure");
            // The guard's first `In` effect (`ST→Idle`) already sees neither entry
            // (the import's own `ID+` / `ST→Processing` saw both).
            assert_eq!(
                *seen.borrow(),
                [
                    ("effect.alloc_id", 1, 1),
                    ("effect.set_status", 1, 1),
                    ("effect.set_status", 0, 0)
                ]
            );
            let lines = run.host.lines();
            let idle = lines
                .iter()
                .find(|line| line.starts_with("emit recording-status-changed \"idle\""))
                .expect("the idle emit");
            assert!(idle.ends_with("h2=0 h1=0"), "retired before In: {idle}");
            assert_eq!(run.history(), (0, 0));
            assert!(!lines.iter().any(|l| l.starts_with("pipeline")));
        });
    }
}

/// C3.9: a stale return, a stale parse failure, a stale pipeline exit, a cancel and
/// a panic each leave neither entry behind.
#[test]
fn history_is_retired_on_every_exit() {
    // stale at the destination capture (cancelled)
    run(|run| {
        *locked(&run.host.during_capture) = Some(Box::new(|app_state| {
            FieldAdapter::cancel_through(app_state, 1);
        }));
        run.process(&wav_base64(8_000)).expect("stale reply");
        assert_eq!(run.history(), (0, 0));
    });
    // stale at the destination capture (a newer recording)
    run(|run| {
        *locked(&run.host.during_capture) = Some(Box::new(|app_state| {
            let newer =
                FieldAdapter::alloc_id(app_state, &mut app_state.dictation.lock_or_recover());
            FieldAdapter::set_status(
                newer,
                &mut app_state.dictation.lock_or_recover(),
                DictationStatus::Recording,
            );
        }));
        run.process(&wav_base64(8_000)).expect("stale reply");
        assert_eq!(run.history(), (0, 0));
    });
    // a newer recording at the parse failure (R204a.5c): nothing is written, both retire
    run(|run| {
        {
            let mut dictation = run.state.app_state.dictation.lock_or_recover();
            dictation.smart_destination_enabled = true;
            dictation.auto_paste = true;
        }
        *locked(&run.host.after_accept) = Some(Box::new(|app_state| {
            let newer =
                FieldAdapter::alloc_id(app_state, &mut app_state.dictation.lock_or_recover());
            FieldAdapter::set_status(
                newer,
                &mut app_state.dictation.lock_or_recover(),
                DictationStatus::Recording,
            );
        }));
        run.process("not base64 !!").expect_err("parse failure");
        assert_eq!(run.history(), (0, 0));
    });
    // a newer recording at the pipeline exit
    run(|run| {
        *locked(&run.host.pipeline) = Pipeline::NewerRecording;
        run.process(&wav_base64(8_000)).expect("reply");
        assert_eq!(run.history(), (0, 0));
    });
    // cancelled mid-pipeline
    run(|run| {
        *locked(&run.host.pipeline) = Pipeline::Cancelled;
        run.process(&wav_base64(8_000)).expect("reply");
        assert_eq!(run.history(), (0, 0));
    });
    // a panic in the pipeline
    run(|run| {
        *locked(&run.host.pipeline) = Pipeline::Panic;
        let audio = wav_base64(8_000);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = run.process(&audio);
        }));
        assert_eq!(run.history(), (0, 0));
    });
    // a panic before the pipeline guard exists (the owner's `Drop` retires)
    run(|run| {
        *locked(&run.host.panic_in_capture) = true;
        let audio = wav_base64(8_000);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = run.process(&audio);
        }));
        assert!(panicked.is_err());
        assert_eq!(run.history(), (0, 0));
    });
}

/// A refused import registers nothing and burns no id.
#[test]
fn history_is_untouched_by_a_refusal() {
    run(|run| {
        run.set_status(DictationStatus::Processing, 3);
        run.process(&wav_base64(8_000)).expect_err("refused");
        assert_eq!(run.history(), (0, 0));
        assert_eq!(run.id(), 3);
    });
}

/// D-S2-2: the entry's first statement refuses a nested call before the transition
/// await, T, D or `core_history`. A listener of the `PT` emits (D and T released,
/// `IN_CORE` set around the port call) that calls the command again is refused and
/// changes nothing; the outer import completes.
#[test]
fn a_nested_process_audio_is_refused_before_any_lock() {
    run(|run| {
        let nested = Arc::new(StdMutex::new(Vec::<String>::new()));
        let seen = Arc::clone(&nested);
        let audio = wav_base64(8_000);
        *locked(&run.host.on_generation_started) = Some(Box::new(move |host, state| {
            let reply = poll_to_end(process_audio_inner(host, state, "irrelevant"));
            locked(&seen).push(format!("{reply:?}"));
        }));
        let outer = run.process(&audio).expect("the outer import completes");
        assert_eq!(outer["type"], "transcription");
        let nested = locked(&nested).clone();
        assert_eq!(nested.len(), 1);
        assert!(
            nested[0].contains("refused process_audio") && nested[0].contains("Reentrant"),
            "{nested:?}"
        );
        assert_eq!(run.id(), 1, "the refused entry burned no id");
        assert_eq!(run.history(), (0, 0));
    });
}

/// D-S2-2/3, T before D: an import that parked on T and is polled again by an
/// inline callback that holds D must park again (take neither T nor D) and carry
/// on when it is next polled outside the span. Run on a poller thread under a
/// watchdog: a poll that went on to take D would block on the held D until the
/// watchdog fires.
#[test]
fn a_resumed_import_polled_inside_a_core_span_parks_instead_of_taking_t_and_d() {
    use std::sync::mpsc;
    use std::time::Duration;
    run(|run| {
        let audio = wav_base64(8_000);
        let mut import = Box::pin(process_audio_inner(&run.host, &run.state, &audio));
        // Park ahead of T: the fake's transition is pending before it asks for the
        // lock, so no waiter is queued on T and its release leaves T free.
        run.host.hold_transition.store(true, Ordering::SeqCst);
        assert!(poll_once(&mut import).is_pending(), "parked ahead of T");
        run.host.hold_transition.store(false, Ordering::SeqCst);
        // The callback: this thread holds D. The poller is the listener's thread
        // inside a core span; it polls the import that T's release just made ready.
        let dictation = run.state.app_state.dictation.lock_or_recover();
        let (sent, parked) = mpsc::channel();
        let outcome = std::thread::scope(|scope| {
            let poller = scope.spawn(|| {
                let _span = crate::dictation_executor::executor::CoreEntry::enter()
                    .expect("a fresh thread is outside any span");
                sent.send(poll_once(&mut import).is_pending()).ok();
            });
            let outcome = parked.recv_timeout(Duration::from_secs(5));
            // Always release D, so a regressed poll can finish and be joined.
            drop(dictation);
            let _ = poller.join();
            outcome
        });
        assert_eq!(
            outcome,
            Ok(true),
            "the poll inside the span must park, not deadlock on D"
        );
        // T was not acquired inside the span: a poll that took T and then parked at
        // `until_outside_core` before D would hold it here, with the import parked.
        drop(
            run.state
                .app_state
                .recording_transition
                .try_lock()
                .expect("the parked poll did not take T"),
        );
        assert_eq!(run.history(), (0, 0), "nothing was registered or written");
        assert_eq!(run.id(), 0);
        // Polled outside the span, the same import carries on and completes.
        let reply = poll_to_end(import).expect("the import completes");
        assert_eq!(reply["type"], "transcription");
        assert_eq!(run.id(), 1);
        assert_eq!(run.history(), (0, 0));
    });
}

/// D-S2-4: an inline callback drops an import suspended at the destination
/// capture. Its owner's history retirement is queued behind a finish queued
/// earlier (FIFO) and runs once, on the drain, after the span; nothing retires
/// inside the span. The abandoned capture keeps today's status and events.
#[test]
fn dropping_a_suspended_import_in_a_core_span_queues_its_retirement_fifo() {
    use murmur_dictation_core::decision::{ContKind, ContinuationSpec};
    use murmur_dictation_core::ids::RecordingId;
    run(|run| {
        let app_state = &run.state.app_state;
        run.host.hold_capture.store(true, Ordering::SeqCst);
        let audio = wav_base64(8_000);
        let mut import = Box::pin(process_audio_inner(&run.host, &run.state, &audio));
        assert!(
            poll_once(&mut import).is_pending(),
            "suspended at the capture"
        );
        assert_eq!(run.history(), (1, 1));
        assert_eq!(run.status(), DictationStatus::Processing);
        let lines_before = run.host.lines();
        // An earlier finish, queued inside the span: another pipeline's guard.
        let earlier = PipelineOwner::register(
            &app_state.core_history,
            ContinuationSpec {
                recording_id: RecordingId(9),
                kind: ContKind::Pipeline,
            },
        );
        assert_eq!(run.history(), (2, 2));
        let counters = app_state.core_history.counters();
        // What an entry's guard does: declared before the span, it drains after.
        let drain = DrainDeferredFinishes::new(app_state);
        {
            let span = crate::dictation_executor::executor::CoreEntry::enter()
                .expect("the test thread is outside any span");
            drop(IdleGuard::for_pipeline(app_state, 9, Some(earlier)));
            drop(import);
            assert_eq!(run.history(), (2, 2), "nothing retires inside the span");
            let queued: Vec<(u64, bool)> = DEFERRED_FINISHES.with(|queue| {
                queue
                    .borrow()
                    .iter()
                    .map(|entry| (entry.recording_id, entry.retire_only))
                    .collect()
            });
            assert_eq!(queued, [(9, false), (1, true)], "FIFO: the finish first");
            drop(span);
        }
        drop(drain);
        assert_eq!(run.history(), (0, 0), "both pairs retired after the drain");
        assert_eq!(
            app_state.core_history.counters(),
            counters,
            "each H1 retired exactly once"
        );
        assert!(DEFERRED_FINISHES.with(|queue| queue.borrow().is_empty()));
        assert_eq!(
            run.host.lines(),
            lines_before,
            "the abandoned capture emits nothing"
        );
        assert_eq!(run.status(), DictationStatus::Processing, "status as today");
    });
}

type Import = super::golden_harness::Fut<Result<serde_json::Value, String>>;

/// The slot the inline listener empties: the suspended import.
static SUSPENDED_IMPORT: StdMutex<Option<Import>> = StdMutex::new(None);

/// The body of the cancel scenario below: an accepted import suspended at the
/// destination capture, then the real cancel command, whose R205.5 `idle` emit runs
/// a synchronous listener that drops the import (`IN_CORE` set, T held).
fn cancel_drops_a_suspended_import(world: &Arc<super::golden_harness::World>) {
    let history = world.state().app_state.core_history.clone();
    let host = Arc::new(FakeHost::with_history(
        std::sync::Weak::new(),
        history.clone(),
    ));
    host.hold_capture.store(true, Ordering::SeqCst);
    let audio = wav_base64(8_000);
    let import: Import = Box::pin({
        let world = Arc::clone(world);
        let host = Arc::clone(&host);
        async move {
            let state = world.state();
            process_audio_inner(&*host, state.inner(), &audio).await
        }
    });
    let mut import = import;
    assert!(
        poll_once(&mut import).is_pending(),
        "suspended at the capture"
    );
    *locked(&SUSPENDED_IMPORT) = Some(import);
    assert_eq!(
        (history.pipeline_count(), history.outstanding_ops()),
        (1, 1),
        "the accepted import is registered"
    );
    assert_eq!(
        world.state().app_state.dictation.lock_or_recover().status,
        DictationStatus::Processing
    );
    let lines_before = host.lines();
    let counters = history.counters();
    world.on_inline("emit:recording-status-changed", || {
        let import = locked(&SUSPENDED_IMPORT).take();
        assert!(import.is_some(), "the listener found the suspended import");
        drop(import);
        // Inside the span: nothing retired on the spot, the work is queued.
        assert!(
            DEFERRED_FINISHES.with(|queue| !queue.borrow().is_empty()),
            "the listener's drop queued the retirement"
        );
    });
    let mut cancel = world.cancel(None);
    super::golden_harness::complete(&mut cancel).expect("the cancel replies Ok");
    // After the cancel returned, with no later drain on this thread: both entries
    // are retired and nothing is left queued.
    assert_eq!(
        (history.pipeline_count(), history.outstanding_ops()),
        (0, 0),
        "H2 and H1 retired by the cancel's own drain"
    );
    assert!(DEFERRED_FINISHES.with(|queue| queue.borrow().is_empty()));
    assert_eq!(history.counters(), counters, "each op retired exactly once");
    assert_eq!(
        host.lines(),
        lines_before,
        "no import effect after the drop"
    );
}

/// Finding 1 of the re-check: cancel drains work a listener queued. The listener
/// drops the suspended import from the cancel's synchronous idle emit; the cancel
/// entry's drain guard retires both history entries before it returns.
#[test]
fn a_cancel_drains_the_retirement_queued_for_an_import_dropped_by_its_idle_emit() {
    use super::golden_harness::{run_sub_run, Setup, SubRun};
    let _serial = locked(&LOCK);
    keyboard::set_processing(false);
    run_sub_run(&SubRun {
        title: "cancel drops a suspended import",
        setup: Setup::idle(),
        body: cancel_drops_a_suspended_import,
    });
    keyboard::set_processing(false);
}

/// (a) from the lead: the synchronous Smart Destination emit runs with `IN_CORE` set.
#[test]
fn the_smart_destination_emit_keeps_in_core_set() {
    run(|run| {
        {
            let mut dictation = run.state.app_state.dictation.lock_or_recover();
            dictation.smart_destination_enabled = true;
            dictation.auto_paste = true;
        }
        run.process(&wav_base64(8_000)).expect("success");
        let lines = run.host.lines();
        assert_eq!(
            line_with(&lines, "emit_smart_destination"),
            "emit_smart_destination 1 in_core=true"
        );
        assert!(!crate::dictation_executor::executor::in_core());
    });
}

/// (b) from the lead: a finish an inline listener queued while the destination
/// helper held its own D (an `IdleGuard` dropped inside the emit) runs when the
/// call returns, before the pipeline starts, not later.
#[test]
fn a_finish_queued_inside_the_destination_emit_drains_when_it_returns() {
    run(|run| {
        {
            let mut dictation = run.state.app_state.dictation.lock_or_recover();
            dictation.smart_destination_enabled = true;
            dictation.auto_paste = true;
        }
        *locked(&run.host.in_destination_emit) = Some(Box::new(|app_state| {
            let rid = app_state.recording_id.load(Ordering::SeqCst);
            // Dropped without finishing, inside a core span: queued, not run.
            drop(IdleGuard::new(app_state, rid));
            // The shell holds D here (it owns the emit's section), so the listener
            // cannot read the status; it sees the finish queued, not run.
            assert_eq!(
                DEFERRED_FINISHES.with(|queue| queue.borrow().len()),
                1,
                "the finish is deferred while the emit runs"
            );
        }));
        let _ = run.process(&wav_base64(8_000));
        assert_eq!(
            *locked(&run.host.status_at_pipeline),
            Some(DictationStatus::Idle),
            "the queued finish ran before the pipeline: {:?}",
            run.host.lines()
        );
    });
}

// ------------------------------------------- D-S2-11: the destination section and the error tail

fn destination_witness() -> Vec<(&'static str, String)> {
    super::process_audio_ports::DESTINATION.with(|slot| {
        slot.borrow()
            .iter()
            .map(|w| (w.code, format!("{:?}", w.outcome)))
            .collect()
    })
}

fn enable_destination(run: &Run) {
    let mut dictation = run.state.app_state.dictation.lock_or_recover();
    dictation.smart_destination_enabled = true;
    dictation.auto_paste = true;
}

/// The import's emit is `In:SDE` under the shell's own D section, and its witness is
/// the emit's real result: a delivered emit is `Ran`, a rejected one `Failed`. With
/// Smart destination off there is no section and no witness.
#[test]
fn the_import_destination_witness_reports_the_real_emit_result() {
    for (fails, expected) in [(false, "Ran"), (true, "Failed")] {
        super::process_audio_ports::DESTINATION.with(|slot| slot.borrow_mut().clear());
        run(|run| {
            enable_destination(run);
            run.host
                .destination_emit_fails
                .store(fails, Ordering::SeqCst);
            run.process(&wav_base64(8_000))
                .expect("a failed emit does not fail the import");
        });
        assert_eq!(
            destination_witness(),
            vec![("SDE", expected.to_string())],
            "emit fails: {fails}"
        );
    }
    super::process_audio_ports::DESTINATION.with(|slot| slot.borrow_mut().clear());
    run(|run| {
        run.process(&wav_base64(8_000)).expect("success");
        assert!(!run
            .host
            .lines()
            .iter()
            .any(|line| line.starts_with("emit_smart_destination")));
    });
    assert_eq!(
        destination_witness(),
        vec![],
        "no section without Smart destination"
    );
}

/// The import's destination section reports its own row, `R301.6`, never the
/// start's `R103.0`: shadow capture would credit a start row to site 103.
#[test]
fn the_import_destination_decision_never_reports_a_start_row() {
    super::process_audio_ports::DESTINATION_ROW.with(|slot| *slot.borrow_mut() = None);
    run(|run| {
        enable_destination(run);
        run.process(&wav_base64(8_000)).expect("success");
    });
    let row = super::process_audio_ports::DESTINATION_ROW
        .with(|slot| *slot.borrow())
        .expect("the destination section decided");
    assert_eq!(row, "R301.6");
    assert!(!row.starts_with("R103"), "the import reported {row}");
}

/// The destination emit holds D (the shell owns the section): a probe from inside the
/// emit finds D taken, and the pipeline still starts after it released it.
#[test]
fn the_destination_emit_runs_under_the_shells_d_hold() {
    run(|run| {
        enable_destination(run);
        let held = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (probe, state) = (Arc::clone(&held), Arc::clone(&run.state));
        *locked(&run.host.in_destination_emit) = Some(Box::new(move |_| {
            probe.store(
                state.app_state.dictation.try_lock().is_err(),
                Ordering::SeqCst,
            );
        }));
        run.process(&wav_base64(8_000)).expect("success");
        assert!(held.load(Ordering::SeqCst), "D is held across the emit");
        assert!(run.state.app_state.dictation.try_lock().is_ok());
    });
}

/// A pipeline that returns `Err` still runs the 204b section (the idle gate does not
/// depend on the outcome) and then the shell returns the pipeline's own error text:
/// that reply is a separate shell tail, not part of R302.2.
#[test]
fn a_pipeline_error_runs_the_idle_gate_and_returns_its_text_as_a_shell_tail() {
    run(|run| {
        *locked(&run.host.pipeline) = Pipeline::Fail;
        let error = run
            .process(&wav_base64(8_000))
            .expect_err("the pipeline failed");
        assert_eq!(error, "the scripted pipeline failed");
        assert_eq!(idle_emits(run), 1, "the 204b section emitted idle once");
        assert_eq!(run.status(), DictationStatus::Idle);
        assert_eq!(run.history(), (0, 0));
        assert!(!keyboard::is_processing());
    });
}

/// D-S2c-7 b, c: through the `process_audio` entry, each refusal allocates under D
/// exactly what it allocates with the shadow compiled in and not armed, and the
/// warning it logs is emitted at the same lock boundary.
#[cfg(feature = "dictation-shadow")]
#[test]
fn the_armed_shadow_adds_nothing_under_d_at_a_process_audio_refusal() {
    use super::golden_harness::REPLAY_DISARMED;
    use super::pd_witness_tests::under_d::count_under_d;
    fn measured(setup: fn(&Run), disarmed: bool) -> u64 {
        REPLAY_DISARMED.store(disarmed, Ordering::SeqCst);
        struct Rearm;
        impl Drop for Rearm {
            fn drop(&mut self) {
                REPLAY_DISARMED.store(false, Ordering::SeqCst);
            }
        }
        let _rearm = Rearm;
        let allocations = std::cell::Cell::new(0);
        run(|run| {
            setup(run);
            let (result, count) = count_under_d(&run.state.app_state.dictation, || {
                run.process(&wav_base64(8_000))
            });

            assert!(result.is_err(), "the scenario refuses");
            allocations.set(count);
        });
        allocations.get()
    }
    type Scenario = (&'static str, fn(&Run));
    let scenarios: [Scenario; 4] = [
        ("a meeting is active (before D)", |run| {
            run.state
                .app_state
                .meeting_active
                .store(true, Ordering::SeqCst);
        }),
        ("a file transcription (under D)", |run| {
            run.state
                .app_state
                .file_transcribing
                .store(true, Ordering::SeqCst);
        }),
        ("a benchmark (under D)", |run| {
            assert!(run.state.benchmark.try_start());
        }),
        ("a live dictation (under D)", |run| {
            run.state.app_state.dictation.lock_or_recover().status = DictationStatus::Recording;
        }),
    ];
    let mut counted = 0;
    for (title, setup) in scenarios {
        let baseline = measured(setup, true);
        let armed = measured(setup, false);
        assert_eq!(
            armed, baseline,
            "process_audio refused by {title}: armed {armed} allocations under D, disarmed {baseline}"
        );
        counted += baseline;
    }
    assert!(
        counted > 0,
        "the probe counted nothing under D: it is not measuring"
    );
}
