//! Privacy-safe structured tracing for selected-text transform passes.
//!
//! This module intentionally exposes only identifiers, stable enum-like
//! strings, counts, buckets, booleans, and timings. It has no parameter for
//! selected text, spoken instructions, proposals, clipboard contents, paths,
//! bundle identifiers, or device names.

pub fn key_start(transform_pass_id: u64) {
    tracing::info!(
        target: "transform",
        transform_pass_id,
        event = "hold_start",
        "transform_key"
    );
}

pub fn key_stop(transform_pass_id: u64, elapsed_ms: u64, reason: &'static str) {
    tracing::info!(
        target: "transform",
        transform_pass_id,
        event = "hold_stop",
        elapsed_ms,
        reason,
        "transform_key"
    );
}

pub fn transition(transform_pass_id: u64, from: &'static str, to: &'static str, won: bool) {
    tracing::info!(
        target: "transform",
        transform_pass_id,
        from,
        to,
        won,
        "transform_status_transition"
    );
}

pub fn resolution(
    transform_pass_id: u64,
    outcome: &'static str,
    stage: &'static str,
    error_code: Option<&'static str>,
) {
    if let Some(error_code) = error_code {
        tracing::info!(
            target: "transform",
            transform_pass_id,
            outcome,
            stage,
            error_code,
            "transform_pass_outcome"
        );
    } else {
        tracing::info!(
            target: "transform",
            transform_pass_id,
            outcome,
            stage,
            "transform_pass_outcome"
        );
    }
}

pub fn audio(
    transform_pass_id: u64,
    event: &'static str,
    outcome: &'static str,
    samples: usize,
    audio_ms: u64,
) {
    tracing::info!(
        target: "transform",
        transform_pass_id,
        event,
        outcome,
        samples,
        audio_ms,
        "transform_instruction_audio"
    );
}

pub fn instruction(
    transform_pass_id: u64,
    attempt: u64,
    outcome: &'static str,
    length_bucket: Option<&'static str>,
    duration_ms: u64,
) {
    if let Some(length_bucket) = length_bucket {
        tracing::info!(
            target: "transform",
            transform_pass_id,
            attempt,
            outcome,
            length_bucket,
            duration_ms,
            "transform_instruction"
        );
    } else {
        tracing::info!(
            target: "transform",
            transform_pass_id,
            attempt,
            outcome,
            duration_ms,
            "transform_instruction"
        );
    }
}

pub fn capture_attempt(
    transform_pass_id: u64,
    attempt: u32,
    outcome: &'static str,
    duration_ms: u64,
) {
    tracing::info!(
        target: "transform",
        transform_pass_id,
        attempt,
        outcome,
        duration_ms,
        "transform_capture_attempt"
    );
}

pub fn capture_path(
    transform_pass_id: u64,
    via: &'static str,
    outcome: &'static str,
    duration_ms: u64,
    length_bucket: Option<&'static str>,
) {
    if let Some(length_bucket) = length_bucket {
        tracing::info!(
            target: "transform",
            transform_pass_id,
            via,
            outcome,
            duration_ms,
            length_bucket,
            "transform_capture_path"
        );
    } else {
        tracing::info!(
            target: "transform",
            transform_pass_id,
            via,
            outcome,
            duration_ms,
            "transform_capture_path"
        );
    }
}

pub fn effect(
    transform_pass_id: u64,
    effect: &'static str,
    outcome: &'static str,
    error_code: Option<&'static str>,
) {
    if let Some(error_code) = error_code {
        tracing::info!(
            target: "transform",
            transform_pass_id,
            effect,
            outcome,
            error_code,
            "transform_effect"
        );
    } else {
        tracing::info!(
            target: "transform",
            transform_pass_id,
            effect,
            outcome,
            "transform_effect"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::{Layer, Registry};

    #[derive(Clone, Default)]
    struct CaptureLayer(Arc<Mutex<Vec<String>>>);

    impl<S: tracing::Subscriber> Layer<S> for CaptureLayer {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            struct Visitor(Vec<String>);
            impl tracing::field::Visit for Visitor {
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    self.0.push(format!("{}={value:?}", field.name()));
                }
            }
            let mut visitor = Visitor(Vec::new());
            event.record(&mut visitor);
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend(visitor.0);
        }
    }

    #[test]
    fn trace_helpers_never_emit_transform_content() {
        let layer = CaptureLayer::default();
        let captured = Arc::clone(&layer.0);
        let subscriber = Registry::default().with(layer);
        let selection = "SENTINEL_SELECTED_TEXT";
        let instruction_text = "SENTINEL_INSTRUCTION_TEXT";
        let proposal = "SENTINEL_PROPOSAL_TEXT";

        tracing::subscriber::with_default(subscriber, || {
            key_start(42);
            key_stop(42, 330, "released");
            transition(42, "idle", "capturing", true);
            audio(42, "stopped", "ok", 1600, 100);
            instruction(
                42,
                1,
                "ok",
                Some(crate::selection::length_bucket(instruction_text.len())),
                12,
            );
            capture_path(
                42,
                "ax_attempt",
                "ok",
                4,
                Some(crate::selection::length_bucket(selection.len())),
            );
            effect(42, "show", "ok", None);
            resolution(42, "ready", "sidecar", None);
            let _ = proposal.len();
        });

        let output = captured
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .join("\n");
        assert!(!output.contains(selection));
        assert!(!output.contains(instruction_text));
        assert!(!output.contains(proposal));
        assert!(output.contains("transform_pass_id=42"));
        assert!(output.contains("length_bucket"));
    }
}
