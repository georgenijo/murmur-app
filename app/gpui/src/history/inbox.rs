//! Ordered terminal inbox. Oversized payloads/backpressure never format text.
use super::model::*;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
#[derive(Clone, Default)]
pub struct Inbox(Arc<Mutex<Pending>>);
#[derive(Default)]
struct Pending {
    queue: VecDeque<(Event, usize)>,
    bytes: usize,
    closed: bool,
}
impl Inbox {
    pub fn publish(&self, e: Event) -> Result<(), Box<Event>> {
        let snapshot = match &e {
            Event::Read { result: Ok(s), .. } | Event::Changed { snapshot: s, .. } => Some(s),
            _ => None,
        };
        let bytes = if let Some(s) = snapshot {
            if !valid(s) {
                return Err(Box::new(e));
            }
            s.entries.iter().map(Entry::retained_bytes).sum::<usize>()
        } else {
            128
        };
        let mut p = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if p.closed || p.queue.len() >= 8 || p.bytes.saturating_add(bytes) > 2 * MAX_SNAPSHOT_BYTES
        {
            return Err(Box::new(e));
        }
        p.bytes += bytes;
        p.queue.push_back((e, bytes));
        Ok(())
    }
    pub fn is_closed(&self) -> bool {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).closed
    }
    pub(crate) fn close(&self) {
        let mut p = self.0.lock().unwrap_or_else(|p| p.into_inner());
        p.closed = true;
        p.queue.clear();
        p.bytes = 0;
    }
    pub(crate) fn drain(&self) -> VecDeque<Event> {
        let mut p = self.0.lock().unwrap_or_else(|p| p.into_inner());
        p.bytes = 0;
        std::mem::take(&mut p.queue)
            .into_iter()
            .map(|(e, _)| e)
            .collect()
    }
}
