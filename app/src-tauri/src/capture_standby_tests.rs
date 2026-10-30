// Hardware-free exact-owner and privacy tests for optional standby.
use super::*;

struct StandbyFactory {
    teardown: Option<Gate>,
    buffer: Arc<Mutex<Option<Arc<Mutex<audio::CaptureSamples>>>>>,
    spawns: Arc<AtomicUsize>,
}
impl WorkerFactory for StandbyFactory {
    fn spawn(&self, spec: AudioWorkerSpec, events: AudioWorkerEventSender) -> Result<JoinHandle<()>, String> {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        *self.buffer.lock().unwrap() = Some(spec.shared.clone());
        let teardown = self.teardown.clone();
        Ok(std::thread::spawn(move || {
            spec.shared.lock().unwrap().append_pcm(&[0.25; 160], 16_000);
            events.send(AudioWorkerEvent::FirstBuffer { owner: spec.owner, sample_rate: 16_000 }).unwrap();
            spec.command_receiver.recv().unwrap();
            if let Some(gate) = teardown { gate.wait(); }
            events.send(AudioWorkerEvent::ThreadExited { owner: spec.owner }).unwrap();
        }))
    }
}

#[test]
fn standby_reuses_exact_worker_and_rejects_stale_logical_owner() {
    let buffer = Arc::new(Mutex::new(None));
    let spawns = Arc::new(AtomicUsize::new(0));
    let sink = Arc::new(RecordingSink::default());
    let supervisor = spawn_supervisor(Arc::new(StandbyFactory { teardown: None, buffer: buffer.clone(), spawns: spawns.clone() }), sink.clone(), SupervisorConfig::default());
    supervisor.sender.send(SupervisorMessage::ConfigureStandby(StandbyPolicy { seconds: 300, pre_roll_ms: 450 })).unwrap();
    assert_eq!(start(&supervisor, AudioOwner::Dictation(1)).recv().unwrap(), Ok(()));
    wait_until("cold ready", || supervisor.public.phase() == PublicPhase::Recording);
    let (tx, rx) = mpsc::channel();
    supervisor.sender.send(SupervisorMessage::Stop { owner: Some(AudioOwner::Dictation(1)), response: tx }).unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap().unwrap(), vec![0.25; 160]);
    assert_eq!(supervisor.public.phase(), PublicPhase::Standby);
    let shared = buffer.lock().unwrap().clone().unwrap();
    shared.lock().unwrap().append_pcm(&[0.5; 16_000], 16_000);
    assert_eq!(shared.lock().unwrap().len(), 7_200);
    assert_eq!(start(&supervisor, AudioOwner::Dictation(2)).recv().unwrap(), Ok(()));
    assert_eq!(spawns.load(Ordering::SeqCst), 1);
    assert_eq!(shared.lock().unwrap().as_slice(), &[0.5; 7_200]);
    assert_eq!(cancel_any_phase(&supervisor, AudioOwner::Dictation(1)).recv().unwrap(), Ok(false));
    let (tx, rx) = mpsc::channel();
    supervisor.sender.send(SupervisorMessage::Stop { owner: Some(AudioOwner::Dictation(1)), response: tx }).unwrap();
    assert!(rx.recv().unwrap().is_err());
    assert!(supervisor.public.is_recording_for(AudioOwner::Dictation(2)));
    assert_eq!(cancel_any_phase(&supervisor, AudioOwner::Dictation(2)).recv().unwrap(), Ok(true));
    wait_until("worker joined", || supervisor.public.phase() == PublicPhase::Idle);
    assert!(!sink.events.lock().unwrap().iter().any(|(owner, event)| *owner == AudioOwner::Dictation(1) && matches!(event, AudioLifecycleEvent::Recovering { .. })));
    shutdown(&supervisor);
}

