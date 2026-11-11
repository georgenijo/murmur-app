// Hardware-free ownership, adoption, and privacy tests for Fn key-down capture.
use super::*;

/// Each spawn retains a distinct sample value so tests can prove which worker's
/// audio reached a recording.
/// Production helpers keep producing PCM after Stop: in-flight callbacks and
/// the ring flush. This worker models that, and can withhold confirmed exit.
#[derive(Clone)]
struct AfterStop {
    appended: Arc<AtomicUsize>,
    unconfirmed: bool,
    exit: Gate,
}

struct KeyDownFactory {
    first_buffer: Option<Gate>,
    teardown: Option<Gate>,
    after_stop: Option<AfterStop>,
    buffers: Arc<Mutex<Vec<Arc<Mutex<audio::CaptureSamples>>>>>,
    owners: Arc<Mutex<Vec<AudioOwner>>>,
    active: Arc<Mutex<Vec<Arc<AtomicBool>>>>,
    spawns: Arc<AtomicUsize>,
}

const KEY_DOWN_PCM: f32 = 0.125;
const COLD_PCM: f32 = 0.875;

impl WorkerFactory for KeyDownFactory {
    fn spawn(&self, spec: AudioWorkerSpec, events: AudioWorkerEventSender) -> Result<JoinHandle<()>, String> {
        let index = self.spawns.fetch_add(1, Ordering::SeqCst);
        self.buffers.lock().unwrap().push(spec.shared.clone());
        self.owners.lock().unwrap().push(spec.owner);
        self.active.lock().unwrap().push(spec.active.clone());
        let first_buffer = if index == 0 { self.first_buffer.clone() } else { None };
        let teardown = if index == 0 { self.teardown.clone() } else { None };
        let after_stop = if index == 0 { self.after_stop.clone() } else { None };
        let value = if index == 0 { KEY_DOWN_PCM } else { COLD_PCM };
        Ok(std::thread::spawn(move || {
            if let Some(gate) = first_buffer {
                gate.wait();
            }
            audio::retain_pcm(&spec.shared, spec.owner, &[value; 160], 16_000);
            events.send(AudioWorkerEvent::FirstBuffer { owner: spec.owner, sample_rate: 16_000, sent_at: Instant::now() }).unwrap();
            while let Ok(command) = spec.command_receiver.recv() {
                if matches!(command, AudioCommand::Stop) {
                    break;
                }
            }
            if let Some(after) = &after_stop {
                // The ring flush the helper sends in reply to Stop.
                audio::retain_pcm(&spec.shared, spec.owner, &[value; 512], 16_000);
                after.appended.fetch_add(1, Ordering::SeqCst);
            }
            if let Some(gate) = teardown {
                gate.wait();
            }
            if let Some(after) = &after_stop {
                // A callback still in the pipe while exit is being confirmed.
                audio::retain_pcm(&spec.shared, spec.owner, &[value; 64], 16_000);
                after.appended.fetch_add(1, Ordering::SeqCst);
                if after.unconfirmed {
                    let _ = events.send(AudioWorkerEvent::TerminationUnconfirmed {
                        owner: spec.owner,
                        failure: AudioFailure::new(AudioFailureKind::TerminationUnconfirmed, AudioInitPhase::Runtime),
                    });
                }
                after.exit.wait();
            }
            let _ = events.send(AudioWorkerEvent::StreamStopped { owner: spec.owner });
            let _ = events.send(AudioWorkerEvent::ThreadExited { owner: spec.owner });
        }))
    }
}

struct KeyDown {
    supervisor: AudioSupervisor,
    sink: Arc<RecordingSink>,
    buffers: Arc<Mutex<Vec<Arc<Mutex<audio::CaptureSamples>>>>>,
    owners: Arc<Mutex<Vec<AudioOwner>>>,
    active: Arc<Mutex<Vec<Arc<AtomicBool>>>>,
    spawns: Arc<AtomicUsize>,
}

fn key_down(first_buffer: Option<Gate>, teardown: Option<Gate>) -> KeyDown {
    key_down_with(first_buffer, teardown, None)
}

