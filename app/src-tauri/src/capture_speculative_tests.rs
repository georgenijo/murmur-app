// Hardware-free ownership, adoption, and privacy tests for Fn key-down capture.
use super::*;

/// Each spawn retains a distinct sample value so tests can prove which worker's
/// audio reached a recording.
struct KeyDownFactory {
    first_buffer: Option<Gate>,
    teardown: Option<Gate>,
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
        let value = if index == 0 { KEY_DOWN_PCM } else { COLD_PCM };
        Ok(std::thread::spawn(move || {
            if let Some(gate) = first_buffer {
                gate.wait();
            }
            spec.shared.lock().unwrap().append_pcm(&[value; 160], 16_000);
            events.send(AudioWorkerEvent::FirstBuffer { owner: spec.owner, sample_rate: 16_000, sent_at: Instant::now() }).unwrap();
            while let Ok(command) = spec.command_receiver.recv() {
                if matches!(command, AudioCommand::Stop) {
                    break;
                }
            }
            if let Some(gate) = teardown {
                gate.wait();
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
    let buffers = Arc::new(Mutex::new(Vec::new()));
    let owners = Arc::new(Mutex::new(Vec::new()));
    let active = Arc::new(Mutex::new(Vec::new()));
    let spawns = Arc::new(AtomicUsize::new(0));
    let sink = Arc::new(RecordingSink::default());
    let supervisor = spawn_supervisor(
        Arc::new(KeyDownFactory { first_buffer, teardown, buffers: buffers.clone(), owners: owners.clone(), active: active.clone(), spawns: spawns.clone() }),
        sink.clone(),
        SupervisorConfig::default(),
    );
    KeyDown { supervisor, sink, buffers, owners, active, spawns }
}

fn open(supervisor: &AudioSupervisor, id: u64, device_id: Option<&str>) -> bool {
    let (response, admitted) = mpsc::channel();
    supervisor
        .sender
        .send(SupervisorMessage::SpeculativeStart(SpeculativeStartRequest { id, device_id: device_id.map(str::to_string), app_handle: None, response }))
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
    assert_eq!(confirmed_start(&t.supervisor, owner, Some(11), None).recv().unwrap(), Ok(()));
    assert!(t.supervisor.public.is_recording_for(owner), "flowing adoption skips the connecting phase");
    assert_eq!(t.spawns.load(Ordering::SeqCst), 1);
    assert!(t.active.lock().unwrap()[0].load(Ordering::SeqCst));
    let events = dictation_events(&t.sink);
    let accepted = events.iter().position(|e| *e == (owner, AudioLifecycleEvent::Accepted)).unwrap();
    let ready = events.iter().position(|e| *e == (owner, AudioLifecycleEvent::Ready)).unwrap();
    assert!(accepted < ready);
    assert_eq!(t.supervisor.public.flowing_speculative.load(Ordering::SeqCst), 0);

    // Audio retained since key-down is the start of the transcribed recording.
    assert_eq!(stop_samples(&t.supervisor, owner), vec![KEY_DOWN_PCM; 160]);
    wait_until("adopted worker joined", || t.supervisor.public.phase() == PublicPhase::Idle);
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
