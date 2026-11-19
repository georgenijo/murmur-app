#[cfg(feature = "dictation-shadow")]
return start_ports::release_and_reply_observed(
    host,
    state,
    &slots,
    pre,
    state.app_state.recording_id.load(Ordering::SeqCst),
    None,
    &flight_101,
);