fn key_down_with(first_buffer: Option<Gate>, teardown: Option<Gate>, after_stop: Option<AfterStop>) -> KeyDown {
    let buffers = Arc::new(Mutex::new(Vec::new()));
    let owners = Arc::new(Mutex::new(Vec::new()));
    let active = Arc::new(Mutex::new(Vec::new()));
    let spawns = Arc::new(AtomicUsize::new(0));
    let sink = Arc::new(RecordingSink::default());
    let supervisor = spawn_supervisor(
        Arc::new(KeyDownFactory { first_buffer, teardown, after_stop, buffers: buffers.clone(), owners: owners.clone(), active: active.clone(), spawns: spawns.clone() }),
        sink.clone(),
        SupervisorConfig::default(),
    );
    KeyDown { supervisor, sink, buffers, owners, active, spawns }
}

fn open(supervisor: &AudioSupervisor, id: u64, device_id: Option<&str>) -> bool {
    open_pressed(supervisor, id, device_id, Instant::now())
}

fn open_pressed(supervisor: &AudioSupervisor, id: u64, device_id: Option<&str>, pressed_at: Instant) -> bool {
    let (response, admitted) = mpsc::channel();
    supervisor
        .sender
        .send(SupervisorMessage::SpeculativeStart(SpeculativeStartRequest { id, pressed_at, device_id: device_id.map(str::to_string), app_handle: None, response }))
        .unwrap();
    admitted.recv_timeout(Duration::from_secs(1)).unwrap()
}

fn discard(supervisor: &AudioSupervisor, id: u64) {
    supervisor
        .sender
        .send(SupervisorMessage::DiscardSpeculative { id, reason: speculative::DiscardReason::NotConfirmed })
        .unwrap();
}

fn confirmed_start(supervisor: &AudioSupervisor, owner: AudioOwner, capture: Option<u64>, device_id: Option<&str>) -> Receiver<Result<(), AudioStartError>> {
    let (response, started) = mpsc::channel();
    supervisor
        .sender
        .send(SupervisorMessage::Start(StartRequest {
            gesture: gesture::StartGesture { speculative_capture: capture, ..Default::default() },
            caller_alive: Arc::new(AtomicBool::new(true)),
            owner,
            app_handle: None,
            device_id: device_id.map(str::to_string),
            origin: "hold".to_string(),
            wait_until_ready: false,
            response,
            probe_permit: None,
        }))
        .unwrap();
    started
}

fn stop_samples(supervisor: &AudioSupervisor, owner: AudioOwner) -> Vec<f32> {
    let (response, samples) = mpsc::channel();
    supervisor.sender.send(SupervisorMessage::Stop { trigger: DictationStopTrigger::HoldRelease, owner: Some(owner), response }).unwrap();
    samples.recv_timeout(Duration::from_secs(2)).unwrap().unwrap()
}

fn dictation_events(sink: &RecordingSink) -> Vec<(AudioOwner, AudioLifecycleEvent)> {
    sink.events.lock().unwrap().iter().filter(|(owner, _)| owner.dictation_id().is_some()).cloned().collect()
}

fn wait_flowing(supervisor: &AudioSupervisor, id: u64) {
    wait_until("key-down capture never retained PCM", || supervisor.public.flowing_speculative.load(Ordering::SeqCst) == id);
}

