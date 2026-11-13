// Hardware-free tests for handing a running Settings microphone test to a
// dictation without closing and reopening the device (issue: an open mic test
// made every dictation start ~200 ms late and clipped the first word).
use super::*;

/// What the fake device is "hearing". The preview phase and the dictation
/// phase use different values so a test can prove which audio was retained.
const PREVIEW_PCM: f32 = 0.25;
const DICTATION_PCM: f32 = 0.75;
const COLD_PCM: f32 = 0.5;

/// A worker that streams PCM continuously until Stop, like a real helper.
/// `paused` is held across every append, so taking it guarantees no frame is
/// in flight; a paused worker produces nothing.
struct StreamingFactory {
    first_buffer: Option<Gate>,
    hearing: Arc<Mutex<f32>>,
    paused: Arc<Mutex<bool>>,
    /// Report a runtime failure, then hold teardown until this gate opens.
    fail_after_ready: Option<(Arc<AtomicBool>, Gate)>,
    buffers: Arc<Mutex<Vec<Arc<Mutex<audio::CaptureSamples>>>>>,
    active: Arc<Mutex<Vec<Arc<AtomicBool>>>>,
    spawns: Arc<AtomicUsize>,
}

impl WorkerFactory for StreamingFactory {
    fn spawn(&self, spec: AudioWorkerSpec, events: AudioWorkerEventSender) -> Result<JoinHandle<()>, String> {
        let index = self.spawns.fetch_add(1, Ordering::SeqCst);
        self.buffers.lock().unwrap().push(spec.shared.clone());
        self.active.lock().unwrap().push(spec.active.clone());
        let first_buffer = if index == 0 { self.first_buffer.clone() } else { None };
        let hearing = self.hearing.clone();
        let paused = self.paused.clone();
        let fail = if index == 0 { self.fail_after_ready.clone() } else { None };
        let cold = index > 0;
        Ok(std::thread::spawn(move || {
            if let Some(gate) = first_buffer {
                gate.wait();
            }
            let frame = |hearing: &Mutex<f32>| [if cold { COLD_PCM } else { *hearing.lock().unwrap() }; 16];
            audio::retain_pcm(&spec.shared, spec.owner, &frame(&hearing), 16_000);
            events.send(AudioWorkerEvent::FirstBuffer { owner: spec.owner, sample_rate: 16_000, sent_at: Instant::now() }).unwrap();
            loop {
                if let Some((trigger, teardown)) = &fail {
                    if trigger.load(Ordering::SeqCst) {
                        events
                            .send(AudioWorkerEvent::RuntimeFailed {
                                owner: spec.owner,
                                failure: AudioFailure::new(AudioFailureKind::BackendError, AudioInitPhase::Runtime),
                            })
                            .unwrap();
                        teardown.wait();
                        break;
                    }
                }
                match spec.command_receiver.recv_timeout(Duration::from_millis(1)) {
                    Ok(AudioCommand::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {
                        let paused = paused.lock().unwrap();
                        if !*paused {
                            audio::retain_pcm(&spec.shared, spec.owner, &frame(&hearing), 16_000);
                        }
                    }
                }
            }
            let _ = events.send(AudioWorkerEvent::StreamStopped { owner: spec.owner });
            let _ = events.send(AudioWorkerEvent::ThreadExited { owner: spec.owner });
        }))
    }
}

struct Handoff {
    supervisor: AudioSupervisor,
    sink: Arc<RecordingSink>,
    hearing: Arc<Mutex<f32>>,
    paused: Arc<Mutex<bool>>,
    buffers: Arc<Mutex<Vec<Arc<Mutex<audio::CaptureSamples>>>>>,
    active: Arc<Mutex<Vec<Arc<AtomicBool>>>>,
    spawns: Arc<AtomicUsize>,
}

fn handoff(first_buffer: Option<Gate>) -> Handoff {
    handoff_with(first_buffer, None)
}

fn handoff_with(first_buffer: Option<Gate>, fail_after_ready: Option<(Arc<AtomicBool>, Gate)>) -> Handoff {
    let hearing = Arc::new(Mutex::new(PREVIEW_PCM));
    let paused = Arc::new(Mutex::new(false));
    let buffers = Arc::new(Mutex::new(Vec::new()));
    let active = Arc::new(Mutex::new(Vec::new()));
    let spawns = Arc::new(AtomicUsize::new(0));
    let sink = Arc::new(RecordingSink::default());
    let supervisor = spawn_supervisor(
        Arc::new(StreamingFactory { first_buffer, hearing: hearing.clone(), paused: paused.clone(), fail_after_ready, buffers: buffers.clone(), active: active.clone(), spawns: spawns.clone() }),
        sink.clone(),
        SupervisorConfig::default(),
    );
    Handoff { supervisor, sink, hearing, paused, buffers, active, spawns }
}

impl Handoff {
    fn hear(&self, value: f32) {
        *self.hearing.lock().unwrap() = value;
    }

