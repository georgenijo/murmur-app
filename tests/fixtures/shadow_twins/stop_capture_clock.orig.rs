let decision = capture_stop.consume(|op_status| {
    stop_ports::decide(|| {
        core_stop::decide_stop_capture_result(
            &view,
            StopCaptureResultInput { op_status, ..probe },
            core_now(),
        )
    })
});
