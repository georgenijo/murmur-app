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
            auto: true
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
            auto: true
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
            0 => c.gate.disabled = true,
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
            auto: true
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
    c.input(Input::Lock);
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
                    auto: true
                })
            );
        } else {
            assert_eq!(answer, None);
            assert_eq!(
                quiet(&mut c, 1, 900, 2000),
                vec![Effect::Stop {
                    generation: 1,
                    auto: true
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
    // This executable has no window dependency. Effects are the capture/delivery
    // seam; the Tauri adapter uses the same controller through AppContext.
    let mut c = control();
    let mut host = Vec::new();
    host.push(c.input(Input::HoldStart(None)).unwrap());
    recording(&mut c, 1, Origin::Hold);
    host.push(c.input(Input::HoldStop).unwrap());
    c.observe(1, Status::Idle, Origin::Toggle);
    host.push(c.input(Input::Toggle(None)).unwrap());
    recording(&mut c, 2, Origin::Toggle);
    host.push(c.input(Input::Escape).unwrap());
    c.observe(2, Status::Idle, Origin::Toggle);
    host.push(c.input(Input::Toggle(None)).unwrap());
    recording(&mut c, 3, Origin::Toggle);
    speech(&mut c, 3, 0);
    host.extend(quiet(&mut c, 3, 600, 2000));
    host.push(c.input(Input::PasteLast).unwrap());
    assert_eq!(host.len(), 7);
    assert_eq!(host.last(), Some(&Effect::PasteLast));
    // Durable History/statistics and delivery de-duplication are explicitly 1.3.
}
