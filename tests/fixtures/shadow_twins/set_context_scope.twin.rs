#[cfg(feature = "dictation-shadow")]
match observing.as_ref() {
    Some(observing) => observing.slots.mirror.set(Some(
        FieldAdapter::set_context_observed(ports.app_state(), id.0, snapshot),
    )),
    None => FieldAdapter::set_context(ports.app_state(), id.0, snapshot),
}
