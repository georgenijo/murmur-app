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
}

struct FakeHost {
    log: StdMutex<Vec<String>>,
    pipeline: StdMutex<Pipeline>,
    /// Runs inside the smart-destination capture await (a cancel or a newer
    /// start landing while the destination is captured).
    during_capture: StdMutex<Option<Hook>>,
    /// Runs in the Smart Destination emit: after the stale check, before the
    /// parse guard exists (a newer start landing in that window).
    after_accept: StdMutex<Option<Hook>>,
    panic_in_capture: StdMutex<bool>,
    history: crate::core_history::CoreHistoryLock,
    state: std::sync::Weak<State>,
}

impl FakeHost {
    fn new(state: &Arc<State>) -> Self {
        Self {
            state: Arc::downgrade(state),
            log: StdMutex::new(Vec::new()),
            pipeline: StdMutex::new(Pipeline::Succeed),
            during_capture: StdMutex::new(None),
            after_accept: StdMutex::new(None),
            panic_in_capture: StdMutex::new(false),
            history: state.app_state.core_history.clone(),
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
    fn emit_json(
        &self,
        event: &str,
        payload: serde_json::Value,
    ) -> Result<(), super::dictation_host::HostEmitError> {
        // History counts at each emit: the idle emit of a parse failure runs in
        // the guard's `In` effect, so this line shows what is registered then.
        let (h2, h1) = self.counts();
        self.push(format!("emit {event} {payload} h2={h2} h1={h1}"));
        Ok(())
    }

    fn transition<'a>(
        &'a self,
        state: &'a State,
    ) -> super::dictation_host::HostFuture<'a, Result<tokio::sync::MutexGuard<'a, ()>, String>>
    {
        Box::pin(async move { Ok(state.app_state.recording_transition.lock().await) })
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
        self.push("capture_destination");
        Box::pin(async move {
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
        _app_state: &AppState,
        recording_id: u64,
        _context: &DictationContextSnapshot,
        _accepted_target: &crate::frontmost::DeliveryTargetSnapshot,
    ) {
        self.push(format!("emit_smart_destination {recording_id}"));
        let hook = locked(&self.after_accept).take();
        if let (Some(hook), Some(state)) = (hook, self.state.upgrade()) {
            hook(&state.app_state);
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

struct Run {
    state: Arc<State>,
    host: FakeHost,
}

fn run(body: impl FnOnce(&Run)) {
    let _serial = locked(&LOCK);
    keyboard::set_processing(false);
    let state = Arc::new(super::tests::test_state(AppState::default()));
    let host = FakeHost::new(&state);
    let run = Run { state, host };
    body(&run);
    keyboard::set_processing(false);
}

impl Run {
    fn process(&self, audio: &str) -> Result<serde_json::Value, String> {
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