#[test]
fn standby_expiry_device_change_sleep_and_disable_confirm_worker_exit() {
    for teardown in ["expiry", "device", "sleep", "disable", "setting", "other_owner", "other_device"] {
        let (supervisor, gate, spawns, _, _) = harness(AudioInitPhase::StreamBuild, SupervisorConfig::default());
        supervisor.sender.send(SupervisorMessage::ConfigureStandby(StandbyPolicy { seconds: 300, pre_roll_ms: 450 })).unwrap();
        start(&supervisor, AudioOwner::Dictation(1)).recv().unwrap().unwrap();
        gate.open();
        wait_until("ready", || supervisor.public.phase() == PublicPhase::Recording);
        let (tx, rx) = mpsc::channel();
        supervisor.sender.send(SupervisorMessage::Stop { owner: Some(AudioOwner::Dictation(1)), response: tx }).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap().unwrap(), vec![0.25]);
        match teardown {
            "expiry" => check_deadlines(&supervisor, Duration::from_secs(301)),
            "device" | "sleep" => {
                let (tx, rx) = mpsc::channel();
                supervisor.sender.send(SupervisorMessage::Cancel { owner: None, reason: if teardown == "device" { AudioCancelReason::DeviceChanged } else { AudioCancelReason::SystemSleep }, starting_only: true, response: tx }).unwrap();
                assert_eq!(rx.recv().unwrap(), Ok(true));
            }
            "setting" => { supervisor.sender.send(SupervisorMessage::ConfigureStandby(StandbyPolicy::default())).unwrap(); }
            "other_owner" | "other_device" => {
                let owner = if teardown == "other_owner" { AudioOwner::Transform(2) } else { AudioOwner::Dictation(2) };
                let device = (teardown == "other_device").then(|| "different".to_string());
                start_with_device(&supervisor, owner, device).recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
                wait_until("replacement ready", || supervisor.public.phase() == PublicPhase::Recording);
                assert_eq!(spawns.load(Ordering::SeqCst), 2);
                assert_eq!(cancel_any_phase(&supervisor, owner).recv().unwrap(), Ok(true));
            }
            _ => {
                let (tx, rx) = mpsc::channel();
                supervisor.sender.send(SupervisorMessage::RetireStandby(tx)).unwrap();
                rx.recv_timeout(Duration::from_secs(1)).unwrap();
                assert_eq!(supervisor.public.phase(), PublicPhase::Idle);
            }
        }
        wait_until("standby joined", || supervisor.public.phase() == PublicPhase::Idle);
        shutdown(&supervisor);
    }
}

#[test]
fn disabled_and_expired_standby_preserve_cold_pcm_byte_for_byte() {
    let input = [0.0_f32, -0.0, 0.1, -0.8, 1.0];
    for expired in [false, true] {
        let mut buffer = audio::CaptureSamples::default();
        if expired {
            buffer.enter_standby(450);
            buffer.append_pcm(&[0.8; 16_000], 16_000);
            // Expiry drops the whole capture attempt; the next factory spawn
            // receives a fresh buffer, exactly as with standby disabled.
            buffer = audio::CaptureSamples::default();
        }
        for chunk in input.chunks(2) { buffer.append_pcm(chunk, 16_000); }
        let actual = samples_for_asr(buffer.take(), 16_000);
        assert_eq!(actual.iter().map(|x| x.to_bits()).collect::<Vec<_>>(), input.iter().map(|x| x.to_bits()).collect::<Vec<_>>());
    }
}

