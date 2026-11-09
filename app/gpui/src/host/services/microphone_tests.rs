//! Production-path regressions for the microphone test, Smart Auto feedback and
//! startup diagnostic. Every payload is synthetic and shaped like the service's
//! wire format; no device, capture or private content is involved.
use super::*;

fn settings() -> Value {
    json!({
        "microphone":"synthetic.saved",
        "vadSensitivity":50,
        "smartAutoMicrophoneEnabled":false,
        "smartAutoApprovedDeviceIds":["synthetic.a","synthetic.b"],
        "smartAutoPreferredDeviceIds":["synthetic.a"],
        "smartAutoAllowContinuity":false,
        "smartAutoProbeEnabled":true,
    })
}
fn fixed(id: &str) -> Config {
    Config {
        device: id.into(),
        smart: None,
    }
}
fn device(id: &str, available: bool) -> Device {
    Device {
        id: id.into(),
        label: format!("Synthetic {id}"),
        available,
        continuity: false,
    }
}
fn idle() -> Value {
    json!({"previewId":null,"state":"idle"})
}
fn running(id: u64, state: &str) -> Value {
    json!({"previewId":id,"state":state,"stillConnecting":false,"errorKind":null,"message":null})
}

#[test]
fn smart_auto_preview_never_sends_a_saved_fixed_microphone() {
    let fixed_config = config(&settings());
    assert_eq!(fixed_config, fixed("synthetic.saved"));
    assert_eq!(
        start_args(&fixed_config, 50),
        json!({"deviceId":"synthetic.saved","smartAuto":null,"vadSensitivity":50})
    );
    let mut smart = settings();
    smart["smartAutoMicrophoneEnabled"] = json!(true);
    let args = start_args(&config(&smart), vad(&smart));
    // The service rejects a fixed device together with Smart Auto.
    assert_eq!(args["deviceId"], "system_default");
    assert_eq!(
        args["smartAuto"]["approvedDeviceIds"],
        json!(["synthetic.a", "synthetic.b"])
    );
    assert_eq!(args["smartAuto"]["requireRecentSignal"], true);
    assert_eq!(args["vadSensitivity"], 50);
    smart["microphone"] = json!("");
    assert_eq!(config(&smart).device, "system_default");
}

#[test]
fn running_preview_follows_device_and_sensitivity_changes() {
    let mut state = Loop::default();
    let a = fixed("synthetic.a");
    assert_eq!(state.step(&a, 50, &idle(), false), Step::Start);
    state.started(7, a.clone(), 50);
    let live = running(7, "active");
    assert_eq!(state.step(&a, 50, &live, false), Step::Wait);
    assert_eq!(state.step(&a, 80, &live, false), Step::Vad(7, 80));
    state.sent(80);
    assert_eq!(state.step(&a, 80, &live, false), Step::Wait);
    let b = fixed("synthetic.b");
    assert_eq!(state.step(&b, 80, &live, false), Step::Stop(7));
    assert_eq!(
        state.step(&b, 80, &running(7, "stopping"), false),
        Step::Wait
    );
    state.stopped();
    assert_eq!(state.step(&b, 80, &idle(), false), Step::Start);
    // Toggling Smart Auto is a different configuration too.
    let mut smart = settings();
    smart["smartAutoMicrophoneEnabled"] = json!(true);
    state.started(8, b, 80);
    assert_eq!(
        state.step(&config(&smart), 80, &running(8, "active"), false),
        Step::Stop(8)
    );
}

#[test]
fn a_capture_this_visit_did_not_open_is_not_adopted() {
    let mut state = Loop::default();
    assert_eq!(
        state.step(&fixed("synthetic.a"), 50, &running(3, "active"), false),
        Step::Stop(3)
    );
}

#[test]
fn refused_or_failed_previews_wait_for_a_change_or_retry() {
    let mut state = Loop::default();
    let a = fixed("synthetic.a");
    state.suspended = Some(a.clone());
    assert_eq!(state.step(&a, 50, &idle(), false), Step::Wait);
    assert_eq!(state.step(&a, 80, &idle(), false), Step::Wait);
    assert_eq!(state.step(&a, 50, &idle(), true), Step::Start);
    state.suspended = Some(a);
    // A different device or Smart Auto setting is a configuration change.
    assert_eq!(
        state.step(&fixed("synthetic.b"), 50, &idle(), false),
        Step::Start
    );
    assert!(state.suspended.is_none());
}

#[test]
fn terminal_preview_error_suspends_until_retry() {
    let mut state = Loop::default();
    let a = fixed("synthetic.a");
    state.started(4, a.clone(), 50);
    let terminal = json!({"previewId":null,"state":"error","errorKind":"stream_interrupted","message":"The microphone stopped unexpectedly."});
    assert_eq!(state.step(&a, 50, &terminal, false), Step::Wait);
    assert_eq!(state.step(&a, 50, &terminal, false), Step::Wait);
    assert_eq!(state.step(&a, 50, &terminal, true), Step::Start);
}

#[test]
fn preview_feedback_explains_connecting_active_and_failed_states() {
    let test = mic_test(&running(1, "connecting"), None, None, 50);
    assert_eq!(test.connecting, Some(false));
    let mut slow = running(1, "connecting");
    slow["stillConnecting"] = json!(true);
    assert_eq!(mic_test(&slow, None, None, 50).connecting, Some(true));
    let live = mic_test(&running(1, "active"), None, None, 50);
    assert!(live.live && live.error.is_none() && !live.retry);
    let terminal =
        json!({"previewId":null,"state":"error","message":"The microphone stopped unexpectedly."});
    let failed = mic_test(&terminal, None, None, 50);
    assert_eq!(
        failed.error.as_deref(),
        Some("The microphone stopped unexpectedly.")
    );
    assert!(failed.retry);
    let silent = mic_test(&json!({"previewId":null,"state":"error"}), None, None, 50);
    assert_eq!(silent.error.as_deref(), Some("Microphone preview stopped."));
    // A refused start is explained and retryable even with an idle service.
    let refused = mic_test(
        &idle(),
        Some("Enable Murmur before testing the microphone"),
        None,
        50,
    );
    assert_eq!(
        refused.error.as_deref(),
        Some("Enable Murmur before testing the microphone")
    );
    assert!(refused.retry);
}