#[test]
fn flowing_key_down_audio_is_adopted_by_its_confirmed_start_without_a_second_worker() {
    let t = key_down(None, None);
    assert!(open(&t.supervisor, 11, None));
    wait_flowing(&t.supervisor, 11);
    assert_eq!(t.supervisor.public.phase(), PublicPhase::Speculative);
    // Unconfirmed audio is invisible: no dictation lifecycle and no level meter.
    assert!(dictation_events(&t.sink).is_empty());
    assert!(!t.active.lock().unwrap()[0].load(Ordering::SeqCst));

    let owner = AudioOwner::Dictation(21);
    assert_eq!(sample_cursor_in(&t.supervisor.public, 21), None, "no cursor before adoption");
    assert_eq!(confirmed_start(&t.supervisor, owner, Some(11), None).recv().unwrap(), Ok(()));
    assert!(t.supervisor.public.is_recording_for(owner), "flowing adoption skips the connecting phase");
    assert_eq!(t.spawns.load(Ordering::SeqCst), 1);
    assert!(t.active.lock().unwrap()[0].load(Ordering::SeqCst));
    let events = dictation_events(&t.sink);
    let accepted = events.iter().position(|e| *e == (owner, AudioLifecycleEvent::Accepted)).unwrap();
    let ready = events.iter().position(|e| *e == (owner, AudioLifecycleEvent::Ready)).unwrap();
    assert!(accepted < ready);
    assert_eq!(t.supervisor.public.flowing_speculative.load(Ordering::SeqCst), 0);
    // The native stop-press stamp can read the adopted buffer (16 kHz here).
    assert_eq!(sample_cursor_in(&t.supervisor.public, 21), Some(160));
    assert_eq!(sample_cursor_in(&t.supervisor.public, 22), None);

    // Audio retained since key-down is the start of the transcribed recording.
    assert_eq!(stop_samples(&t.supervisor, owner), vec![KEY_DOWN_PCM; 160]);
    wait_until("adopted worker joined", || t.supervisor.public.phase() == PublicPhase::Idle);
    assert_eq!(sample_cursor_in(&t.supervisor.public, 21), None);
    shutdown(&t.supervisor);
}

#[test]
fn adoption_before_first_pcm_continues_as_an_ordinary_start() {
    let first_buffer = Gate::closed();
    let t = key_down(Some(first_buffer.clone()), None);
    assert!(open(&t.supervisor, 12, None));
    let owner = AudioOwner::Dictation(22);
    assert_eq!(confirmed_start(&t.supervisor, owner, Some(12), None).recv().unwrap(), Ok(()));
    wait_until("adopted start phase", || t.supervisor.public.phase() == PublicPhase::Starting);
    assert!(!dictation_events(&t.sink).iter().any(|(_, e)| *e == AudioLifecycleEvent::Ready));
    first_buffer.open();
    wait_until("ready on first buffer", || t.supervisor.public.is_recording_for(owner));
    assert!(dictation_events(&t.sink).contains(&(owner, AudioLifecycleEvent::Ready)));
    assert_eq!(t.spawns.load(Ordering::SeqCst), 1);
    assert_eq!(stop_samples(&t.supervisor, owner), vec![KEY_DOWN_PCM; 160]);
    shutdown(&t.supervisor);
}

#[test]
fn unconfirmed_capture_clears_samples_and_holds_ownership_until_confirmed_exit() {
    let teardown = Gate::closed();
    let t = key_down(None, Some(teardown.clone()));
    assert!(open(&t.supervisor, 13, None));
    wait_flowing(&t.supervisor, 13);
    let discarded_at = Instant::now();
    discard(&t.supervisor, 13);
    wait_until("discard reached the supervisor", || t.supervisor.public.phase() == PublicPhase::Recovering);
    // Samples are dropped before teardown completes, and the microphone stays
    // exclusively owned until the exact worker has exited.
    assert!(t.buffers.lock().unwrap()[0].lock().unwrap().is_empty());
    assert!(t.supervisor.public.is_active());
    assert!(!open(&t.supervisor, 14, None), "no second key-down capture during teardown");
    teardown.open();
    wait_until("discarded worker joined", || t.supervisor.public.phase() == PublicPhase::Idle);
    assert!(discarded_at.elapsed() < Duration::from_millis(500));
    assert!(dictation_events(&t.sink).is_empty(), "unconfirmed audio never reaches a dictation owner");
    assert!(!t.active.lock().unwrap()[0].load(Ordering::SeqCst));
    assert_eq!(t.spawns.load(Ordering::SeqCst), 1);
    shutdown(&t.supervisor);
}

