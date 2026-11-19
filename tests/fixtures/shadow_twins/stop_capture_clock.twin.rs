#[cfg(feature = "dictation-shadow")]
let decided_at = std::cell::Cell::new(None);
#[cfg(feature = "dictation-shadow")]
let decision = capture_stop.consume(|op_status| {
    seen_status.set(&flight, op_status);
    stop_ports::decide(|| {
        let at = core_now();
        decided_at.set(Some(at));
        core_stop::decide_stop_capture_result(
            &view,
            StopCaptureResultInput { op_status, ..probe },
            at,
        )
    })
});
