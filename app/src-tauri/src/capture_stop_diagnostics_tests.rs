// Hardware-free lifecycle-to-SQL coverage of recording-stop attribution.
use super::*;
use crate::performance_metrics::{
    production, PerformanceMetrics, PerformanceRunV1, RunCorrelationV1, RunOutcomeV1,
};

struct MetricsSink {
    metrics: PerformanceMetrics,
    recorded: RecordingSink,
}

impl LifecycleSink for MetricsSink {
    fn invalidate_input_health(&self, device_id: Option<&str>) {
        self.recorded.invalidate_input_health(device_id);
    }

    fn notify(
        &self,
        app: Option<&tauri::AppHandle>,
        owner: AudioOwner,
        event: AudioLifecycleEvent,
    ) {
        if let (Some(id), AudioLifecycleEvent::InitializationFailed { kind, .. }) =
            (owner.dictation_id(), &event)
        {
            // Match recording.rs, including the indirect failure route.
            self.metrics.production_capture_failure(id, *kind);
        }
        if let (Some(id), AudioLifecycleEvent::StartupDiagnostic(diagnostic)) =
            (owner.dictation_id(), &event)
        {
            self.metrics.observe_production_capture(id, *diagnostic);
        }
        self.recorded.notify(app, owner, event);
    }
}

struct StopFactory {
    teardown: Gate,
    stopped: Sender<()>,
    buffer: Arc<Mutex<Option<Arc<Mutex<audio::CaptureSamples>>>>>,
}

impl WorkerFactory for StopFactory {
    fn spawn(
        &self,
        spec: AudioWorkerSpec,
        events: AudioWorkerEventSender,
    ) -> Result<JoinHandle<()>, String> {
        *self.buffer.lock().unwrap() = Some(spec.shared.clone());
        let teardown = self.teardown.clone();
        let stopped = self.stopped.clone();
        Ok(std::thread::spawn(move || {
            spec.shared
                .lock()
                .unwrap()
                .append_pcm(&[0.25; 160], WHISPER_SAMPLE_RATE);
            events
                .send(AudioWorkerEvent::FirstBuffer {
                    owner: spec.owner,
                    sample_rate: WHISPER_SAMPLE_RATE,
                    sent_at: Instant::now(),
                })
                .unwrap();
            spec.command_receiver.recv().unwrap();
            events
                .send(AudioWorkerEvent::RecordingSealed { owner: spec.owner })
                .unwrap();
            stopped.send(()).unwrap();
            teardown.wait();
            events
                .send(AudioWorkerEvent::StartupDiagnostic {
                    owner: spec.owner,
                    diagnostic: audio::AudioStartupDiagnostic::WorkerStopped { elapsed_ms: 212 },
                })
                .unwrap();
            events
                .send(AudioWorkerEvent::ThreadExited { owner: spec.owner })
                .unwrap();
        }))
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    sink: Arc<MetricsSink>,
    supervisor: AudioSupervisor,
    teardown: Gate,
    stopped: Receiver<()>,
    buffer: Arc<Mutex<Option<Arc<Mutex<audio::CaptureSamples>>>>>,
}

impl Fixture {
    fn new(standby: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let metrics = PerformanceMetrics::default();
        metrics.initialize(temp.path().to_path_buf(), None).unwrap();
        let sink = Arc::new(MetricsSink {
            metrics,
            recorded: RecordingSink::default(),
        });
        let teardown = Gate::closed();
        let (stopped, receiver) = mpsc::channel();
        let buffer = Arc::new(Mutex::new(None));
        let supervisor = spawn_supervisor(
            Arc::new(StopFactory {
                teardown: teardown.clone(),
                stopped,
                buffer: buffer.clone(),
            }),
            sink.clone(),
            SupervisorConfig::default(),
        );
        if standby {
            supervisor
                .sender
                .send(SupervisorMessage::ConfigureStandby(StandbyPolicy {
                    seconds: 300,
                    pre_roll_ms: 450,
                }))
                .unwrap();
        }
        Self {
            _temp: temp,
            sink,
            supervisor,
            teardown,
            stopped: receiver,
            buffer,
        }
    }