#[test]
fn backstop_discards_a_capture_no_start_adopted() {
    let t = key_down(None, None);
    assert!(open(&t.supervisor, 15, None));
    wait_flowing(&t.supervisor, 15);
    check_deadlines(&t.supervisor, speculative::SPECULATIVE_CAPTURE_TTL - Duration::from_millis(1));
    assert_eq!(t.supervisor.public.phase(), PublicPhase::Speculative);
    check_deadlines(&t.supervisor, speculative::SPECULATIVE_CAPTURE_TTL);
    wait_until("expired capture joined", || t.supervisor.public.phase() == PublicPhase::Idle);
    assert!(t.buffers.lock().unwrap()[0].lock().unwrap().is_empty());
    assert!(dictation_events(&t.sink).is_empty());
    shutdown(&t.supervisor);
}

#[test]
fn backstop_runs_from_physical_key_down_not_supervisor_acceptance() {
    let t = key_down(None, None);
    // Admission was delayed: Fn went down 1.3 s before the supervisor saw it.
    let lag = speculative::SPECULATIVE_CAPTURE_TTL - Duration::from_millis(200);
    assert!(open_pressed(&t.supervisor, 34, None, Instant::now() - lag));
    wait_flowing(&t.supervisor, 34);
    // `check_deadlines` measures from acceptance, which is at least `lag`
    // after key-down, so the capture expires about 200 ms after acceptance.
    check_deadlines(&t.supervisor, Duration::from_millis(100));
    assert_eq!(t.supervisor.public.phase(), PublicPhase::Speculative);
    check_deadlines(&t.supervisor, Duration::from_millis(200));
    wait_until("key-down backstop joined", || t.supervisor.public.phase() == PublicPhase::Idle);
    assert!(t.buffers.lock().unwrap()[0].lock().unwrap().is_empty());

    // A request whose backstop already elapsed is never opened.
    assert!(!open_pressed(&t.supervisor, 35, None, Instant::now() - speculative::SPECULATIVE_CAPTURE_TTL));
    assert_eq!(t.spawns.load(Ordering::SeqCst), 1);
    shutdown(&t.supervisor);
}

#[test]
fn non_matching_start_discards_key_down_audio_and_starts_cold_after_exit() {
    for (capture, device) in [(None, None), (Some(99), None), (Some(16), Some("other-input"))] {
        let teardown = Gate::closed();
        let t = key_down(None, Some(teardown.clone()));
        assert!(open(&t.supervisor, 16, None));
        wait_flowing(&t.supervisor, 16);
        let owner = AudioOwner::Dictation(23);
        let started = confirmed_start(&t.supervisor, owner, capture, device);
        wait_until("mismatch began teardown", || t.supervisor.public.phase() == PublicPhase::Recovering);
        assert!(t.buffers.lock().unwrap()[0].lock().unwrap().is_empty());
        assert_eq!(t.spawns.load(Ordering::SeqCst), 1, "cold start waits for confirmed exit");
        teardown.open();
        assert_eq!(started.recv_timeout(Duration::from_secs(1)).unwrap(), Ok(()));
        assert_eq!(t.spawns.load(Ordering::SeqCst), 2);
        assert_eq!(t.owners.lock().unwrap()[1], owner);
        wait_until("cold ready", || t.supervisor.public.is_recording_for(owner));
        assert_eq!(stop_samples(&t.supervisor, owner), vec![COLD_PCM; 160]);
        shutdown(&t.supervisor);
    }
}

#[test]
fn second_gesture_during_teardown_queues_its_start_behind_the_exact_exit() {
    let teardown = Gate::closed();
    let t = key_down(None, Some(teardown.clone()));
    assert!(open(&t.supervisor, 17, None));
    discard(&t.supervisor, 17);
    // The next Fn press is refused speculation; its confirmed start queues.
    assert!(!open(&t.supervisor, 18, None));
    let owner = AudioOwner::Dictation(24);
    let started = confirmed_start(&t.supervisor, owner, Some(18), None);
    assert!(started.recv_timeout(Duration::from_millis(50)).is_err());
    teardown.open();
    assert_eq!(started.recv_timeout(Duration::from_secs(1)).unwrap(), Ok(()));
    assert_eq!(t.spawns.load(Ordering::SeqCst), 2);
    wait_until("queued start ready", || t.supervisor.public.is_recording_for(owner));
    assert_eq!(stop_samples(&t.supervisor, owner), vec![COLD_PCM; 160]);
    shutdown(&t.supervisor);
}