#[test]
fn voice_detection_unavailable_applies_to_the_current_preview_and_sensitivity() {
    let live = running(9, "active");
    assert!(mic_test(&live, None, Some((9, 50, true)), 50).vad_unavailable);
    assert!(!mic_test(&live, None, Some((9, 50, false)), 50).vad_unavailable);
    // Another preview or an earlier sensitivity does not describe this one.
    assert!(!mic_test(&live, None, Some((8, 50, true)), 50).vad_unavailable);
    assert!(!mic_test(&live, None, Some((9, 20, true)), 50).vad_unavailable);
    assert!(!mic_test(&idle(), None, Some((9, 50, true)), 50).vad_unavailable);
}

#[test]
fn vad_events_are_recorded_and_refresh_only_on_change() {
    let preview = Preview::default();
    let event = |decision: &str, sensitivity: u64| json!({"previewId":9,"sensitivity":sensitivity,"decision":decision});
    assert!(preview.observe("microphone-preview-vad", &event("speech_detected", 50)));
    assert!(!preview.observe("microphone-preview-vad", &event("no_speech", 50)));
    assert!(preview.observe("microphone-preview-vad", &event("unavailable", 50)));
    assert_eq!(*lock(&preview.vad), Some((9, 50, true)));
    assert!(!preview.observe("microphone-preview-vad", &json!({"decision":"unavailable"})));
}

#[test]
fn smart_auto_reports_probing_blocked_and_unreadable_status() {
    let mut smart = settings();
    smart["smartAutoMicrophoneEnabled"] = json!(true);
    let devices = [device("synthetic.a", true), device("synthetic.b", false)];
    let probing =
        json!({"state":"probing","deviceId":"synthetic.a","phase":"verifying","remainingMs":1500});
    assert_eq!(
        smart_auto(&smart, &devices, true, Ok(probing), false),
        (
            SmartAuto::Probing {
                device: "Synthetic synthetic.a".into(),
                phase: "checking signal"
            },
            Some(Duration::from_millis(1500))
        )
    );
    let unknown = json!({"state":"probing","deviceId":"synthetic.gone","phase":"connecting","remainingMs":10});
    assert_eq!(
        smart_auto(&smart, &devices, true, Ok(unknown), false).0,
        SmartAuto::Probing {
            device: "included microphone".into(),
            phase: "connecting"
        }
    );
    let blocked = json!({"state":"blocked","message":"No included microphone produced a signal.","retryAfterMs":2300});
    assert_eq!(
        smart_auto(&smart, &devices, true, Ok(blocked.clone()), true),
        (
            SmartAuto::Blocked {
                message: "No included microphone produced a signal.".into(),
                cooldown_secs: Some(3),
                retry: true,
                retry_failed: true
            },
            Some(Duration::from_millis(2300))
        )
    );
    assert_eq!(
        smart_auto(
            &smart,
            &devices,
            true,
            Err("Native view is unavailable".into()),
            false
        )
        .0,
        SmartAuto::Unavailable
    );
    assert_eq!(
        smart_auto(
            &smart,
            &devices,
            true,
            Ok(json!({"state":"mystery"})),
            false
        )
        .0,
        SmartAuto::Unavailable
    );
    // A ready selection, or Smart Auto being off, needs no feedback.
    let ready = json!({"state":"ready","deviceId":"synthetic.a","reason":"recent_signal","validForMs":null});
    assert_eq!(
        smart_auto(&smart, &devices, true, Ok(ready), false).0,
        SmartAuto::Inactive
    );
    assert_eq!(
        smart_auto(&settings(), &devices, true, Ok(blocked), false).0,
        SmartAuto::Inactive
    );
}

#[test]
fn smart_auto_blocked_explains_missing_microphones_and_hides_retry() {
    let mut smart = settings();
    smart["smartAutoMicrophoneEnabled"] = json!(true);
    let blocked = json!({"state":"blocked","message":"Service wording.","retryAfterMs":null});
    let none_available = [device("synthetic.a", false), device("synthetic.b", false)];
    let (feedback, deadline) =
        smart_auto(&smart, &none_available, false, Ok(blocked.clone()), false);
    assert_eq!(
        feedback,
        SmartAuto::Blocked {
            message: "None of the included microphones are available right now.".into(),
            cooldown_secs: None,
            retry: false,
            retry_failed: false
        }
    );
    assert_eq!(deadline, None);
    smart["smartAutoApprovedDeviceIds"] = json!([]);
    let (feedback, _) = smart_auto(&smart, &none_available, false, Ok(blocked.clone()), false);
    assert!(
        matches!(feedback, SmartAuto::Blocked { message, retry: false, .. }
        if message == "Include at least one microphone before Smart Auto can record.")
    );
    // Background checks disabled: no retry even with a ready microphone.
    smart["smartAutoApprovedDeviceIds"] = json!(["synthetic.a"]);
    smart["smartAutoProbeEnabled"] = json!(false);
    let ready_device = [device("synthetic.a", true)];
    assert!(matches!(
        smart_auto(&smart, &ready_device, true, Ok(blocked), false).0,
        SmartAuto::Blocked { retry: false, .. }
    ));
}