    fn start(&self, id: u64) {
        self.sink.metrics.register_production(
            id,
            production::MicrophoneSelectionV1::SystemDefault,
            None,
        );
        self.sink.metrics.begin_dictation(id, Vec::new()).unwrap();
        start(&self.supervisor, AudioOwner::Dictation(id))
            .recv()
            .unwrap()
            .unwrap();
        wait_until("recording ready", || {
            self.supervisor.public.phase() == PublicPhase::Recording
        });
    }

    fn stop_and_persist(&self, id: u64) -> PerformanceRunV1 {
        let (response, result) = mpsc::channel();
        self.supervisor
            .sender
            .send(SupervisorMessage::Stop {
                owner: Some(AudioOwner::Dictation(id)),
                trigger: DictationStopTrigger::Toggle,
                response,
            })
            .unwrap();
        assert!(!result
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap()
            .is_empty());
        self.sink
            .metrics
            .complete(
                &RunCorrelationV1::Dictation { recording_id: id },
                RunOutcomeV1::Success,
                Vec::new(),
                None,
                None,
            )
            .unwrap()
            .unwrap()
    }

    fn finish_teardown(&self) {
        self.teardown.open();
        wait_until("helper exited and joined", || {
            self.supervisor.public.phase() == PublicPhase::Idle
        });
    }

    fn assert_late_exit(&self, mut expected: PerformanceRunV1) {
        let capture = &mut expected.production.as_mut().unwrap().capture;
        capture.stop_to_worker_exit_ms = Some(212);
        capture.worker_invariant_violation = Some(false);
        wait_until("late exit persisted on exact run", || {
            self.sink.metrics.get(&expected.run_id).unwrap() == Some(expected.clone())
        });
    }
}

#[test]
fn standby_disable_expiry_and_recovery_exit_leave_completed_run_unchanged() {
    for retirement in ["disable", "expiry", "recovery"] {
        let fixture = Fixture::new(true);
        fixture.start(18);
        let completed = fixture.stop_and_persist(18);
        assert_eq!(fixture.supervisor.public.phase(), PublicPhase::Standby);
        match retirement {
            "disable" => fixture
                .supervisor
                .sender
                .send(SupervisorMessage::ConfigureStandby(StandbyPolicy::default()))
                .unwrap(),
            "expiry" => check_deadlines(&fixture.supervisor, Duration::from_secs(301)),
            _ => {
                assert_eq!(
                    cancel_any_phase(&fixture.supervisor, AudioOwner::Dictation(18))
                        .recv()
                        .unwrap(),
                    Ok(true)
                );
            }
        }
        fixture
            .stopped
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        // The same last dictation owner is retained during retirement. Even an
        // unconfirmed retirement must not supply that completed run's evidence.
        fixture
            .supervisor
            .sender
            .send(SupervisorMessage::Worker(
                AudioWorkerEvent::TerminationUnconfirmed {
                    owner: AudioOwner::Dictation(18),
                    failure: AudioFailure::new(
                        AudioFailureKind::TerminationUnconfirmed,
                        AudioInitPhase::Runtime,
                    ),
                },
            ))
            .unwrap();
        fixture.finish_teardown();
        assert!(!fixture
            .sink
            .recorded
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|(_, event)| matches!(
                event,
                AudioLifecycleEvent::StartupDiagnostic(
                    audio::AudioStartupDiagnostic::WorkerStopped { .. }
                        | audio::AudioStartupDiagnostic::WorkerInvariantViolation
                )
            )));
        assert_eq!(
            fixture.sink.metrics.get(&completed.run_id).unwrap(),
            Some(completed)
        );
        shutdown(&fixture.supervisor);
    }
}