#[test]
fn ownerless_stop_peek_and_stale_discard_cannot_read_or_leak_key_down_audio() {
    let t = key_down(None, None);
    assert!(open(&t.supervisor, 19, None));
    wait_flowing(&t.supervisor, 19);
    let (response, samples) = mpsc::channel();
    t.supervisor.sender.send(SupervisorMessage::Stop { trigger: DictationStopTrigger::Other, owner: None, response }).unwrap();
    assert_eq!(samples.recv().unwrap(), Ok(Vec::new()));
    let (response, peeked) = mpsc::channel();
    t.supervisor.sender.send(SupervisorMessage::Peek { owner: AudioOwner::Dictation(19), max_output_samples: None, response }).unwrap();
    assert_eq!(peeked.recv().unwrap(), None);
    discard(&t.supervisor, 20);
    assert_eq!(cancel_any_phase(&t.supervisor, AudioOwner::Dictation(19)).recv().unwrap(), Ok(false));
    assert_eq!(t.supervisor.public.phase(), PublicPhase::Speculative);
    // Sleep, wake, and device changes cancel starting captures with no owner.
    let (response, cancelled) = mpsc::channel();
    t.supervisor.sender.send(SupervisorMessage::Cancel { owner: None, reason: AudioCancelReason::SystemSleep, starting_only: true, response }).unwrap();
    assert_eq!(cancelled.recv().unwrap(), Ok(true));
    wait_until("environment discard joined", || t.supervisor.public.phase() == PublicPhase::Idle);
    assert!(dictation_events(&t.sink).is_empty());
    shutdown(&t.supervisor);
}

#[test]
fn another_microphone_feature_retires_key_down_capture_before_claiming() {
    let t = key_down(None, None);
    assert!(open(&t.supervisor, 25, None));
    wait_flowing(&t.supervisor, 25);
    let (response, retired) = mpsc::channel();
    t.supervisor.sender.send(SupervisorMessage::RetireStandby(response)).unwrap();
    retired.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(t.supervisor.public.phase(), PublicPhase::Idle);
    assert!(t.buffers.lock().unwrap()[0].lock().unwrap().is_empty());
    shutdown(&t.supervisor);
}

#[test]
fn busy_microphone_refuses_key_down_capture() {
    let t = key_down(None, None);
    let owner = AudioOwner::Dictation(26);
    assert_eq!(confirmed_start(&t.supervisor, owner, None, None).recv().unwrap(), Ok(()));
    wait_until("recording", || t.supervisor.public.is_recording_for(owner));
    assert!(!open(&t.supervisor, 27, None));
    assert_eq!(t.spawns.load(Ordering::SeqCst), 1);
    assert_eq!(stop_samples(&t.supervisor, owner), vec![KEY_DOWN_PCM; 160]);
    shutdown(&t.supervisor);
}

#[test]
fn discarded_capture_stays_empty_through_stop_flush_late_pcm_and_unconfirmed_exit() {
    let teardown = Gate::closed();
    let exit = Gate::closed();
    let appended = Arc::new(AtomicUsize::new(0));
    let after = AfterStop { appended: appended.clone(), unconfirmed: true, exit: exit.clone() };
    let t = key_down_with(None, Some(teardown.clone()), Some(after));
    assert!(open(&t.supervisor, 31, None));
    wait_flowing(&t.supervisor, 31);
    discard(&t.supervisor, 31);
    let buffer = t.buffers.lock().unwrap()[0].clone();
    wait_until("helper flushed its ring after Stop", || appended.load(Ordering::SeqCst) == 1);
    assert!(buffer.lock().unwrap().is_empty(), "Stop-flushed PCM repopulated the discarded buffer");
    teardown.open();
    wait_until("late PCM arrived while exit was pending", || appended.load(Ordering::SeqCst) == 2);
    assert!(buffer.lock().unwrap().is_empty(), "late PCM repopulated the discarded buffer");
    // Quarantine keeps the attempt (and its buffer) while exit is unconfirmed;
    // it must hold nothing.
    let (response, peeked) = mpsc::channel();
    t.supervisor.sender.send(SupervisorMessage::Peek { owner: AudioOwner::Dictation(31), max_output_samples: None, response }).unwrap();
    assert_eq!(peeked.recv().unwrap(), None);
    assert_eq!(t.supervisor.public.phase(), PublicPhase::Recovering);
    assert!(t.supervisor.public.is_active(), "ownership kept until exit is confirmed");
    {
        let samples = buffer.lock().unwrap();
        assert!(samples.is_empty());
        assert!(samples.retention_revoked());
    }
    assert!(dictation_events(&t.sink).is_empty());
    exit.open();
    wait_until("worker exit confirmed", || t.supervisor.public.phase() == PublicPhase::Idle);
    assert!(buffer.lock().unwrap().is_empty());
    shutdown(&t.supervisor);
}

