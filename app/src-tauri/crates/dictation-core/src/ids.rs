//! Identifier newtypes (appendix C2.1, C1.4.1).
//!
//! The core never allocates ids. The shell draws recording ids through the
//! field adapter (`ID+`, C0.2) and operation ids from the process-global
//! `NEXT_OP` counter (C1.4.1 rule 2); these types only give them a shape.

/// A dictation recording id. `0` means "none yet" (C2.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecordingId(pub u64);

impl RecordingId {
    /// The "no recording" id.
    pub const NONE: Self = Self(0);

    /// `true` for the "none yet" id.
    pub const fn is_none(self) -> bool {
        self.0 == 0
    }

    /// The id the next allocation will return: a pure function of the core
    /// (`fetch_add(1) + 1`, `state.rs` `next_recording_id`; C0.2 `ID+`).
    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// An operation (continuation) id, allocated by the shell from the static
/// `NEXT_OP` counter, monotonic and never reused (C1.4.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationId(pub u64);

/// A microphone-preview id (`MicrophonePreviewState::claim`), carried by
/// [`crate::admission::Admission::Takeover`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PreviewId(pub u64);

/// Microseconds since a shell-fixed monotonic epoch. Time enters the core only
/// as this value; the core has no clock (C2.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Now(pub u64);

/// An opaque shell-owned payload (context snapshot, delivery target, smart
/// destination snapshot, ...). Effects carry these instead of the payload so the
/// core never sees them (C1.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Handle(pub u32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_is_current_plus_one() {
        assert_eq!(RecordingId::NONE.next(), RecordingId(1));
        assert_eq!(RecordingId(41).next(), RecordingId(42));
        assert!(RecordingId::NONE.is_none());
        assert!(!RecordingId(1).is_none());
    }

    #[test]
    fn next_wraps_like_fetch_add() {
        assert_eq!(RecordingId(u64::MAX).next(), RecordingId(0));
    }
}
