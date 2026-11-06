//! Port of keyboard routing, Escape, silenceAutoStop, smartTurn and their hook
//! lifecycle tests. Fake time/host: no runtime, OS listener or webview is created.
use murmur_core::dictation::*;
fn control() -> Controller {
    let mut c = Controller::default();
    c.gate = LaunchGate {
        configured: true,
        onboarding_complete: true,
        model_installed: true,
        accessibility: true,
        disabled: false,
    };
    c.silence_ms = 1500;
    c
}
fn recording(c: &mut Controller, generation: u64, origin: Origin) {
    c.observe(generation, Status::Starting, origin);
    c.observe(generation, Status::Recording, origin);
}
fn level(c: &mut Controller, generation: u64, value: f64, time: u64) -> Option<Effect> {
    c.input(Input::Level {
        generation,
        level: value,
        at_ms: time,
    })
}
fn speech(c: &mut Controller, generation: u64, offset: u64) {
    for time in (0..=500).step_by(100) {
        assert_eq!(level(c, generation, 0.1, offset + time), None);
    }
}
fn quiet(c: &mut Controller, generation: u64, start: u64, end: u64) -> Vec<Effect> {
    (start..=end)
        .step_by(100)
        .filter_map(|time| level(c, generation, 0.001, time))
        .collect()
}
#[test]
fn launch_requires_each_gate_but_disabled_still_allows_escape() {
    for index in 0..5 {
        let mut c = control();
        match index {
            0 => c.gate.configured = false,
            1 => c.gate.onboarding_complete = false,
            2 => c.gate.model_installed = false,
            3 => c.gate.accessibility = false,
            _ => c.gate.disabled = true,
        }
        assert_eq!(c.input(Input::Toggle(None)), None);
        assert_eq!(c.input(Input::HoldStart(None)), None);
    }
    let mut c = control();
    recording(&mut c, 1, Origin::Hold);
    c.gate.disabled = true;
    assert_eq!(
        c.input(Input::Escape),
        Some(Effect::Cancel { generation: 1 })
    );
}
#[test]
fn gesture_tokens_are_forwarded_without_reuse_in_all_modes() {
    for hold in [true, false] {
        let mut c = control();
        for token in [Some("9007199254740993".into()), None] {
            let input = if hold {
                Input::HoldStart(token.clone())
            } else {
                Input::Toggle(token.clone())
            };
            assert_eq!(
                c.input(input),
                Some(Effect::Start {
                    origin: if hold { Origin::Hold } else { Origin::Toggle },
                    gesture_token: token
                })
            );
        }
    }
}
#[test]
fn escape_idle_processing_starting_and_duplicate_cancellation() {
    let mut c = control();
    assert_eq!(c.input(Input::Escape), None);
    for (generation, status) in [
        (1, Status::Starting),
        (2, Status::Recording),
        (3, Status::Processing),
    ] {
        c.observe(generation, status, Origin::Toggle);
        assert_eq!(c.input(Input::Escape), Some(Effect::Cancel { generation }));
        assert_eq!(c.input(Input::Escape), None);
    }
}
#[test]
fn release_before_readiness_cancels_exact_start_and_stale_levels_cannot_stop_new_owner() {
    let mut c = control();
    c.observe(5, Status::Starting, Origin::Hold);
    assert_eq!(
        c.input(Input::HoldStop),
        Some(Effect::Cancel { generation: 5 })
    );
    recording(&mut c, 6, Origin::Toggle);
    speech(&mut c, 6, 0);
    assert!(quiet(&mut c, 5, 600, 3000).is_empty());
    assert_eq!(
        quiet(&mut c, 6, 600, 2000),
        vec![Effect::Stop {
            generation: 6,
            auto: true,
            hold: false
        }]
    );
}
#[test]
fn silence_once_after_speech_and_fresh_per_recording() {
    let mut c = control();
    recording(&mut c, 1, Origin::Toggle);
    assert!(quiet(&mut c, 1, 0, 5000).is_empty());
    speech(&mut c, 1, 5100);
    assert_eq!(
        quiet(&mut c, 1, 5700, 8000),
        vec![Effect::Stop {
            generation: 1,
            auto: true,
            hold: false
        }]
    );
    c.observe(1, Status::Idle, Origin::Toggle);
    recording(&mut c, 2, Origin::Toggle);
    assert!(quiet(&mut c, 2, 8100, 12000).is_empty());
}
#[test]
fn disabled_off_and_nonrecording_levels_are_ignored() {
    for variant in 0..4 {
        let mut c = control();
        recording(&mut c, 1, Origin::Toggle);
        match variant {
            0 => c.gate.onboarding_complete = false,
            1 => c.silence_ms = 0,
            2 => c.observe(1, Status::Starting, Origin::Toggle),
            _ => c.observe(1, Status::Processing, Origin::Toggle),
        }
        speech(&mut c, 1, 0);
        assert!(quiet(&mut c, 1, 600, 5000).is_empty());
    }
}
#[test]
fn hold_exclusion_hold_over_combo_and_cancelled_origin_healing() {
    let mut c = control();
    recording(&mut c, 1, Origin::Hold);
    speech(&mut c, 1, 0);
    assert!(quiet(&mut c, 1, 600, 4000).is_empty());
    c.observe(1, Status::Idle, Origin::Toggle);
    recording(&mut c, 2, Origin::Toggle);
    speech(&mut c, 2, 5000);
    c.input(Input::HoldOverStart);
    assert!(quiet(&mut c, 2, 5600, 9000).is_empty());
    c.input(Input::HoldOverEnd);
    assert_eq!(
        quiet(&mut c, 2, 9100, 11000),
        vec![Effect::Stop {
            generation: 2,
            auto: true,
            hold: false
        }]
    );
    c.observe(3, Status::Starting, Origin::Hold);
    c.input(Input::HoldCancel);
    c.observe(3, Status::Idle, Origin::Toggle);
    recording(&mut c, 4, Origin::Toggle);
    speech(&mut c, 4, 12000);
    assert_eq!(quiet(&mut c, 4, 12600, 14000).len(), 1);
}
#[test]
fn locked_mode_is_hands_free_and_clears_on_cancel() {
    let mut c = control();
    assert!(matches!(c.input(Input::Lock), Some(Effect::Start { .. })));
    recording(&mut c, 1, Origin::Toggle);
    assert!(c.locked);
    speech(&mut c, 1, 0);
    assert_eq!(quiet(&mut c, 1, 600, 2000).len(), 1);
    assert!(!c.locked);
    recording(&mut c, 2, Origin::Toggle);
    c.input(Input::Lock);
    c.input(Input::Escape);
    assert!(!c.locked);
}
#[test]
fn smart_threshold_once_per_pause_and_fallback() {
    for probability in [None, Some(0.69), Some(f64::NAN), Some(0.7), Some(1.0)] {
        let mut c = control();
        c.smart_enabled = true;
        recording(&mut c, 1, Origin::Toggle);
        speech(&mut c, 1, 0);
        assert_eq!(
            quiet(&mut c, 1, 600, 800),
            vec![Effect::Score {
                generation: 1,
                pause: 1
            }]
        );
        let answer = c.input(Input::SmartAnswer {
            generation: 1,
            pause: 1,
            probability,
        });
        if probability.is_some_and(|p| p >= 0.7) {
            assert_eq!(
                answer,
                Some(Effect::Stop {
                    generation: 1,
                    auto: true,
                    hold: false
                })
            );
        } else {
            assert_eq!(answer, None);
            assert_eq!(
                quiet(&mut c, 1, 900, 2000),
                vec![Effect::Stop {
                    generation: 1,
                    auto: true,
                    hold: false
                }]
            );
        }
    }
}
#[test]
fn smart_rejects_resumed_speech_new_pause_new_generation_and_disabled_answer() {
    for variant in 0..5 {
        let mut c = control();
        c.smart_enabled = true;
        recording(&mut c, 1, Origin::Toggle);
        speech(&mut c, 1, 0);
        quiet(&mut c, 1, 600, 800);
        match variant {
            0 => {
                level(&mut c, 1, 0.1, 900);
            }
            1 => {
                level(&mut c, 1, 0.1, 900);
                quiet(&mut c, 1, 1000, 1300);
            }
            2 => recording(&mut c, 2, Origin::Toggle),
            3 => c.smart_enabled = false,
            _ => c.silence_ms = 0,
        }
        assert_eq!(
            c.input(Input::SmartAnswer {
                generation: 1,
                pause: 1,
                probability: Some(0.9)
            }),
            None
        );
    }
}
#[test]
fn smart_asks_again_after_low_answer_and_next_pause() {
    let mut c = control();
    c.smart_enabled = true;
    recording(&mut c, 1, Origin::Toggle);
    speech(&mut c, 1, 0);
    quiet(&mut c, 1, 600, 800);
    c.input(Input::SmartAnswer {
        generation: 1,
        pause: 1,
        probability: None,
    });
    assert!(quiet(&mut c, 1, 900, 1000).is_empty());
    level(&mut c, 1, 0.1, 1100);
    assert_eq!(
        quiet(&mut c, 1, 1200, 1400),
        vec![Effect::Score {
            generation: 1,
            pause: 2
        }]
    );
}
#[test]
fn detector_threshold_cumulative_speech_resumption_gaps_backwards_and_nonfinite() {
    let mut s = Silence::default();
    assert_eq!(s.threshold(), 0.015);
    s.sample(1.0, 0, 1500);
    assert_eq!(s.threshold(), 0.08);
    s.sample(0.1, 200, 1500);
    s.sample(0.0, 300, 1500);
    assert!(!s.armed);
    s.sample(0.1, 500, 1500);
    assert!(s.armed);
    s.sample(0.01, 700, 1500);
    assert_eq!(s.silence_ms, 200);
    s.sample(0.1, 800, 1500);
    assert_eq!(s.silence_ms, 0);
    s.sample(f64::NAN, 10000, 1500);
    assert_eq!(s.silence_ms, 500);
    s.sample(f64::INFINITY, 9000, 1500);
    assert_eq!(s.silence_ms, 500);
    let mut quiet = Silence::default();
    for t in (0..10000).step_by(100) {
        quiet.sample(0.014, t, 1500);
    }
    assert!(!quiet.armed);
}
#[test]
fn headless_fake_host_can_arm_start_stop_cancel_autostop_and_paste() {
    use murmur_core::completion::{producers::Producers, Command, CompletionService, Scope};
    use murmur_core::events::{Consumer, EventKind, EventSink, Router, WaveformConsumer};
    #[derive(Default)]
    struct FakeSink(std::sync::Mutex<Vec<(Consumer, EventKind)>>);
    impl EventSink for FakeSink {
        fn publish<P: serde::Serialize + Clone>(
            &self,
            event: EventKind,
            _payload: P,
        ) -> Result<(), String> {
            let mut router = Router::default();
            router.set_waveform(WaveformConsumer::Overlay, true);
            let recipients = router.recipients(event, |consumer, _| consumer != Consumer::Main);
            self.0
                .lock()
                .unwrap()
                .extend(recipients.into_iter().map(|consumer| (consumer, event)));
            Ok(())
        }
    }
    struct FakeHost {
        sink: FakeSink,
        producer: Producers,
        generation: u64,
        retained: std::collections::HashMap<u64, bool>,
        settings: serde_json::Value,
    }
    impl FakeHost {
        fn apply(&mut self, controller: &mut Controller, effect: Effect) {
            match effect {
                Effect::Start { origin, .. } => {
                    self.generation += 1;
                    // Same immutable start-time configuration as RecordingContext.
                    self.retained.insert(
                        self.generation,
                        self.settings["retainHistory"].as_bool().unwrap_or(true),
                    );
                    recording(controller, self.generation, origin);
                    self.sink
                        .publish(EventKind::RecordingStatusChanged, "recording")
                        .unwrap();
                    self.sink.publish(EventKind::AudioLevel, 0.25).unwrap();
                }
                Effect::Stop { generation, .. } => {
                    let entry = serde_json::json!({"id":generation.to_string(),"text":"synthetic","duration":1,"timestamp":0});
                    for _attachment in 0..2 {
                        self.producer
                            .dictation(
                                generation,
                                entry.clone(),
                                self.retained[&generation],
                                None,
                                "2026-11-05".into(),
                            )
                            .unwrap();
                    }
                    self.sink.publish(EventKind::HistoryChanged, ()).unwrap();
                    self.sink.publish(EventKind::StatisticsChanged, ()).unwrap();
                    self.sink
                        .publish(EventKind::RecordingStatusChanged, "idle")
                        .unwrap();
                    controller.observe(generation, Status::Idle, Origin::Toggle);
                }
                Effect::Cancel { generation } => {
                    controller.observe(generation, Status::Idle, Origin::Toggle);
                }
                Effect::Score { .. } => {
                    panic!("fake silence flow must not request Smart Turn scoring")
                }
                Effect::PasteLast => {
                    self.producer.paste_last("delivery:1").unwrap();
                }
            }
        }
        fn input(&mut self, controller: &mut Controller, input: Input) {
            if let Some(effect) = controller.input(input) {
                self.apply(controller, effect);
            }
        }
    }
    let root = tempfile::tempdir().unwrap();
    let service = CompletionService::start(root.path().into(), false).unwrap();
    let mut host = FakeHost {
        sink: FakeSink::default(),
        producer: Producers::new(service.clone(), "fake-native-session".into()),
        generation: 0,
        retained: Default::default(),
        settings: serde_json::json!({"retainHistory":true}),
    };
    let mut c = control();
    c.gate.configured = service.execute(Command::Snapshot).unwrap().v1_committed;
    host.input(&mut c, Input::HoldStart(None));
    host.input(&mut c, Input::HoldStop);
    host.input(&mut c, Input::Toggle(None));
    host.input(&mut c, Input::Escape);
    host.input(&mut c, Input::Toggle(None));
    speech(&mut c, host.generation, 0);
    for effect in quiet(&mut c, host.generation, 600, 2000) {
        host.apply(&mut c, effect);
    }
    host.settings["retainHistory"] = serde_json::json!(false);
    host.input(&mut c, Input::Toggle(None));
    host.settings["retainHistory"] = serde_json::json!(true); // cannot change start policy
    host.input(&mut c, Input::Toggle(None));
    host.input(&mut c, Input::PasteLast);
    host.producer.begin_file("file:1").unwrap();
    host.producer.finish_file("file:1", Some(serde_json::json!({"id":"file","text":"synthetic file","duration":2,"timestamp":0,"source":"file"})), true);
    host.producer.accept_file("file:1").unwrap();
    for _ in 0..2 {
        host.producer.meeting(1, 60.0, "2026-11".into()).unwrap();
        host.producer.correction_proposed(1).unwrap();
        host.producer.correction_taught(1, Scope::Global).unwrap();
    }
    let before = service.execute(Command::Snapshot).unwrap();
    assert_eq!(before.history.len(), 3);
    assert_eq!(before.statistics.total_recordings, 3);
    assert_eq!(before.statistics.activity.meetings.count, 1);
    assert_eq!(before.statistics.activity.corrections.proposed, 1);
    assert_eq!(before.statistics.activity.corrections.taught, 1);
    assert_eq!(before.statistics.activity.paste_last_uses, 1);
    for _ in 0..3 {
        assert_eq!(
            service.execute(Command::Snapshot).unwrap().statistics,
            before.statistics
        );
    }
    let deliveries = host.sink.0.lock().unwrap();
    assert!(!deliveries
        .iter()
        .any(|(consumer, _)| *consumer == Consumer::Main));
    for route in [
        (
            Consumer::DictationControl,
            EventKind::RecordingStatusChanged,
        ),
        (Consumer::SoundCues, EventKind::RecordingStatusChanged),
        (Consumer::SmartAuto, EventKind::RecordingStatusChanged),
        (
            Consumer::MeetingSuggestions,
            EventKind::RecordingStatusChanged,
        ),
        (Consumer::Overlay, EventKind::AudioLevel),
        (Consumer::Diagnostics, EventKind::HistoryChanged),
        (Consumer::Diagnostics, EventKind::StatisticsChanged),
    ] {
        assert!(
            deliveries.contains(&route),
            "missing headless route {route:?}"
        );
    }
    drop(deliveries);
    drop(host);
    drop(service);
    let relaunched = CompletionService::start(root.path().into(), false).unwrap();
    let after = relaunched.execute(Command::Snapshot).unwrap();
    assert_eq!(before.history, after.history);
    assert_eq!(before.statistics, after.statistics);
}