#[test]
fn adopted_capture_keeps_stop_flushed_pcm() {
    let exit = Gate::closed();
    exit.open();
    let appended = Arc::new(AtomicUsize::new(0));
    let after = AfterStop { appended: appended.clone(), unconfirmed: false, exit };
    let t = key_down_with(None, None, Some(after));
    assert!(open(&t.supervisor, 32, None));
    wait_flowing(&t.supervisor, 32);
    let owner = AudioOwner::Dictation(33);
    assert_eq!(confirmed_start(&t.supervisor, owner, Some(32), None).recv().unwrap(), Ok(()));
    // Key-down PCM, the Stop ring flush, and the last in-flight callback all
    // belong to the adopted recording.
    assert_eq!(stop_samples(&t.supervisor, owner), vec![KEY_DOWN_PCM; 160 + 512 + 64]);
    assert_eq!(appended.load(Ordering::SeqCst), 2);
    shutdown(&t.supervisor);
}

fn retire(supervisor: &AudioSupervisor) {
    let (response, retired) = mpsc::channel();
    supervisor.sender.send(SupervisorMessage::RetireStandby(response)).unwrap();
    retired.recv_timeout(Duration::from_secs(2)).unwrap();
}

/// The Settings mic test starts while an Fn press holds an unconfirmed
/// capture. It supersedes that capture instead of reporting the microphone or
/// a dictation as busy, and no dictation lifecycle is ever published.
#[test]
fn mic_test_supersedes_an_unconfirmed_key_down_capture() {
    let t = key_down(None, None);
    assert!(open(&t.supervisor, 41, None));
    wait_flowing(&t.supervisor, 41);
    assert!(holds_only_unconfirmed_key_down(&t.supervisor.public));
    retire(&t.supervisor);
    assert_eq!(t.supervisor.public.phase(), PublicPhase::Idle);
    assert!(!holds_only_unconfirmed_key_down(&t.supervisor.public));
    assert!(t.buffers.lock().unwrap()[0].lock().unwrap().is_empty());
    assert!(dictation_events(&t.sink).is_empty(), "a discarded key-down capture is not a dictation");
    let preview = AudioOwner::Preview(5);
    assert_eq!(confirmed_start(&t.supervisor, preview, None, None).recv().unwrap(), Ok(()));
    shutdown(&t.supervisor);
}

#[test]
fn an_adopted_key_down_capture_is_a_dictation_the_mic_test_never_supersedes() {
    let t = key_down(None, None);
    assert!(open(&t.supervisor, 42, None));
    wait_flowing(&t.supervisor, 42);
    let owner = AudioOwner::Dictation(43);
    assert_eq!(confirmed_start(&t.supervisor, owner, Some(42), None).recv().unwrap(), Ok(()));
    assert!(!holds_only_unconfirmed_key_down(&t.supervisor.public));
    // The retire the mic test sends before its busy check leaves it alone.
    retire(&t.supervisor);
    assert!(t.supervisor.public.is_recording_for(owner));
    assert_eq!(stop_samples(&t.supervisor, owner), vec![KEY_DOWN_PCM; 160]);
    shutdown(&t.supervisor);
}

/// A worker that refuses to open, as the stale helper did natively (it could
/// not decode `StartPreauthorized` and exited ~6 ms after the start was sent).
struct RefusingFactory {
    kind: AudioFailureKind,
}

