let decision = stop_ports::decide(|| match kind {
    PostKind::StopPost => core_stop::decide_pipeline_idle_emit(&view, input, core_now()),
    PostKind::LegacyPost => core_process_audio::decide_process_audio_finish(
        &view,
        ProcessAudioFinishInput {
            rid: input.rid,
            kind: ProcessAudioFinishKind::PipelineDone,
            closure_emit: true,
        },
        core_now(),
    ),
});
// With the shadow compiled in: the same decision, keeping its own timestamp for the
// flight (D-S2c-9 e).