#[test]
fn native_completion_sources_use_the_shared_production_adapters() {
    // This is a wiring contract in addition to executing the adapters above:
    // removing a native source call must not silently preserve the exit check.
    for (source, calls) in [
        (
            include_str!("../../../src/commands/recording.rs"),
            &[
                "producer.dictation(",
                "FileOperation::begin(",
                "operation.finish(",
            ][..],
        ),
        (
            include_str!("../../../src/meeting_capture.rs"),
            &["producer.meeting("][..],
        ),
        (
            include_str!("../../../src/commands/correct_and_teach.rs"),
            &[
                "producer.correction_proposed(",
                "producer.correction_taught(",
            ][..],
        ),
        (
            include_str!("../../../src/delivery_recovery.rs"),
            &["producer.paste_last("][..],
        ),
    ] {
        for call in calls {
            assert!(
                source.contains(call),
                "missing native completion source {call}"
            );
        }
    }
}

#[test]
fn delayed_native_inputs_belong_only_to_their_accepted_start() {
    let mut guard = QueueGuard::default();
    let before = std::time::Instant::now();
    let completed = before + std::time::Duration::from_millis(50);
    guard.accepted(0, 1, completed);
    assert_eq!(
        guard.route(0, 1, before, Input::HoldStop),
        Some(Input::HoldCancel)
    );
    assert_eq!(
        guard.route(
            1,
            1,
            completed + std::time::Duration::from_millis(1),
            Input::HoldStop
        ),
        Some(Input::HoldStop)
    );
    assert_eq!(
        guard.route(
            1,
            1,
            before + std::time::Duration::from_millis(25),
            Input::HoldStop
        ),
        Some(Input::HoldCancel)
    );
    for input in [
        Input::HoldStop,
        Input::Escape,
        Input::Lock,
        Input::Toggle(None),
    ] {
        assert_eq!(guard.route(0, 2, before, input), None);
    }
}
#[test]
fn escape_after_stop_can_cancel_processing_and_disabled_does_not_block_autostop() {
    let mut c = control();
    recording(&mut c, 1, Origin::Toggle);
    assert!(matches!(
        c.input(Input::Toggle(None)),
        Some(Effect::Stop { .. })
    ));
    c.observe(1, Status::Processing, Origin::Toggle);
    assert_eq!(
        c.input(Input::Escape),
        Some(Effect::Cancel { generation: 1 })
    );
    c.observe(1, Status::Idle, Origin::Toggle);
    recording(&mut c, 2, Origin::Toggle);
    c.gate.disabled = true;
    speech(&mut c, 2, 0);
    assert_eq!(quiet(&mut c, 2, 600, 2000).len(), 1);
}

