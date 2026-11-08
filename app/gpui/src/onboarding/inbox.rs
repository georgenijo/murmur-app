//! Bounded updates plus one reserved terminal slot for the newest operation.
use super::model::{Problem, Snapshot, Ticket};
use crate::workspace::{Event, Projection};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
pub struct Inbox(Arc<Mutex<Pending>>);
#[derive(Default)]
struct Pending {
    updates: crate::workspace::Inbox<Snapshot>,
    terminal: Option<(Ticket, Result<Snapshot, Problem>)>,
}
impl Inbox {
    pub fn publish(&self, event: Event<Snapshot>) -> Result<(), Box<Event<Snapshot>>> {
        let mut pending = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if pending.updates.is_closed() {
            return Err(Box::new(event));
        }
        if let Event::Reply { ticket, result } = event {
            if result.as_ref().is_ok_and(|snapshot| !snapshot.valid()) {
                return Err(Box::new(Event::Reply { ticket, result }));
            }
            // An attachment has only one pending operation. Cancellation/retry
            // advances its ticket; a late old reply cannot displace that owner.
            if pending.terminal.as_ref().is_none_or(|(queued, _)| {
                (ticket.attachment, ticket.generation, ticket.revision)
                    >= (queued.attachment, queued.generation, queued.revision)
            }) {
                pending.terminal = Some((ticket, result));
            }
            Ok(())
        } else {
            pending.updates.publish(event)
        }
    }
    pub fn is_closed(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .updates
            .is_closed()
    }
    pub(crate) fn close(&self) {
        let mut pending = self.0.lock().unwrap_or_else(|p| p.into_inner());
        pending.updates.close();
        pending.terminal = None;
    }
    pub(crate) fn drain(&self) -> VecDeque<Event<Snapshot>> {
        let mut pending = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let mut events = pending.updates.drain();
        if let Some((ticket, result)) = pending.terminal.take() {
            events.push_back(Event::Reply { ticket, result });
        }
        events
    }
}

pub fn reply(inbox: &Inbox, ticket: Ticket, result: Result<Snapshot, Problem>) {
    if inbox.publish(Event::Reply { ticket, result }).is_err() {
        // Invalid projections become a recoverable terminal error. A closed
        // attachment has no view to recover and deliberately rejects both.
        let _ = inbox.publish(Event::Reply {
            ticket,
            result: Err(Problem::Invalid),
        });
    }
}
