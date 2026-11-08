//! Terminal projection failures must reach the view instead of leaving Read pending.
use super::model::{Problem, Snapshot, Ticket};
use crate::workspace::{Event, Projection};
pub type Inbox = crate::workspace::Inbox<Snapshot>;

pub fn reply(inbox: &Inbox, ticket: Ticket, result: Result<Snapshot, Problem>) {
    let result = result.and_then(|snapshot| {
        if snapshot.valid() {
            Ok(snapshot)
        } else {
            Err(Problem::Invalid)
        }
    });
    let _ = inbox.publish(Event::Reply { ticket, result });
}
