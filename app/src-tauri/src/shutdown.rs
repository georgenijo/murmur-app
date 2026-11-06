//! Process-wide admission latch; the host adapter owns shutdown effects.
use std::sync::atomic::{AtomicBool, Ordering};
static CLOSING: AtomicBool = AtomicBool::new(false);
pub(crate) fn closing() -> bool {
    CLOSING.load(Ordering::Acquire)
}
pub(crate) fn close_admission() {
    CLOSING.store(true, Ordering::Release);
}
pub(crate) fn require_running() -> Result<(), String> {
    if closing() {
        Err("Murmur is shutting down".into())
    } else {
        Ok(())
    }
}