#[test]
fn cold_sealed_delivery_late_exit_amends_its_completed_run() {
    let fixture = Fixture::new(false);
    fixture.start(18);
    let completed = fixture.stop_and_persist(18);
    assert_eq!(fixture.supervisor.public.phase(), PublicPhase::Stopping);
    assert_eq!(
        completed
            .production
            .as_ref()
            .unwrap()
            .capture
            .stop_to_worker_exit_ms,
        None
    );
    fixture.finish_teardown();
    fixture.assert_late_exit(completed);
    shutdown(&fixture.supervisor);
}

#[test]
fn unconfirmed_termination_directly_in_standby_cannot_amend_completed_run() {
    let fixture = Fixture::new(true);
    fixture.start(18);
    let completed = fixture.stop_and_persist(18);
    assert_eq!(fixture.supervisor.public.phase(), PublicPhase::Standby);
    fixture
        .supervisor
        .sender
        .send(SupervisorMessage::Worker(
            AudioWorkerEvent::TerminationUnconfirmed {
                owner: AudioOwner::Dictation(18),
                failure: AudioFailure::new(
                    AudioFailureKind::TerminationUnconfirmed,
                    AudioInitPhase::Runtime,
                ),
            },
        ))
        .unwrap();
    fixture
        .stopped
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    fixture.finish_teardown();
    assert!(fixture
        .sink
        .recorded
        .events
        .lock()
        .unwrap()
        .iter()
        .any(|(_, event)| matches!(
            event,
            AudioLifecycleEvent::InitializationFailed {
                kind: AudioFailureKind::TerminationUnconfirmed,
                ..
            }
        )));
    fixture.sink.metrics.flush_production_for_test(18);
    assert_eq!(
        fixture.sink.metrics.get(&completed.run_id).unwrap(),
        Some(completed)
    );
    shutdown(&fixture.supervisor);
}

#[test]
fn genuine_cold_stop_unconfirmed_termination_still_amends_its_run() {
    let fixture = Fixture::new(false);
    fixture.start(18);
    let mut expected = fixture.stop_and_persist(18);
    fixture
        .supervisor
        .sender
        .send(SupervisorMessage::Worker(
            AudioWorkerEvent::TerminationUnconfirmed {
                owner: AudioOwner::Dictation(18),
                failure: AudioFailure::new(
                    AudioFailureKind::TerminationUnconfirmed,
                    AudioInitPhase::Runtime,
                ),
            },
        ))
        .unwrap();
    fixture.finish_teardown();
    let capture = &mut expected.production.as_mut().unwrap().capture;
    capture.stop_to_worker_exit_ms = Some(212);
    capture.worker_invariant_violation = Some(true);
    fixture.sink.metrics.flush_production_for_test(18);
    assert_eq!(
        fixture.sink.metrics.get(&expected.run_id).unwrap(),
        Some(expected)
    );
    shutdown(&fixture.supervisor);
}

#[test]
fn reused_standby_workers_next_recording_stop_only_amends_the_next_run() {
    let fixture = Fixture::new(true);
    fixture.start(18);
    let previous = fixture.stop_and_persist(18);
    fixture
        .buffer
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .append_pcm(&[0.5; 160], WHISPER_SAMPLE_RATE);
    fixture.start(19);
    fixture
        .supervisor
        .sender
        .send(SupervisorMessage::ConfigureStandby(StandbyPolicy::default()))
        .unwrap();
    let next = fixture.stop_and_persist(19);
    fixture.finish_teardown();
    // The raw worker exit still bears its original owner (18). The supervisor
    // binds it to the accepted stop of logical recording 19, never run 18.
    fixture.assert_late_exit(next);
    assert_eq!(
        fixture.sink.metrics.get(&previous.run_id).unwrap(),
        Some(previous)
    );
    shutdown(&fixture.supervisor);
}
