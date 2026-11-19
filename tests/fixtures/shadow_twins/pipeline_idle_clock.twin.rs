#[cfg(feature = "dictation-shadow")]
let decided_at = std::cell::Cell::new(None);
#[cfg(feature = "dictation-shadow")]
let decision = stop_ports::decide(|| {
    let at = core_now();
    decided_at.set(Some(at));
    match kind {
        PostKind::StopPost => core_stop::decide_pipeline_idle_emit(&view, input, at),
        PostKind::LegacyPost => core_process_audio::decide_process_audio_finish(
            &view,
            ProcessAudioFinishInput {
                rid: input.rid,
                kind: ProcessAudioFinishKind::PipelineDone,
                closure_emit: true,
            },
            at,
        ),
    }
});