    /// Stop the fake device between frames; returns once no frame is in flight.
    fn pause(&self) {
        *self.paused.lock().unwrap() = true;
    }

    fn resume(&self) {
        *self.paused.lock().unwrap() = false;
    }

    fn retained(&self, worker: usize) -> Vec<f32> {
        self.buffers.lock().unwrap()[worker].lock().unwrap().to_vec()
    }

    fn events_for(&self, owner: AudioOwner) -> Vec<AudioLifecycleEvent> {
        self.sink.events.lock().unwrap().iter().filter(|(o, _)| *o == owner).map(|(_, e)| e.clone()).collect()
    }

    fn position(&self, wanted: (AudioOwner, AudioLifecycleEvent)) -> Option<usize> {
        self.sink.events.lock().unwrap().iter().position(|event| *event == wanted)
    }
}

fn start(supervisor: &AudioSupervisor, owner: AudioOwner, capture: Option<u64>, device_id: Option<&str>) -> Receiver<Result<(), AudioStartError>> {
    let (response, started) = mpsc::channel();
    supervisor
        .sender
        .send(SupervisorMessage::Start(StartRequest {
            gesture: gesture::StartGesture { speculative_capture: capture, ..Default::default() },
            caller_alive: Arc::new(AtomicBool::new(true)),
            owner,
            app_handle: None,
            device_id: device_id.map(str::to_string),
            origin: if owner.preview_id().is_some() { "preview" } else { "hold" }.to_string(),
            wait_until_ready: false,
            response,
            probe_permit: None,
        }))
        .unwrap();
    started
}

/// The Settings test is open and its meter is live.
fn flowing_preview(t: &Handoff, preview: AudioOwner, device_id: Option<&str>) {
    assert_eq!(start(&t.supervisor, preview, None, device_id).recv().unwrap(), Ok(()));
    wait_until("preview flowing", || t.supervisor.public.is_recording_for(preview));
    // Let the meter run for a while: none of this audio may ever be kept.
    std::thread::sleep(Duration::from_millis(15));
    assert!(t.retained(0).is_empty(), "preview PCM is never retained");
}

fn key_down(t: &Handoff, id: u64, device_id: Option<&str>, adopt_preview: Option<u64>) -> bool {
    let (response, admitted) = mpsc::channel();
    t.supervisor
        .sender
        .send(SupervisorMessage::SpeculativeStart(SpeculativeStartRequest {
            id,
            pressed_at: Instant::now(),
            device_id: device_id.map(str::to_string),
            app_handle: None,
            adopt_preview,
            response,
        }))
        .unwrap();
    admitted.recv_timeout(Duration::from_secs(1)).unwrap()
}

fn stop(t: &Handoff, owner: AudioOwner) -> Vec<f32> {
    let (response, samples) = mpsc::channel();
    t.supervisor.sender.send(SupervisorMessage::Stop { released_at: None, trigger: DictationStopTrigger::HoldRelease, owner: Some(owner), response }).unwrap();
    samples.recv_timeout(Duration::from_secs(2)).unwrap().unwrap()
}

#[test]
fn fn_key_down_takes_over_a_flowing_mic_test_without_reopening_the_device() {
    let t = handoff(None);
    let preview = AudioOwner::Preview(5);
    flowing_preview(&t, preview, None);

    // Fn goes down while the meter is live: the capture is open at once.
    // Pausing between frames makes every frame before the switch below a
    // preview frame and every frame after it a dictation frame, so any kept
    // preview audio would show up in the final assertion.
    t.pause();
    assert!(key_down(&t, 31, None, Some(5)));
    assert_eq!(t.supervisor.public.phase(), PublicPhase::Speculative);
    assert!(t.supervisor.public.owns(AudioOwner::Speculative(31)), "single owner: the key-down capture");
    assert_eq!(t.supervisor.public.flowing_speculative.load(Ordering::SeqCst), 31, "PCM from the first instant");
    assert_eq!(t.events_for(preview), vec![AudioLifecycleEvent::Accepted, AudioLifecycleEvent::Ready, AudioLifecycleEvent::Idle], "the test yields; Settings sees Idle");
    assert!(!t.active.lock().unwrap()[0].load(Ordering::SeqCst), "unconfirmed capture publishes no level meter");

    // The user speaks the first word before the gesture is confirmed.
    t.hear(DICTATION_PCM);
    t.resume();
    wait_until("key-down capture retains PCM", || !t.retained(0).is_empty());
    let dictation = AudioOwner::Dictation(41);
    assert_eq!(start(&t.supervisor, dictation, Some(31), None).recv().unwrap(), Ok(()));
    assert!(t.supervisor.public.is_recording_for(dictation), "no connecting phase");
    assert_eq!(t.spawns.load(Ordering::SeqCst), 1, "the device was never closed and reopened");

    let samples = stop(&t, dictation);
    assert!(!samples.is_empty());
    assert!(samples.iter().all(|&s| s == DICTATION_PCM), "only audio from key-down on; no preview audio");
    wait_until("worker joined with its dictation", || t.supervisor.public.phase() == PublicPhase::Idle);
    assert_eq!(t.events_for(preview).iter().filter(|e| **e == AudioLifecycleEvent::Idle).count(), 1);
    shutdown(&t.supervisor);
}

#[test]
fn confirmed_start_without_a_key_down_capture_adopts_a_flowing_mic_test() {
    let t = handoff(None);
    let preview = AudioOwner::Preview(6);
    flowing_preview(&t, preview, Some("usb-mic"));
    t.pause();
    let dictation = AudioOwner::Dictation(42);
    assert_eq!(start(&t.supervisor, dictation, None, Some("usb-mic")).recv().unwrap(), Ok(()));
    t.hear(DICTATION_PCM);
    t.resume();
    assert!(t.supervisor.public.is_recording_for(dictation), "ready at the confirmed start");
    assert_eq!(t.spawns.load(Ordering::SeqCst), 1);
    assert!(t.active.lock().unwrap()[0].load(Ordering::SeqCst), "dictation levels flow at once");
    let preview_idle = t.position((preview, AudioLifecycleEvent::Idle)).expect("the test yielded");
    let ready = t.position((dictation, AudioLifecycleEvent::Ready)).expect("dictation ready");
    assert!(preview_idle < ready, "the test yields before the dictation reports ready");

    wait_until("dictation retains PCM", || !t.retained(0).is_empty());
    let samples = stop(&t, dictation);
    assert!(!samples.is_empty() && samples.iter().all(|&s| s == DICTATION_PCM));
    shutdown(&t.supervisor);
}

#[test]
fn unconfirmed_takeover_is_discarded_with_its_samples_and_the_mic_test_can_reopen() {
    let t = handoff(None);
    let preview = AudioOwner::Preview(7);
    flowing_preview(&t, preview, None);
    assert!(key_down(&t, 32, None, Some(7)));
    t.hear(DICTATION_PCM);
    wait_until("key-down capture retains PCM", || !t.retained(0).is_empty());

    // A plain Fn tap: never confirmed.
    t.supervisor.sender.send(SupervisorMessage::DiscardSpeculative { id: 32, reason: speculative::DiscardReason::NotConfirmed }).unwrap();
    wait_until("discarded worker joined", || t.supervisor.public.phase() == PublicPhase::Idle);
    assert!(t.retained(0).is_empty(), "unconfirmed audio is dropped");
    assert!(t.sink.events.lock().unwrap().iter().all(|(owner, _)| owner.dictation_id().is_none()));

    // Settings reopens its test, as it does after a dictation.
    let next = AudioOwner::Preview(8);
    assert_eq!(start(&t.supervisor, next, None, None).recv().unwrap(), Ok(()));
    wait_until("mic test back", || t.supervisor.public.is_recording_for(next));
    assert_eq!(t.spawns.load(Ordering::SeqCst), 2);
    shutdown(&t.supervisor);
}

#[test]
fn a_mic_test_on_another_input_is_never_taken_over() {
    let t = handoff(None);
    let preview = AudioOwner::Preview(9);
    flowing_preview(&t, preview, Some("built-in"));

    // Key-down for another input keeps the old refusal; the test keeps running.
    assert!(!key_down(&t, 33, Some("usb-mic"), Some(9)));
    assert!(t.supervisor.public.is_recording_for(preview));
    assert!(!t.events_for(preview).contains(&AudioLifecycleEvent::Idle));

    // The confirmed start yields the test and opens its own input cold, after
    // the test's worker has exited (never two owners).
    let dictation = AudioOwner::Dictation(43);
    let started = start(&t.supervisor, dictation, None, Some("usb-mic"));
    assert_eq!(started.recv_timeout(Duration::from_secs(1)).unwrap(), Ok(()));
    wait_until("cold dictation ready", || t.supervisor.public.is_recording_for(dictation));
    assert_eq!(t.spawns.load(Ordering::SeqCst), 2);
    let preview_idle = t.position((preview, AudioLifecycleEvent::Idle)).expect("the test yielded");
    let accepted = t.position((dictation, AudioLifecycleEvent::Accepted)).unwrap();
    assert!(preview_idle < accepted, "the cold start waits for the test's confirmed exit");
    assert!(t.retained(0).is_empty(), "the yielded test kept nothing");
    let samples = stop(&t, dictation);
    assert!(!samples.is_empty() && samples.iter().all(|&s| s == COLD_PCM));
    shutdown(&t.supervisor);
}

#[test]
fn a_mic_test_still_connecting_is_not_taken_over() {
    let first_buffer = Gate::closed();
    let t = handoff(Some(first_buffer.clone()));
    let preview = AudioOwner::Preview(10);
    assert_eq!(start(&t.supervisor, preview, None, None).recv().unwrap(), Ok(()));
    assert_eq!(t.supervisor.public.phase(), PublicPhase::Starting);
    assert!(!key_down(&t, 34, None, Some(10)), "nothing is flowing to hand over");
    assert!(t.supervisor.public.owns(preview));
    first_buffer.open();
    shutdown(&t.supervisor);
}

#[test]
fn a_mic_test_failing_at_runtime_never_fails_the_dictation_behind_it() {
    // The test's device failed, and its worker is still tearing down when a
    // confirmed start (no key-down capture) arrives. The start must wait for
    // that exit and open cold, never report the test's recovery as its own.
    let (failed, teardown) = (Arc::new(AtomicBool::new(false)), Gate::closed());
    let t = handoff_with(None, Some((failed.clone(), teardown.clone())));
    let preview = AudioOwner::Preview(13);
    flowing_preview(&t, preview, None);
    failed.store(true, Ordering::SeqCst);
    wait_until("the test is recovering", || !t.supervisor.public.is_recording_for(preview));

    let dictation = AudioOwner::Dictation(44);
    let started = start(&t.supervisor, dictation, None, None);
    std::thread::sleep(Duration::from_millis(20));
    assert!(started.try_recv().is_err(), "queued behind the failed test's exit");
    teardown.open();
    assert_eq!(started.recv_timeout(Duration::from_secs(2)).unwrap(), Ok(()));
    wait_until("cold dictation ready", || t.supervisor.public.is_recording_for(dictation));
    assert_eq!(t.spawns.load(Ordering::SeqCst), 2, "opened its own device");
    let samples = stop(&t, dictation);
    assert!(!samples.is_empty() && samples.iter().all(|&s| s == COLD_PCM));
    shutdown(&t.supervisor);
}

#[test]
fn a_dictation_that_took_over_the_mic_test_is_cancelled_by_a_topology_change() {
    // Like an adopted key-down capture, the taken-over device is warm: it was
    // opened before the dictation's own start, so a newer topology revokes it.
    let t = handoff(None);
    flowing_preview(&t, AudioOwner::Preview(14), None);
    let dictation = AudioOwner::Dictation(45);
    assert_eq!(start(&t.supervisor, dictation, None, None).recv().unwrap(), Ok(()));
    wait_until("dictation ready", || t.supervisor.public.is_recording_for(dictation));
    crate::audio_inventory::topology_changed();
    t.supervisor.sender.send(SupervisorMessage::InputTopologyChanged).unwrap();
    wait_until("warm dictation revoked", || !t.supervisor.public.is_recording_for(dictation));
    assert_eq!(t.spawns.load(Ordering::SeqCst), 1);
    shutdown(&t.supervisor);
}

#[test]
fn a_mic_test_opened_before_a_topology_change_is_never_taken_over() {
    let t = handoff(None);
    let preview = AudioOwner::Preview(15);
    flowing_preview(&t, preview, None);
    crate::audio_inventory::topology_changed();
    t.supervisor.sender.send(SupervisorMessage::InputTopologyChanged).unwrap();
    assert!(!key_down(&t, 37, None, Some(15)), "the open device may be the old input");
    let dictation = AudioOwner::Dictation(46);
    let started = start(&t.supervisor, dictation, None, None);
    assert_eq!(started.recv_timeout(Duration::from_secs(1)).unwrap(), Ok(()));
    wait_until("cold dictation ready", || t.supervisor.public.is_recording_for(dictation));
    assert_eq!(t.spawns.load(Ordering::SeqCst), 2, "reopened under the new topology");
    assert!(t.position((preview, AudioLifecycleEvent::Idle)).is_some());
    let samples = stop(&t, dictation);
    assert!(!samples.is_empty() && samples.iter().all(|&s| s == COLD_PCM));
    shutdown(&t.supervisor);
}

#[test]
fn key_down_without_a_handoff_request_still_refuses_a_running_mic_test() {
    let t = handoff(None);
    let preview = AudioOwner::Preview(11);
    flowing_preview(&t, preview, None);
    assert!(!key_down(&t, 35, None, None));
    assert!(!key_down(&t, 36, None, Some(99)), "only the exact test generation is taken over");
    assert!(t.supervisor.public.is_recording_for(preview));
    assert_eq!(t.spawns.load(Ordering::SeqCst), 1);
    shutdown(&t.supervisor);
}

#[test]
fn handed_off_preview_worker_retains_only_post_handoff_audio() {
    let shared = Mutex::new(audio::CaptureSamples::default());
    let preview = AudioOwner::Preview(12);
    audio::retain_pcm(&shared, preview, &[PREVIEW_PCM; 8], 16_000);
    assert!(shared.lock().unwrap().is_empty());
    assert!(!audio::capture_handed_off(&shared));
    shared.lock().unwrap().begin_handoff_retention();
    assert!(audio::capture_handed_off(&shared));
    audio::retain_pcm(&shared, preview, &[DICTATION_PCM; 8], 16_000);
    assert_eq!(shared.lock().unwrap().to_vec(), vec![DICTATION_PCM; 8]);
    // A discarded takeover still drops everything, including later PCM.
    shared.lock().unwrap().revoke_retention();
    audio::retain_pcm(&shared, preview, &[DICTATION_PCM; 8], 16_000);
    assert!(shared.lock().unwrap().is_empty());
    // A benchmark never retains, handed off or not.
    let benchmark = Mutex::new(audio::CaptureSamples::default());
    benchmark.lock().unwrap().begin_handoff_retention();
    audio::retain_pcm(&benchmark, AudioOwner::MicrophoneBenchmark(1), &[0.1; 8], 16_000);
    assert!(benchmark.lock().unwrap().is_empty());
}