impl WorkerFactory for RefusingFactory {
    fn spawn(&self, spec: AudioWorkerSpec, events: AudioWorkerEventSender) -> Result<JoinHandle<()>, String> {
        let kind = self.kind;
        Ok(std::thread::spawn(move || {
            let _ = events.send(AudioWorkerEvent::InitFailed {
                owner: spec.owner,
                failure: AudioFailure::new(kind, AudioInitPhase::StreamBuild),
            });
            let _ = events.send(AudioWorkerEvent::ThreadExited { owner: spec.owner });
        }))
    }
}

#[test]
fn a_failed_key_down_open_leaves_no_owner_dictation_or_busy_microphone() {
    for kind in [AudioFailureKind::ProtocolError, AudioFailureKind::PermissionDenied] {
        let sink = Arc::new(RecordingSink::default());
        let supervisor = spawn_supervisor(Arc::new(RefusingFactory { kind }), sink.clone(), SupervisorConfig::default());
        assert!(open(&supervisor, 51, None), "{kind:?}: admission precedes the worker's refusal");
        wait_until("failed key-down capture released the microphone", || supervisor.public.phase() == PublicPhase::Idle);
        assert!(!supervisor.public.is_active(), "{kind:?}");
        assert!(!holds_only_unconfirmed_key_down(&supervisor.public), "{kind:?}");
        assert!(dictation_events(&sink).is_empty(), "{kind:?}: no dictation was ever reported");
        // The production sink drops every Speculative-owner event; even so, the
        // supervisor publishes nothing past acceptance: no failure, recovery, or
        // interruption that Settings could read as a dictation in progress.
        let events = sink.events.lock().unwrap().clone();
        assert!(
            events.iter().all(|(owner, event)| matches!(owner, AudioOwner::Speculative(51)) && *event == AudioLifecycleEvent::Accepted),
            "{kind:?}: {events:?}"
        );
        shutdown(&supervisor);
    }
}

/// Review finding: the command-level race check. A key-down capture admitted
/// after the preview command's first retire must be superseded by the check
/// the command runs under its transition lock, not reported as busy. Drives
/// the command's own decision function against a live supervisor.
#[test]
fn preview_command_supersedes_a_key_down_capture_admitted_after_its_first_retire() {
    let t = key_down(None, None);
    // The command's first retire found nothing; then an Fn press won the lock.
    retire(&t.supervisor);
    assert!(open(&t.supervisor, 61, None));
    wait_flowing(&t.supervisor, 61);
    let blocker = crate::commands::microphone_preview::preview_audio_blocker(
        || holds_only_unconfirmed_key_down(&t.supervisor.public),
        || retire(&t.supervisor),
        || t.supervisor.public.is_active(),
    );
    assert_eq!(blocker, None, "the mic test must not report the key-down capture as busy");
    assert_eq!(t.supervisor.public.phase(), PublicPhase::Idle);
    assert!(t.buffers.lock().unwrap()[0].lock().unwrap().is_empty());
    assert!(dictation_events(&t.sink).is_empty());
    shutdown(&t.supervisor);
}

#[test]
fn preview_command_still_reports_a_dictation_owned_microphone_busy() {
    let t = key_down(None, None);
    assert!(open(&t.supervisor, 62, None));
    wait_flowing(&t.supervisor, 62);
    let owner = AudioOwner::Dictation(63);
    assert_eq!(confirmed_start(&t.supervisor, owner, Some(62), None).recv().unwrap(), Ok(()));
    let blocker = crate::commands::microphone_preview::preview_audio_blocker(
        || holds_only_unconfirmed_key_down(&t.supervisor.public),
        || retire(&t.supervisor),
        || t.supervisor.public.is_active(),
    );
    assert_eq!(blocker, Some(crate::commands::microphone_preview::PREVIEW_AUDIO_BUSY));
    assert!(t.supervisor.public.is_recording_for(owner), "a dictation is never superseded");
    assert_eq!(stop_samples(&t.supervisor, owner), vec![KEY_DOWN_PCM; 160]);
    shutdown(&t.supervisor);
}
