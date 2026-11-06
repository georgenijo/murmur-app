//! Bounded native monotonic stamps, claimed only by the originating request's
//! opaque token. No key label, device identifier, or user content is stored.
use crate::MutexExt;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const MAX_PENDING: usize = 8;
const MAX_AGE: Duration = Duration::from_secs(2);
static PENDING: Mutex<VecDeque<Pending>> = Mutex::new(VecDeque::new());

pub(crate) use murmur_core::keyboard::Candidate;
#[derive(Clone, Copy)]
struct Pending {
    candidate: Candidate,
    emitted: Instant,
    hold: bool,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct AcceptedGesture {
    pub candidate: Candidate,
    pub accepted: Instant,
    pub gap_ms: Option<u64>,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct StartGesture {
    pub expected: bool,
    pub timing: Option<AcceptedGesture>,
}
/// Claim at command entry, before any await, validation, or busy return. A
/// rejected request drops its owned stamp; it cannot be reused by a later call.
pub(crate) struct GestureClaim {
    expected: bool,
    stamp: Option<Pending>,
}
impl GestureClaim {
    pub fn accepted(self, hold: bool) -> StartGesture {
        let now = Instant::now();
        StartGesture {
            expected: self.expected,
            timing: self
                .stamp
                .filter(|p| p.hold == hold && now.saturating_duration_since(p.emitted) <= MAX_AGE)
                .map(|p| AcceptedGesture {
                    candidate: p.candidate,
                    accepted: now,
                    gap_ms: if hold {
                        None
                    } else {
                        p.candidate
                            .released
                            .map(|r| millis(p.emitted.saturating_duration_since(r)))
                    },
                }),
        }
    }
}
/// Decimal text avoids loss of u64 precision across the webview. The token
/// carries identity only; all timing stays native and is never reconstructed.
pub(crate) fn emit(candidate: Candidate, hold: bool) -> String {
    let now = Instant::now();
    let mut pending = PENDING.lock_or_recover();
    pending.retain(|p| now.saturating_duration_since(p.emitted) <= MAX_AGE);
    while pending.len() >= MAX_PENDING {
        pending.pop_front();
    }
    pending.push_back(Pending {
        candidate,
        emitted: now,
        hold,
    });
    candidate.token.to_string()
}
pub(crate) fn claim(token: Option<&str>) -> GestureClaim {
    let mut pending = PENDING.lock_or_recover();
    let now = Instant::now();
    pending.retain(|p| now.saturating_duration_since(p.emitted) <= MAX_AGE);
    let stamp = token.and_then(|t| t.parse::<u64>().ok()).and_then(|id| {
        pending
            .iter()
            .position(|p| p.candidate.token == id)
            .and_then(|index| pending.remove(index))
    });
    GestureClaim {
        expected: token.is_some(),
        stamp,
    }
}
pub(crate) fn clear_pending() {
    PENDING.lock_or_recover().clear();
}
pub(crate) fn millis(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_physical_press_and_release_to_toggle_gap() {
        clear_pending();
        let now = Instant::now();
        let candidate = Candidate {
            token: Candidate::new().token,
            pressed: now - Duration::from_millis(200),
            released: Some(now - Duration::from_millis(160)),
        };
        let token = emit(candidate, false);
        let g = claim(Some(&token)).accepted(false).timing.unwrap();
        assert_eq!(g.candidate.pressed, candidate.pressed);
        assert!(millis(g.accepted.duration_since(g.candidate.pressed)) >= 200);
        assert!(g.gap_ms.unwrap() >= 160);
        assert!(claim(Some(&token)).accepted(false).timing.is_none());
    }
    #[test]
    fn rejected_busy_transcribing_file_request_clears_its_stamp() {
        clear_pending();
        let token = emit(Candidate::new(), false);
        // Same entry operation, before the busy guard; the command then returns
        // busy_transcribing_file without accepting.
        let _ = claim(Some(&token));
        assert!(PENDING.lock_or_recover().is_empty());
        let toolbar = claim(None).accepted(false);
        assert!(!toolbar.expected);
        assert!(toolbar.timing.is_none());
        assert!(claim(Some(&token)).accepted(false).timing.is_none());
    }
    #[test]
    fn toolbar_racing_a_queued_keyboard_start_cannot_steal_its_stamp() {
        clear_pending();
        let candidate = Candidate::new();
        let token = emit(candidate, false); // keyboard event is queued in webview
        let toolbar = claim(None).accepted(false); // toolbar wins native admission
        assert!(!toolbar.expected);
        assert!(toolbar.timing.is_none());
        let _ = claim(Some(&token)); // now rejected as already starting
        assert!(PENDING.lock_or_recover().is_empty());
        assert!(claim(None).accepted(false).timing.is_none());
    }
    #[test]
    fn interleaved_requests_are_correlated_by_token_not_next_recording_id() {
        clear_pending();
        let first = Candidate::new();
        let second = Candidate::new();
        let a = emit(first, false);
        let b = emit(second, true);
        assert_eq!(
            claim(Some(&b))
                .accepted(true)
                .timing
                .unwrap()
                .candidate
                .token,
            second.token
        );
        assert_eq!(
            claim(Some(&a))
                .accepted(false)
                .timing
                .unwrap()
                .candidate
                .token,
            first.token
        );
    }
    #[test]
    fn wrong_origin_expired_and_evicted_tokens_are_unavailable() {
        clear_pending();
        let token = emit(Candidate::new(), true);
        let wrong_origin = claim(Some(&token)).accepted(false);
        assert!(wrong_origin.expected && wrong_origin.timing.is_none());
        let token = emit(Candidate::new(), false);
        PENDING.lock_or_recover().back_mut().unwrap().emitted -= MAX_AGE + Duration::from_millis(1);
        assert!(claim(Some(&token)).accepted(false).timing.is_none());
        let token = emit(Candidate::new(), false);
        for _ in 0..MAX_PENDING {
            emit(Candidate::new(), false);
        }
        assert_eq!(PENDING.lock_or_recover().len(), MAX_PENDING);
        assert!(claim(Some(&token)).accepted(false).timing.is_none());
        clear_pending();
    }
    #[test]
    fn hold_has_no_double_tap_gap() {
        clear_pending();
        let token = emit(Candidate::new(), true);
        assert_eq!(
            claim(Some(&token)).accepted(true).timing.unwrap().gap_ms,
            None
        );
    }
}