#[test]
fn cancellation_during_standby_handoff_never_launches_pending_owner() {
    let gate = Gate::closed();
    let sink = Arc::new(RecordingSink::default());
    let supervisor = spawn_supervisor(Arc::new(BlockingTeardownFactory { gate: gate.clone() }), sink.clone(), SupervisorConfig::default());
    supervisor.sender.send(SupervisorMessage::ConfigureStandby(StandbyPolicy { seconds: 300, pre_roll_ms: 450 })).unwrap();
    start(&supervisor, AudioOwner::Dictation(1)).recv().unwrap().unwrap();
    wait_until("ready", || supervisor.public.phase() == PublicPhase::Recording);
    let (tx, rx) = mpsc::channel();
    supervisor.sender.send(SupervisorMessage::Stop { owner: Some(AudioOwner::Dictation(1)), response: tx }).unwrap();
    rx.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
    let pending = start(&supervisor, AudioOwner::Transform(2));
    wait_until("old worker terminating", || supervisor.public.phase() == PublicPhase::Recovering);
    assert!(matches!(pending.try_recv(), Err(mpsc::TryRecvError::Empty)));
    assert_eq!(cancel_any_phase(&supervisor, AudioOwner::Transform(2)).recv().unwrap(), Ok(true));
    assert_eq!(pending.recv().unwrap(), Err(AudioStartError::Cancelled));
    gate.open();
    wait_until("old worker joined", || supervisor.public.phase() == PublicPhase::Idle);
    assert!(!sink.events.lock().unwrap().iter().any(|(owner, event)| *owner == AudioOwner::Transform(2) && *event == AudioLifecycleEvent::Accepted));
    shutdown(&supervisor);
}

