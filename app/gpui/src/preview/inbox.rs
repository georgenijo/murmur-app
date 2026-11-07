//! Bounded, ordered transport. Only consecutive partials of the same owner
//! coalesce, so no text update can jump a consent/lifecycle barrier.
use super::model::{Event, MAX_PARTIAL_UNITS};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
#[derive(Default, Clone)]
pub struct Inbox(Arc<Mutex<Pending>>);
#[derive(Default)]
struct Pending {
    queue: VecDeque<Event>,
    closed: bool,
}
impl Inbox {
    /// Caller must retry lifecycle backpressure. Never format rejected events.
    pub fn publish(&self, event: Event) -> Result<(), Box<Event>> {
        if matches!(&event, Event::Partial {text, ..} if text.encode_utf16().count() > MAX_PARTIAL_UNITS)
        {
            return Err(Box::new(event));
        }
        if matches!(&event, Event::AuthoritativePartial {stable, tentative, ..}
            if stable.encode_utf16().count() + tentative.encode_utf16().count() > MAX_PARTIAL_UNITS)
        {
            return Err(Box::new(event));
        }
        let mut pending = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if pending.closed {
            return Err(Box::new(event));
        }
        let queue = &mut pending.queue;
        if let (
            Some(Event::Partial {
                generation: old, ..
            }),
            Event::Partial { generation, .. },
        ) = (queue.back(), &event)
            && old == generation
        {
            queue.pop_back();
        }
        if queue.len() >= 256 {
            return Err(Box::new(event));
        }
        queue.push_back(event);
        Ok(())
    }
    /// A dropped consumer is terminal; do not retry rejected events on it.
    pub fn is_closed(&self) -> bool {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).closed
    }
    pub(crate) fn close(&self) {
        let mut pending = self.0.lock().unwrap_or_else(|p| p.into_inner());
        pending.closed = true;
        pending.queue.clear();
    }
    pub(crate) fn pending(&self) -> bool {
        !self
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .queue
            .is_empty()
    }
    pub(crate) fn drain(&self) -> VecDeque<Event> {
        std::mem::take(&mut self.0.lock().unwrap_or_else(|p| p.into_inner()).queue)
    }
}
