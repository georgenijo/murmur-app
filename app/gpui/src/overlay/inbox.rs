//! Bounded lifecycle queue plus one replaceable level sample. Publishers never
//! touch GPUI/NSWindow, wait for main, or serialize payloads into diagnostic text.
use super::model::Event;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
#[derive(Default, Clone)]
pub struct Inbox(Arc<Mutex<Pending>>);
#[derive(Default)]
struct Pending {
    events: VecDeque<Event>,
    level: Option<(u64, f32)>,
}
impl Inbox {
    /// Backpressure is explicit. A full lifecycle queue must be retried or
    /// followed by a fresh authoritative snapshot, never silently dropped.
    pub fn publish(&self, event: Event) -> Result<(), Box<Event>> {
        let mut pending = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if pending.events.len() >= 256 {
            return Err(Box::new(event));
        }
        pending.events.push_back(event);
        Ok(())
    }
    pub fn level(&self, generation: u64, level: f32) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).level = Some((generation, level));
    }
    pub(crate) fn drain(&self) -> (VecDeque<Event>, Option<(u64, f32)>) {
        let mut p = self.0.lock().unwrap_or_else(|p| p.into_inner());
        (std::mem::take(&mut p.events), p.level.take())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audio_coalesces_without_displacing_lifecycle_and_backpressure_is_explicit() {
        let inbox = Inbox::default();
        for _ in 0..256 {
            assert!(inbox.publish(Event::Visible(true)).is_ok());
        }
        assert!(inbox.publish(Event::Visible(false)).is_err());
        for i in 0..1000 {
            inbox.level(7, i as f32);
        }
        let (events, level) = inbox.drain();
        assert_eq!(events.len(), 256);
        assert_eq!(level, Some((7, 999.)));
        assert!(inbox.drain().0.is_empty());
        assert!(inbox.drain().1.is_none());
    }
}
