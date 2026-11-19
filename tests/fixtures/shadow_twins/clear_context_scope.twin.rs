#[cfg(feature = "dictation-shadow")]
match observing.as_ref() {
    Some(observing) => {
        let (_, witness) =
            FieldAdapter::clear_context_observed(ports.app_state(), id.0);
        observing.slots.mirror.set(Some(witness));
    }
    None => {
        FieldAdapter::clear_context(ports.app_state(), id.0);
    }
}
