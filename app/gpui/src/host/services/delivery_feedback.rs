//! Main-window banner for text-delivery and file-output failures, matching the
//! shipped `useRecordingState` / `useDeliveryRecoveryListeners` behavior:
//! `auto-paste-failed` and `file-output-failed` show their service message for
//! five seconds, an auto-paste failure that ended as a confirmed clipboard copy
//! offers Try again, and every `retry_last_delivery` result replaces the banner
//! with its bounded service message. Messages are fixed service strings; no
//! transcript, clipboard or destination text ever reaches this module.
//!
//! A banner is owned by the recording generation that produced it, and a Try
//! again by a retry identity kept apart from the visible notice. The command's
//! completion, and the identity-less service event, only apply while that
//! ownership still holds: a newer recording, a dismissal or a newer retry makes
//! a late result inert instead of overwriting newer feedback.
use serde_json::Value;
use std::time::{Duration, Instant};

/// Shipped banners clear themselves after five seconds.
pub const TTL: Duration = Duration::from_secs(5);
pub const TRYING: &str = "Trying delivery again…";
/// Shown when the retry command itself fails before returning a result.
pub const RETRY_UNFINISHED: &str = "Paste Last did not finish. Try again.";
const MAX_MESSAGE: usize = 240;

/// Identity of one banner Try again: the recording generation it began under and
/// a process-unique attempt number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryToken {
    generation: u64,
    id: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub message: String,
    /// Offers Try again (`retry_last_delivery`).
    pub retryable: bool,
    /// A retry is in flight; the action is disabled until it reports.
    pub busy: bool,
}

#[derive(Default)]
pub struct DeliveryFeedback {
    pub revision: u64,
    pub notice: Option<Notice>,
    expires: Option<Instant>,
    /// The current banner came from `auto-paste-failed` and may still be
    /// promoted to retryable by its `clipboardOnly` delivery outcome.
    pending_paste: bool,
    /// The current banner reports a retry result, not a capture-time failure.
    from_retry: bool,
    /// A dismissed capture-time message stays dismissed until the next recording.
    dismissed: Option<String>,
    /// Latest recording generation announced by `dictation-generation-started`.
    generation: u64,
    retry_seq: u64,
    /// The one banner retry whose command completion may still be applied.
    retry: Option<RetryToken>,
    /// A dismissed or superseded retry's identity-less service event must not
    /// bring its notice back. Cleared by the next retry or recording.
    suppress_retry_events: bool,
}

/// Trim and bound a service message for display.
pub fn bounded(message: &str) -> Option<String> {
    let message = message.trim();
    (!message.is_empty()).then(|| message.chars().take(MAX_MESSAGE).collect())
}

fn retryable_kind(kind: &str) -> Option<bool> {
    match kind {
        "clipboard_only" | "busy" | "failed" => Some(true),
        "auto_pasted" | "empty" => Some(false),
        _ => None,
    }
}

