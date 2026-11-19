#[cfg(feature = "dictation-shadow")]
let decided_at = std::cell::Cell::new(None);
#[cfg(feature = "dictation-shadow")]
let decide = || {
    stop_ports::decide(|| {
        let at = core_now();
        decided_at.set(Some(at));
        match origin {
            GuardOrigin::LegacyParse => core_process_audio::decide_process_audio_finish(
                &view,
                ProcessAudioFinishInput {
                    rid: input.rid,
                    kind: ProcessAudioFinishKind::ParseFailed,
                    closure_emit: input.closure_emit,
                },
                at,
            ),
            GuardOrigin::StopCapture | GuardOrigin::PipelineExit | GuardOrigin::Dropped => {
                core_stop::decide_guard_finish(&view, input, at)
            }
        }
    })
};
