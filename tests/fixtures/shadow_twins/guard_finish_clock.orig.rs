let decide = || {
    stop_ports::decide(|| match origin {
        GuardOrigin::LegacyParse => core_process_audio::decide_process_audio_finish(
            &view,
            ProcessAudioFinishInput {
                rid: input.rid,
                kind: ProcessAudioFinishKind::ParseFailed,
                closure_emit: input.closure_emit,
            },
            core_now(),
        ),
        GuardOrigin::StopCapture | GuardOrigin::PipelineExit | GuardOrigin::Dropped => {
            core_stop::decide_guard_finish(&view, input, core_now())
        }
    })
};
// With the shadow compiled in: the same decision, whose own timestamp (read once,
// where the original reads it) is kept for the flight, so nothing reads the clock
// again under D (D-S2c-9 e).
