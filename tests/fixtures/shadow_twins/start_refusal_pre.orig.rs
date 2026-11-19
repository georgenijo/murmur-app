return start_ports::release_and_reply(
    host,
    state,
    &slots,
    pre,
    state.app_state.recording_id.load(Ordering::SeqCst),
    None,
);