#[test]
fn refused_locked_start_clears_lock_without_touching_a_newer_owner() {
    let mut c = control();
    c.input(Input::Lock);
    assert!(c.locked);
    assert!(c.reject_start(0));
    assert!(!c.locked);
    c.input(Input::Lock);
    recording(&mut c, 1, Origin::Toggle);
    assert!(!c.reject_start(0));
    assert!(c.locked);
}

#[test]
fn disabled_lock_cancels_starting_and_stops_locked_recording() {
    for phase in [Status::Starting, Status::Recording] {
        let mut c = control();
        assert!(matches!(c.input(Input::Lock), Some(Effect::Start { .. })));
        c.observe(1, Status::Starting, Origin::Toggle);
        c.observe(1, phase, Origin::Toggle);
        assert!(c.locked);
        c.gate.disabled = true;
        assert_eq!(
            c.input(Input::Lock),
            Some(if phase == Status::Starting {
                Effect::Cancel { generation: 1 }
            } else {
                Effect::Stop {
                    generation: 1,
                    auto: false,
                    hold: false,
                }
            })
        );
        assert!(!c.locked);
        c.observe(1, Status::Idle, Origin::Toggle);
        assert_eq!(c.input(Input::Lock), None);
        assert!(!c.locked);
    }
}

#[test]
fn completion_outcome_table_names_executable_tests_in_plan_order() {
    let note = include_str!("../../../../../docs/draft/gpui-completion-service.md");
    let code = include_str!("../src/completion/tests.rs");
    let table = note
        .split("## Defined outcome coverage")
        .nth(1)
        .unwrap()
        .split("## Test port")
        .next()
        .unwrap();
    let names: Vec<_> = table
        .lines()
        .filter_map(|line| {
            if !line.starts_with("| ") {
                return None;
            }
            let cell = line.split('|').nth(2)?.trim();
            cell.strip_prefix('`')?.strip_suffix('`')
        })
        .collect();
    let executable: Vec<_> = code
        .split("#[test]\nfn ")
        .skip(1)
        .take(37)
        .map(|s| s.split('(').next().unwrap())
        .collect();
    assert_eq!(names.len(), 37);
    assert_eq!(names, executable);
}