impl DeliveryFeedback {
    fn present(
        &mut self,
        notice: Option<Notice>,
        now: Instant,
        ttl: bool,
        pending_paste: bool,
        from_retry: bool,
    ) {
        self.expires = notice.as_ref().filter(|_| ttl).map(|_| now + TTL);
        self.notice = notice;
        self.pending_paste = pending_paste;
        self.from_retry = from_retry;
        self.revision = self.revision.wrapping_add(1);
    }
    fn busy(&self) -> bool {
        self.notice.as_ref().is_some_and(|n| n.busy)
    }
    fn capture_failure(&mut self, message: &str, now: Instant, pending_paste: bool) {
        let Some(message) = bounded(message) else {
            return;
        };
        if self.busy() || self.dismissed.as_deref() == Some(message.as_str()) {
            return;
        }
        self.present(
            Some(Notice {
                message,
                retryable: false,
                busy: false,
            }),
            now,
            true,
            pending_paste,
            false,
        );
    }
    fn show_result(&mut self, kind: &str, message: &str, now: Instant) -> bool {
        let (Some(retryable), Some(message)) = (retryable_kind(kind), bounded(message)) else {
            return false;
        };
        self.present(
            Some(Notice {
                message,
                retryable,
                busy: false,
            }),
            now,
            true,
            false,
            true,
        );
        true
    }
    /// Feed a main-window event. Unrelated or malformed events are ignored.
    pub fn event(&mut self, event: &str, payload: &str, now: Instant) {
        if !matches!(
            event,
            "dictation-generation-started"
                | "auto-paste-failed"
                | "file-output-failed"
                | "dictation-delivery-outcome"
                | "delivery-retry-feedback"
        ) {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(payload) else {
            return;
        };
        match event {
            // A new recording starts a new producer. Every banner, dismissal and
            // retry from an older generation is over, and a retry that was still
            // running can no longer report into the new recording's feedback.
            "dictation-generation-started" => {
                let Some(id) = v["recordingId"].as_u64().filter(|id| *id > 0) else {
                    return;
                };
                if id < self.generation {
                    return;
                }
                self.generation = id;
                self.dismissed = None;
                self.suppress_retry_events = self.retry.take().is_some();
                if self.notice.is_some() {
                    self.present(None, now, false, false, false);
                }
            }
            "auto-paste-failed" => {
                if let Some(message) = v.as_str() {
                    self.capture_failure(message, now, true);
                }
            }
            "file-output-failed" => {
                self.pending_paste = false;
                if let Some(message) = v.as_str() {
                    self.capture_failure(message, now, false);
                }
            }
            "dictation-delivery-outcome" => {
                let valid = v["recordingId"].as_u64().is_some_and(|id| id > 0);
                let outcome = v["outcome"].as_str();
                let pending = std::mem::take(&mut self.pending_paste);
                if valid
                    && pending
                    && outcome == Some("clipboardOnly")
                    && let Some(notice) = self.notice.as_mut()
                    && !notice.busy
                    && !notice.retryable
                {
                    notice.retryable = true;
                    self.revision = self.revision.wrapping_add(1);
                }
            }
            "delivery-retry-feedback" => {
                if self.suppress_retry_events {
                    return;
                }
                if let (Some(kind), Some(message)) = (v["kind"].as_str(), v["message"].as_str())
                    && self.show_result(kind, message, now)
                {
                    // The service reported the running retry's result first.
                    self.retry = None;
                }
            }
            _ => {}
        }
    }
    /// The banner's Try again was pressed. None when one is already running.
    pub fn begin_retry(&mut self, now: Instant) -> Option<RetryToken> {
        if self.busy() {
            return None;
        }
        self.retry_seq = self.retry_seq.wrapping_add(1);
        let token = RetryToken {
            generation: self.generation,
            id: self.retry_seq,
        };
        self.retry = Some(token);
        self.dismissed = None;
        self.suppress_retry_events = false;
        self.present(
            Some(Notice {
                message: TRYING.into(),
                retryable: true,
                busy: true,
            }),
            now,
            false,
            false,
            true,
        );
        Some(token)
    }
    /// The retry command finished: `Some((kind, message))` is its result, None
    /// means it failed without one. Applied only while `token` still owns the
    /// banner; returns whether it was.
    pub fn finish_retry(
        &mut self,
        token: RetryToken,
        result: Option<(&str, &str)>,
        now: Instant,
    ) -> bool {
        if self.retry != Some(token) || token.generation != self.generation {
            return false;
        }
        self.retry = None;
        let (kind, message) = result.unwrap_or(("failed", RETRY_UNFINISHED));
        if !self.show_result(kind, message, now) {
            self.show_result("failed", RETRY_UNFINISHED, now);
        }
        true
    }
    pub fn dismiss(&mut self) {
        if let Some(notice) = self.notice.as_ref().filter(|_| !self.from_retry) {
            self.dismissed = Some(notice.message.clone());
        }
        if self.from_retry {
            // The dismissed attempt's late result, in either of its two forms,
            // must not bring the notice back.
            self.retry = None;
            self.suppress_retry_events = true;
        }
        self.present(None, Instant::now(), false, false, false);
    }
    /// Clear a banner whose five seconds have passed.
    pub fn expire(&mut self, now: Instant) {
        if self.expires.is_some_and(|at| at <= now) {
            self.present(None, now, false, false, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASTE: &str = "Text delivery timed out. Check the clipboard before pasting.";
    const FILE: &str = "Couldn't save dictation to file. Text is still in your clipboard.";

    fn at(f: &mut DeliveryFeedback, event: &str, payload: &str, now: Instant) {
        f.event(event, payload, now);
    }
    fn text(message: &str) -> String {
        serde_json::to_string(message).unwrap()
    }
    fn outcome(kind: &str) -> String {
        format!(r#"{{"recordingId":4,"outcome":"{kind}"}}"#)
    }
    fn started(f: &mut DeliveryFeedback, id: u64, now: Instant) {
        at(
            f,
            "dictation-generation-started",
            &format!(r#"{{"recordingId":{id}}}"#),
            now,
        );
    }
    fn retry_event(f: &mut DeliveryFeedback, kind: &str, message: &str, now: Instant) {
        let payload = serde_json::json!({"kind": kind, "message": message}).to_string();
        at(f, "delivery-retry-feedback", &payload, now);
    }

    #[test]
    fn auto_paste_failure_shows_the_service_message_and_clears_after_five_seconds() {
        let t0 = Instant::now();
        let mut f = DeliveryFeedback::default();
        at(&mut f, "auto-paste-failed", &text(PASTE), t0);
        let n = f.notice.clone().unwrap();
        assert_eq!(n.message, PASTE);
        assert!(!n.retryable && !n.busy);
        let shown = f.revision;
        f.expire(t0 + Duration::from_millis(4999));
        assert!(f.notice.is_some() && f.revision == shown);
        f.expire(t0 + TTL);
        assert!(f.notice.is_none());
        assert!(f.revision != shown);
    }

    #[test]
    fn clipboard_only_outcome_promotes_the_paste_banner_to_retryable_without_extending_it() {
        let t0 = Instant::now();
        let mut f = DeliveryFeedback::default();
        at(&mut f, "auto-paste-failed", &text(PASTE), t0);
        at(
            &mut f,
            "dictation-delivery-outcome",
            &outcome("clipboardOnly"),
            t0 + Duration::from_secs(2),
        );
        assert!(f.notice.as_ref().unwrap().retryable);
        f.expire(t0 + TTL);
        assert!(f.notice.is_none());
        // Other outcomes never promote, and an outcome is consumed once.
        for other in ["clipboardWriteFailed", "unconfirmed", "autoPastePosted"] {
            let mut f = DeliveryFeedback::default();
            at(&mut f, "auto-paste-failed", &text(PASTE), t0);
            at(&mut f, "dictation-delivery-outcome", &outcome(other), t0);
            assert!(!f.notice.as_ref().unwrap().retryable, "{other}");
        }
        let mut f = DeliveryFeedback::default();
        at(
            &mut f,
            "dictation-delivery-outcome",
            &outcome("clipboardOnly"),
            t0,
        );
        assert!(f.notice.is_none(), "no banner means nothing to promote");
    }

    #[test]
    fn file_output_failure_is_a_non_retryable_five_second_banner() {
        let t0 = Instant::now();
        let mut f = DeliveryFeedback::default();
        at(&mut f, "file-output-failed", &text(FILE), t0);
        let n = f.notice.clone().unwrap();
        assert_eq!(n.message, FILE);
        assert!(!n.retryable);
        // Its delivery outcome cannot turn a file failure into a paste retry.
        at(
            &mut f,
            "dictation-delivery-outcome",
            &outcome("clipboardOnly"),
            t0,
        );
        assert!(!f.notice.as_ref().unwrap().retryable);
        f.expire(t0 + TTL);
        assert!(f.notice.is_none());
    }

    #[test]
    fn file_failure_after_paste_failure_replaces_it_and_blocks_promotion() {
        let t0 = Instant::now();
        let mut f = DeliveryFeedback::default();
        at(&mut f, "auto-paste-failed", &text(PASTE), t0);
        at(&mut f, "file-output-failed", &text(FILE), t0);
        at(
            &mut f,
            "dictation-delivery-outcome",
            &outcome("clipboardOnly"),
            t0,
        );
        let n = f.notice.unwrap();
        assert_eq!(n.message, FILE);
        assert!(!n.retryable);
    }

    #[test]
    fn dismissal_survives_a_repeat_until_the_next_recording() {
        let t0 = Instant::now();
        let mut f = DeliveryFeedback::default();
        at(&mut f, "auto-paste-failed", &text(PASTE), t0);
        f.dismiss();
        assert!(f.notice.is_none());
        at(&mut f, "auto-paste-failed", &text(PASTE), t0);
        assert!(f.notice.is_none(), "dismissed message stays dismissed");
        at(&mut f, "auto-paste-failed", &text(FILE), t0);
        assert_eq!(f.notice.as_ref().unwrap().message, FILE);
        f.dismiss();
        started(&mut f, 5, t0);
        at(&mut f, "auto-paste-failed", &text(FILE), t0);
        assert_eq!(f.notice.as_ref().unwrap().message, FILE);
    }

    #[test]
    fn a_new_recording_clears_every_banner_including_a_retry_result() {
        let t0 = Instant::now();
        let mut f = DeliveryFeedback::default();
        at(&mut f, "file-output-failed", &text(FILE), t0);
        started(&mut f, 5, t0);
        assert!(f.notice.is_none());
        retry_event(&mut f, "clipboard_only", "Copied; paste manually.", t0);
        assert!(f.notice.is_some());
        started(&mut f, 6, t0);
        assert!(
            f.notice.is_none(),
            "a retry result is owned by its generation"
        );
        // An older or malformed generation announcement changes nothing.
        retry_event(&mut f, "busy", "Busy.", t0);
        started(&mut f, 3, t0);
        at(&mut f, "dictation-generation-started", "{}", t0);
        assert_eq!(f.notice.as_ref().unwrap().message, "Busy.");
    }

    #[test]
    fn retry_results_use_the_service_message_and_the_shipped_retryable_kinds() {
        let t0 = Instant::now();
        for (kind, retryable) in [
            ("auto_pasted", false),
            ("empty", false),
            ("clipboard_only", true),
            ("busy", true),
            ("failed", true),
        ] {
            let mut f = DeliveryFeedback::default();
            retry_event(&mut f, kind, &format!("Synthetic {kind} message."), t0);
            let n = f.notice.clone().unwrap();
            assert_eq!(n.message, format!("Synthetic {kind} message."));
            assert_eq!(n.retryable, retryable, "{kind}");
            assert!(!n.busy);
            f.expire(t0 + TTL);
            assert!(f.notice.is_none(), "{kind} clears after five seconds");
        }
        let mut f = DeliveryFeedback::default();
        retry_event(&mut f, "bogus", "x", t0);
        at(&mut f, "delivery-retry-feedback", r#"{"kind":"busy"}"#, t0);
        assert!(f.notice.is_none(), "malformed results are ignored");
    }

    #[test]
    fn try_again_runs_once_and_ends_with_the_result_or_the_unfinished_message() {
        let t0 = Instant::now();
        let mut f = DeliveryFeedback::default();
        at(&mut f, "auto-paste-failed", &text(PASTE), t0);
        at(
            &mut f,
            "dictation-delivery-outcome",
            &outcome("clipboardOnly"),
            t0,
        );
        let token = f.begin_retry(t0).unwrap();
        let n = f.notice.clone().unwrap();
        assert_eq!(n.message, TRYING);
        assert!(n.busy && n.retryable);
        assert!(
            f.begin_retry(t0).is_none(),
            "a retry in flight cannot start another"
        );
        // The Trying state does not expire; the result does.
        f.expire(t0 + TTL * 3);
        assert!(f.notice.as_ref().unwrap().busy);
        // A stale capture failure cannot replace the running retry.
        at(&mut f, "auto-paste-failed", &text(PASTE), t0);
        assert!(f.notice.as_ref().unwrap().busy);
        assert!(f.finish_retry(
            token,
            Some(("auto_pasted", "Pasted the latest dictation.")),
            t0
        ));
        let done = f.notice.clone().unwrap();
        assert_eq!(done.message, "Pasted the latest dictation.");
        assert!(!done.retryable && !done.busy);
        assert!(
            !f.finish_retry(token, None, t0),
            "a completion applies once"
        );

        let mut f = DeliveryFeedback::default();
        let token = f.begin_retry(t0).unwrap();
        assert!(f.finish_retry(token, None, t0));
        let n = f.notice.clone().unwrap();
        assert_eq!(n.message, RETRY_UNFINISHED);
        assert!(n.retryable && !n.busy);
        // Once a result is on screen, a late failure cannot overwrite it.
        retry_event(&mut f, "empty", "Nothing to paste yet.", t0);
        assert!(!f.finish_retry(token, None, t0));
        assert_eq!(f.notice.unwrap().message, "Nothing to paste yet.");
    }

    #[test]
    fn the_service_event_finishes_the_running_retry_and_the_late_command_result_is_inert() {
        let t0 = Instant::now();
        let mut f = DeliveryFeedback::default();
        let token = f.begin_retry(t0).unwrap();
        retry_event(&mut f, "auto_pasted", "Pasted the latest dictation.", t0);
        let shown = f.revision;
        assert!(!f.finish_retry(token, Some(("failed", "Late.")), t0));
        assert_eq!(f.revision, shown);
        assert_eq!(
            f.notice.unwrap().message,
            "Pasted the latest dictation.",
            "the already-shown result is not replaced"
        );
    }

    #[test]
    fn a_newer_recording_supersedes_a_running_retry() {
        let t0 = Instant::now();
        let mut f = DeliveryFeedback::default();
        started(&mut f, 4, t0);
        let token = f.begin_retry(t0).unwrap();
        started(&mut f, 5, t0);
        assert!(f.notice.is_none(), "the running retry's banner is gone");
        at(&mut f, "file-output-failed", &text(FILE), t0);
        let shown = (f.revision, f.notice.clone());
        // The late command result, in both forms, cannot replace newer feedback.
        assert!(!f.finish_retry(token, Some(("auto_pasted", "Pasted.")), t0));
        assert!(!f.finish_retry(token, None, t0));
        assert_eq!((f.revision, f.notice.clone()), shown);
        assert_eq!(f.notice.as_ref().unwrap().message, FILE);
        // Nor can the old retry's service event.
        retry_event(&mut f, "auto_pasted", "Pasted.", t0);
        assert_eq!(f.notice.as_ref().unwrap().message, FILE);
        // A retry started under the new recording works normally.
        f.expire(t0 + TTL);
        let next = f.begin_retry(t0).unwrap();
        assert_ne!(next, token);
        retry_event(&mut f, "busy", "Busy.", t0);
        assert_eq!(f.notice.as_ref().unwrap().message, "Busy.");
        assert!(!f.finish_retry(token, None, t0));
    }

    #[test]
    fn dismissing_a_running_retry_keeps_its_late_result_dismissed() {
        let t0 = Instant::now();
        let mut f = DeliveryFeedback::default();
        let token = f.begin_retry(t0).unwrap();
        f.dismiss();
        assert!(f.notice.is_none());
        let shown = f.revision;
        assert!(!f.finish_retry(token, Some(("failed", PASTE)), t0));
        retry_event(&mut f, "failed", PASTE, t0);
        assert!(f.notice.is_none());
        assert_eq!(f.revision, shown);
        // A new Try again is a new owner and may report.
        let next = f.begin_retry(t0).unwrap();
        assert!(f.finish_retry(next, Some(("empty", "Nothing to paste yet.")), t0));
        assert_eq!(f.notice.as_ref().unwrap().message, "Nothing to paste yet.");
    }

    #[test]
    fn dismissing_a_finished_retry_keeps_its_duplicate_service_event_dismissed() {
        let t0 = Instant::now();
        let mut f = DeliveryFeedback::default();
        let token = f.begin_retry(t0).unwrap();
        assert!(f.finish_retry(token, Some(("failed", PASTE)), t0));
        f.dismiss();
        retry_event(&mut f, "failed", PASTE, t0);
        assert!(f.notice.is_none());
        started(&mut f, 9, t0);
        retry_event(&mut f, "failed", PASTE, t0);
        assert!(
            f.notice.is_some(),
            "the next recording starts with a clean slate"
        );
    }

    #[test]
    fn retry_results_are_not_suppressed_by_a_dismissed_capture_failure() {
        let t0 = Instant::now();
        let mut f = DeliveryFeedback::default();
        at(&mut f, "auto-paste-failed", &text(PASTE), t0);
        f.dismiss();
        let token = f.begin_retry(t0).unwrap();
        assert!(f.finish_retry(token, Some(("failed", PASTE)), t0));
        assert_eq!(f.notice.as_ref().unwrap().message, PASTE);
    }

    #[test]
    fn messages_are_bounded_and_non_string_or_malformed_payloads_are_ignored() {
        let t0 = Instant::now();
        let mut f = DeliveryFeedback::default();
        at(&mut f, "auto-paste-failed", "not json", t0);
        at(&mut f, "auto-paste-failed", r#"{"message":"x"}"#, t0);
        at(&mut f, "auto-paste-failed", r#""   ""#, t0);
        assert!(f.notice.is_none());
        at(&mut f, "auto-paste-failed", &text(&"x".repeat(1000)), t0);
        assert_eq!(f.notice.unwrap().message.chars().count(), MAX_MESSAGE);
    }
}