#[test]
fn standby_ring_is_bounded_at_both_rates_and_preserves_latest_pcm_order() {
    for rate in [16_000, 48_000] {
        for pre_roll in [300, 450] {
            let mut samples = audio::CaptureSamples::default();
            samples.append_pcm(&[0.9; 100], rate);
            assert_eq!(samples.enter_standby(pre_roll), vec![0.9; 100]);
            let input: Vec<f32> = (0..rate).map(|i| i as f32).collect();
            for chunk in input.chunks(512) { samples.append_pcm(chunk, rate); }
            let limit = (rate * pre_roll / 1_000) as usize;
            assert_eq!(samples.as_slice(), &input[input.len() - limit..]);
            samples.resume();
            samples.append_pcm(&[0.8; 1_000], rate);
            assert_eq!(samples.len(), limit + 1_000);
            assert_eq!(&samples[..limit], &input[input.len() - limit..]);
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn native_default_input_topology_notification_prevents_old_stream_reuse() {
    let buffer = Arc::new(Mutex::new(None));
    let spawns = Arc::new(AtomicUsize::new(0));
    let supervisor = spawn_supervisor(Arc::new(StandbyFactory { teardown: None, buffer: buffer.clone(), spawns: spawns.clone() }), Arc::new(RecordingSink::default()), SupervisorConfig::default());
    supervisor.sender.send(SupervisorMessage::ConfigureStandby(StandbyPolicy { seconds: 300, pre_roll_ms: 450 })).unwrap();
    // Both requests follow System Default (device_id=None).
    start(&supervisor, AudioOwner::Dictation(1)).recv().unwrap().unwrap();
    wait_until("ready", || supervisor.public.phase() == PublicPhase::Recording);
    let (tx, rx) = mpsc::channel();
    supervisor.sender.send(SupervisorMessage::Stop { owner: Some(AudioOwner::Dictation(1)), response: tx }).unwrap();
    rx.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
    buffer.lock().unwrap().as_ref().unwrap().lock().unwrap().append_pcm(&[0.5; 160], 16_000);
    // Use the exact entry point called by native HAL/default-input listeners,
    // rather than directly injecting an audio Cancel message.
    crate::audio_inventory::topology_changed();
    start(&supervisor, AudioOwner::Dictation(2)).recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
    wait_until("replacement ready", || supervisor.public.phase() == PublicPhase::Recording);
    assert_eq!(spawns.load(Ordering::SeqCst), 2);
    assert_eq!(buffer.lock().unwrap().as_ref().unwrap().lock().unwrap().as_slice(), &[0.25; 160]);
    assert_eq!(cancel_any_phase(&supervisor, AudioOwner::Dictation(2)).recv().unwrap(), Ok(true));
    wait_until("joined", || supervisor.public.phase() == PublicPhase::Idle);
    shutdown(&supervisor);
}

#[test]
fn timed_out_handoff_never_spawns_after_blocked_teardown_completes() {
    let gate = Gate::closed();
    let spawns = Arc::new(AtomicUsize::new(0));
    let sink = Arc::new(RecordingSink::default());
    let supervisor = spawn_supervisor(Arc::new(StandbyFactory { teardown: Some(gate.clone()), buffer: Arc::new(Mutex::new(None)), spawns: spawns.clone() }), sink.clone(), SupervisorConfig::default());
    supervisor.sender.send(SupervisorMessage::ConfigureStandby(StandbyPolicy { seconds: 300, pre_roll_ms: 450 })).unwrap();
    start(&supervisor, AudioOwner::Dictation(1)).recv().unwrap().unwrap();
    wait_until("ready", || supervisor.public.phase() == PublicPhase::Recording);
    let (tx, rx) = mpsc::channel();
    supervisor.sender.send(SupervisorMessage::Stop { owner: Some(AudioOwner::Dictation(1)), response: tx }).unwrap();
    rx.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
    let lease = Arc::new(AtomicBool::new(true));
    let (response, result) = mpsc::channel();
    supervisor.sender.send(SupervisorMessage::Start(StartRequest {
        caller_alive: lease.clone(), owner: AudioOwner::Transform(2),
        app_handle: None, device_id: None, origin: "transform".into(),
        wait_until_ready: false, response, probe_permit: None,
    })).unwrap();
    let began = Instant::now();
    assert_eq!(await_start_response(&supervisor, AudioOwner::Transform(2), lease.clone(), result, SUPERVISOR_RESPONSE_TIMEOUT), Err(AudioStartError::SupervisorUnavailable));
    assert!(began.elapsed() >= SUPERVISOR_RESPONSE_TIMEOUT);
    assert!(!lease.load(Ordering::Acquire));
    assert_eq!(supervisor.public.phase(), PublicPhase::Recovering);
    gate.open();
    wait_until("old worker joined", || supervisor.public.phase() == PublicPhase::Idle);
    assert_eq!(spawns.load(Ordering::SeqCst), 1);
    assert!(!sink.events.lock().unwrap().iter().any(|(owner, event)| *owner == AudioOwner::Transform(2) && *event == AudioLifecycleEvent::Accepted));
    shutdown(&supervisor);
}

#[test]
fn standby_termination_failure_cannot_deliver_idle_audio_as_interrupted_dictation() {
    let buffer = Arc::new(Mutex::new(None));
    let sink = Arc::new(RecordingSink::default());
    let owner = AudioOwner::Dictation(887);
    let supervisor = spawn_supervisor(Arc::new(StandbyFactory { teardown: None, buffer: buffer.clone(), spawns: Arc::new(AtomicUsize::new(0)) }), sink.clone(), SupervisorConfig::default());
    supervisor.sender.send(SupervisorMessage::ConfigureStandby(StandbyPolicy { seconds: 300, pre_roll_ms: 450 })).unwrap();
    start(&supervisor, owner).recv().unwrap().unwrap();
    wait_until("ready", || supervisor.public.phase() == PublicPhase::Recording);
    let (tx, rx) = mpsc::channel();
    supervisor.sender.send(SupervisorMessage::Stop { owner: Some(owner), response: tx }).unwrap();
    rx.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
    buffer.lock().unwrap().as_ref().unwrap().lock().unwrap().append_pcm(&[0.5; 160], 16_000);
    supervisor.sender.send(SupervisorMessage::Worker(AudioWorkerEvent::TerminationUnconfirmed {
        owner, failure: AudioFailure::new(AudioFailureKind::TerminationUnconfirmed, AudioInitPhase::Runtime),
    })).unwrap();
    wait_until("joined", || supervisor.public.phase() == PublicPhase::Idle);
    assert!(take_interrupted_dictation(887).is_none());
    assert!(!sink.events.lock().unwrap().iter().any(|(_, event)| matches!(event, AudioLifecycleEvent::Interrupted { .. })));
    shutdown(&supervisor);
}

#[cfg(target_os = "macos")]
#[test]
fn topology_change_after_warm_stop_begins_rejects_invalidated_pcm() {
    let gate = Gate::closed();
    let buffer = Arc::new(Mutex::new(None));
    let sink = Arc::new(RecordingSink::default());
    let supervisor = spawn_supervisor(Arc::new(StandbyFactory { teardown: Some(gate.clone()), buffer: buffer.clone(), spawns: Arc::new(AtomicUsize::new(0)) }), sink.clone(), SupervisorConfig::default());
    supervisor.sender.send(SupervisorMessage::ConfigureStandby(StandbyPolicy { seconds: 300, pre_roll_ms: 450 })).unwrap();
    start(&supervisor, AudioOwner::Dictation(1)).recv().unwrap().unwrap();
    wait_until("ready", || supervisor.public.phase() == PublicPhase::Recording);
    let (tx, rx) = mpsc::channel();
    supervisor.sender.send(SupervisorMessage::Stop { owner: Some(AudioOwner::Dictation(1)), response: tx }).unwrap();
    rx.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
    buffer.lock().unwrap().as_ref().unwrap().lock().unwrap().append_pcm(&[0.5; 160], 16_000);
    start(&supervisor, AudioOwner::Dictation(2)).recv().unwrap().unwrap();
    let (tx, rx) = mpsc::channel();
    supervisor.sender.send(SupervisorMessage::Stop { owner: Some(AudioOwner::Dictation(2)), response: tx }).unwrap();
    wait_until("warm stop entered Stopping", || supervisor.public.phase() == PublicPhase::Stopping);
    // Stop is already ahead of the topology event. Its tail must not return
    // invalidated pre-roll successfully, even though begin_recovery is a no-op.
    crate::audio_inventory::topology_changed();
    supervisor.sender.send(SupervisorMessage::InputTopologyChanged).unwrap();
    check_deadlines(&supervisor, Duration::from_millis(100));
    assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
    gate.open();
    assert!(rx.recv_timeout(Duration::from_secs(1)).unwrap().is_err());
    wait_until("joined", || supervisor.public.phase() == PublicPhase::Idle);
    assert!(!sink.events.lock().unwrap().iter().any(|(_, event)| matches!(event, AudioLifecycleEvent::Interrupted { .. })));
    shutdown(&supervisor);
}

#[cfg(target_os = "macos")]
#[test]
fn warm_pcm_conversion_rechecks_topology_after_resampling_for_every_delivery() {
    let raw: Vec<f32> = (0..4_800).map(|i| (i as f32 * 0.1).sin() * 0.1).collect();
    // This exact conversion+guard is shared by normal Stop-to-standby,
    // held-teardown Stop, and interrupted-capture storage/event delivery.
    let epoch = INPUT_TOPOLOGY_GENERATION.load(Ordering::Acquire);
    let result = convert_for_delivery(raw.clone(), 48_000, Some(epoch), |raw, rate| {
        let samples = samples_for_asr(raw, rate);
        crate::audio_inventory::topology_changed();
        samples
    });
    assert!(result.is_err(), "warm PCM invalidated during conversion must never reach a delivery path");
    // Cold behavior remains byte-for-byte identical, including when a native
    // topology notification occurs during the conversion.
    let expected = samples_for_asr(raw.clone(), 48_000);
    let cold = convert_for_delivery(raw, 48_000, None, |raw, rate| {
        let samples = samples_for_asr(raw, rate);
        crate::audio_inventory::topology_changed();
        samples
    }).unwrap();
    assert_eq!(cold.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
}
