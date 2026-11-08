//! Bounded, content-free recording feedback. Idle/cleanup never erase a failure;
//! a new generation or explicit dismissal does. No backend error text is copied.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    MicrophoneSettings,
    ChooseMicrophone,
    Retry,
    Reinstall,
    Restart,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Notice {
    pub message: &'static str,
    pub recovery: Recovery,
}
impl Notice {
    pub fn from_code(code: &str) -> Self {
        use Recovery::*;
        let (message, recovery) = match code {
            "microphone_not_determined" => (
                "Allow microphone access in the macOS prompt, then try dictation again.",
                MicrophoneSettings,
            ),
            "permission_denied" | "microphone_denied" => (
                "Microphone access was denied or revoked. Enable Murmur in Privacy & Security → Microphone, then try again.",
                MicrophoneSettings,
            ),
            "microphone_unknown" | "microphone_request_failed" | "permission_prompt_timeout" => (
                "Microphone access could not be confirmed. Check Privacy & Security → Microphone, then try again.",
                MicrophoneSettings,
            ),
            "signature_invalid" => (
                "Murmur’s capture helper could not be verified. Reinstall Murmur from a trusted download, then try again.",
                Reinstall,
            ),
            "device_unavailable" | "resource_exhausted" => (
                "Your microphone is unavailable or busy. Reconnect it or close other recording apps, then choose a microphone in Recording settings.",
                ChooseMicrophone,
            ),
            "termination_unconfirmed" | "cleanup_stalled" => (
                "macOS has not finished stopping the microphone. Restart Murmur, then try again.",
                Restart,
            ),
            "interrupted_partial" => (
                "Microphone capture was interrupted. Murmur is transcribing the audio received so far.",
                Retry,
            ),
            "runtime_interruption" => (
                "Microphone capture was interrupted. Check your microphone connection, then try again.",
                ChooseMicrophone,
            ),
            "transcription_failed" | "pipeline_failure" | "stop_failure" => (
                "Dictation could not finish. Check your transcription model and microphone, then try again.",
                Retry,
            ),
            _ => (
                "Microphone capture failed. Check your microphone connection and try dictation again.",
                Retry,
            ),
        };
        Self { message, recovery }
    }
}
#[derive(Default)]
pub struct Feedback {
    generation: u64,
    terminal_seen: Option<u64>,
    cancelled: Option<u64>,
    interrupted: Option<u64>,
    stop_seen: Option<u64>,
    pub revision: u64,
    pub notice: Option<Notice>,
}
impl Feedback {
    fn set(&mut self, notice: Option<Notice>) {
        self.notice = notice;
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn dismiss(&mut self) {
        self.set(None);
    }
    pub fn event(&mut self, event: &str, payload: &str) {
        if !matches!(
            event,
            "dictation-generation-started"
                | "recording-cancelled"
                | "dictation-control-error"
                | "recording-recovery-stalled"
                | "recording-interrupted"
                | "dictation-stop-completed"
                | "dictation-stop-failed"
        ) {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(payload) else {
            return;
        };
        if event == "dictation-control-error" {
            if let Some(code) = v["code"].as_str() {
                self.terminal_seen = Some(self.generation);
                self.set(Some(Notice::from_code(code)));
            } else if self.notice.is_none()
                && self.terminal_seen != Some(self.generation)
                && self.stop_seen != Some(self.generation)
                && self.cancelled != Some(self.generation)
            {
                // An unscoped control error is only a fallback. It neither
                // replaces specific feedback nor consumes the capture terminal.
                self.set(Some(Notice {
                    message: "Could not start dictation. Check Recording settings, then try again.",
                    recovery: Recovery::Retry,
                }));
            }
            return;
        }
        let Some(generation) = v["recordingId"].as_u64().filter(|g| *g > 0) else {
            return;
        };
        if generation < self.generation
            || (self.cancelled == Some(generation) && event != "dictation-generation-started")
            || (self.stop_seen == Some(generation)
                && matches!(
                    event,
                    "recording-interrupted" | "dictation-stop-completed" | "dictation-stop-failed"
                ))
        {
            return;
        }
        self.generation = generation;
        match event {
            "dictation-generation-started" => {
                self.cancelled = None;
                self.interrupted = None;
                self.set(None);
            }
            "recording-cancelled" => {
                self.cancelled = Some(generation);
                self.set(None);
            }
            "recording-recovery-stalled" => self.set(Some(Notice::from_code("cleanup_stalled"))),
            "recording-interrupted" => {
                self.interrupted = (v["autoTranscribe"] == true).then_some(generation);
                self.set(Some(Notice::from_code(if v["autoTranscribe"] == true {
                    "interrupted_partial"
                } else {
                    "runtime_interruption"
                })))
            }
            "dictation-stop-completed" | "dictation-stop-failed" => {
                self.stop_seen = Some(generation);
                if event == "dictation-stop-failed" {
                    if self.notice != Some(Notice::from_code("cleanup_stalled"))
                        && (self.interrupted == Some(generation)
                            || self.terminal_seen != Some(generation))
                    {
                        self.set(Some(Notice::from_code("transcription_failed")));
                    }
                } else if self.notice == Some(Notice::from_code("interrupted_partial")) {
                    self.set(None);
                }
                self.interrupted = None;
            }
            _ => {}
        }
    }
    /// The service snapshot is durable for this process, even when Starting and
    /// Idle both arrive between frames. Only the current, uncancelled owner is read.
    pub fn terminal(&mut self, terminal: Option<(u64, &str, &str)>) {
        let Some((generation, outcome, code)) = terminal else {
            return;
        };
        if generation < self.generation
            || self.terminal_seen == Some(generation)
            || self.cancelled == Some(generation)
            || self.stop_seen == Some(generation)
        {
            return;
        }
        self.generation = generation;
        // Capture telemetry can terminate before partial transcription starts.
        // Its runtime interruption is not the final transcription result.
        if outcome == "runtime_interruption" && self.interrupted == Some(generation) {
            return;
        }
        self.terminal_seen = Some(generation);
        self.interrupted = None;
        if matches!(
            outcome,
            "capture_init_failure" | "runtime_interruption" | "stop_failure" | "pipeline_failure"
        ) {
            // A stalled cleanup still requires a restart after processing ends.
            if self.notice == Some(Notice::from_code("cleanup_stalled")) {
                return;
            }
            self.set(Some(Notice::from_code(if code == "runtime_failure" {
                outcome
            } else {
                code
            })));
        } else if self.notice == Some(Notice::from_code("interrupted_partial")) {
            self.set(None);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn specific_capture_failure_wins_before_or_after_generic_control_error() {
        for code in [
            "signature_invalid",
            "permission_denied",
            "device_unavailable",
        ] {
            for terminal_first in [false, true] {
                let mut f = Feedback::default();
                f.event("dictation-generation-started", r#"{"recordingId":8}"#);
                if terminal_first {
                    f.terminal(Some((8, "capture_init_failure", code)));
                }
                f.event("dictation-control-error", r#""PRIVATE SYNTHETIC SENTINEL""#);
                f.terminal(Some((8, "capture_init_failure", code)));
                assert_eq!(f.notice, Some(Notice::from_code(code)));
                f.dismiss();
                f.event("dictation-control-error", r#""Could not start dictation.""#);
                f.terminal(Some((8, "capture_init_failure", code)));
                assert!(f.notice.is_none(), "dismissed failure must stay dismissed");
            }
        }
    }

    #[test]
    fn interruption_progress_is_replaced_by_final_terminal_success_or_failure() {
        for (outcome, code, expected) in [
            ("success", "none", None),
            ("no_speech", "vad_no_speech", None),
            (
                "pipeline_failure",
                "transcription_failed",
                Some(Notice::from_code("transcription_failed")),
            ),
            (
                "capture_init_failure",
                "permission_denied",
                Some(Notice::from_code("permission_denied")),
            ),
        ] {
            let mut f = Feedback::default();
            f.event("dictation-generation-started", r#"{"recordingId":8}"#);
            f.event(
                "recording-interrupted",
                r#"{"recordingId":8,"autoTranscribe":true}"#,
            );
            assert_eq!(f.notice, Some(Notice::from_code("interrupted_partial")));
            f.terminal(Some((8, outcome, code)));
            assert_eq!(f.notice, expected);
            f.dismiss();
            f.terminal(Some((8, outcome, code)));
            assert!(f.notice.is_none());
        }
    }

    #[test]
    fn partial_transcription_result_wins_over_capture_terminal_in_either_order() {
        for event in ["dictation-stop-completed", "dictation-stop-failed"] {
            for capture_first in [false, true] {
                let mut f = Feedback::default();
                f.event("dictation-generation-started", r#"{"recordingId":8}"#);
                if capture_first {
                    f.terminal(Some((8, "runtime_interruption", "device_unavailable")));
                }
                f.event(
                    "recording-interrupted",
                    r#"{"recordingId":8,"autoTranscribe":true}"#,
                );
                f.terminal(Some((8, "runtime_interruption", "device_unavailable")));
                assert_eq!(f.notice, Some(Notice::from_code("interrupted_partial")));
                f.event(
                    event,
                    r#"{"recordingId":8,"error":"PRIVATE SYNTHETIC SENTINEL"}"#,
                );
                f.terminal(Some((8, "runtime_interruption", "device_unavailable")));
                let expected = (event == "dictation-stop-failed")
                    .then(|| Notice::from_code("transcription_failed"));
                assert_eq!(f.notice, expected);
                // A delayed progress event cannot resurrect a finished operation.
                f.event(
                    "recording-interrupted",
                    r#"{"recordingId":8,"autoTranscribe":true}"#,
                );
                assert_eq!(f.notice, expected);
                f.dismiss();
                f.event(event, r#"{"recordingId":8}"#);
                f.terminal(Some((8, "runtime_interruption", "device_unavailable")));
                assert!(f.notice.is_none());
            }
        }
    }

    #[test]
    fn cleanup_stall_remains_actionable_when_transcription_finishes() {
        for event in ["dictation-stop-completed", "dictation-stop-failed"] {
            for stall_first in [false, true] {
                let mut f = Feedback::default();
                f.event("dictation-generation-started", r#"{"recordingId":8}"#);
                f.event(
                    "recording-interrupted",
                    r#"{"recordingId":8,"autoTranscribe":true}"#,
                );
                if stall_first {
                    f.event("recording-recovery-stalled", r#"{"recordingId":8}"#);
                }
                f.event(event, r#"{"recordingId":8}"#);
                if !stall_first {
                    f.event("recording-recovery-stalled", r#"{"recordingId":8}"#);
                }
                assert_eq!(f.notice, Some(Notice::from_code("cleanup_stalled")));
            }
        }
    }

    #[test]
    fn stale_or_cancelled_stop_results_cannot_change_current_feedback() {
        for event in ["dictation-stop-completed", "dictation-stop-failed"] {
            let mut f = Feedback::default();
            f.event("dictation-generation-started", r#"{"recordingId":8}"#);
            f.event(
                "recording-interrupted",
                r#"{"recordingId":8,"autoTranscribe":true}"#,
            );
            f.event("recording-cancelled", r#"{"recordingId":8}"#);
            f.event(event, r#"{"recordingId":8}"#);
            assert!(f.notice.is_none());
            f.event("dictation-generation-started", r#"{"recordingId":9}"#);
            f.terminal(Some((9, "capture_init_failure", "signature_invalid")));
            f.event(event, r#"{"recordingId":8}"#);
            assert_eq!(f.notice, Some(Notice::from_code("signature_invalid")));
        }
    }

    #[test]
    fn fast_failures_survive_idle_and_have_specific_recovery_without_private_text() {
        for (code, recovery, word) in [
            (
                "permission_denied",
                Recovery::MicrophoneSettings,
                "Privacy & Security",
            ),
            ("signature_invalid", Recovery::Reinstall, "Reinstall"),
            ("device_unavailable", Recovery::ChooseMicrophone, "busy"),
            ("resource_exhausted", Recovery::ChooseMicrophone, "busy"),
            ("backend_error", Recovery::Retry, "try dictation again"),
        ] {
            let mut f = Feedback::default();
            f.event("dictation-generation-started", r#"{"recordingId":4}"#);
            f.terminal(Some((4, "capture_init_failure", code)));
            f.event("recording-status-changed", r#""idle""#);
            f.event("recording-cleanup-finished", r#"{"recordingId":4}"#);
            let n = f.notice.unwrap();
            assert_eq!(n.recovery, recovery);
            assert!(n.message.contains(word));
            f.dismiss();
            f.terminal(Some((4, "capture_init_failure", code)));
            assert!(f.notice.is_none());
            f.event("dictation-generation-started", r#"{"recordingId":5}"#);
            f.terminal(Some((4, "capture_init_failure", code)));
            assert!(f.notice.is_none());
        }
        let mut f = Feedback::default();
        f.event(
            "dictation-control-error",
            r#"{"code":"microphone_not_determined","error":"PRIVATE SYNTHETIC SENTINEL"}"#,
        );
        assert_eq!(f.notice.unwrap().recovery, Recovery::MicrophoneSettings);
        assert!(!f.notice.unwrap().message.contains("SENTINEL"));
        f.event("dictation-generation-started", r#"{"recordingId":6}"#);
        f.event("recording-cancelled", r#"{"recordingId":6}"#);
        f.terminal(Some((6, "capture_init_failure", "signature_invalid")));
        assert!(
            f.notice.is_none(),
            "cancellation wins a previously read terminal snapshot"
        );
        f.event("dictation-generation-started", r#"{"recordingId":7}"#);
        f.event("dictation-control-error", r#"{"code":"microphone_denied"}"#);
        f.terminal(Some((7, "capture_init_failure", "signature_invalid")));
        assert_eq!(f.notice.unwrap().recovery, Recovery::MicrophoneSettings);
        for outcome in ["pipeline_failure", "stop_failure", "runtime_interruption"] {
            let mut f = Feedback::default();
            f.terminal(Some((1, outcome, "runtime_failure")));
            assert!(f.notice.is_some());
        }
    }
}
