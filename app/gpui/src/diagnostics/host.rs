//! The production host (#994/#996/#997) supplies services, never a Tauri handle.
use super::{inbox::Inbox, model::*};
use murmur_core::events::Consumer;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
};
/// Drop stops every named subscription. Hosts must also stop producers when the
/// Inbox closes; failed publications are backpressure, never loggable payloads.
pub trait Subscription {
    fn stop(&mut self);
}
/// This interface deliberately has no arm/disarm, benchmark/microphone start,
/// internal recorder, transcript delivery, arbitrary command or store-write API.
pub trait Host {
    fn subscribe(&self, consumer: Consumer, attachment: u64, inbox: Inbox)
    -> Box<dyn Subscription>;
    /// Offload blocking work. Before a side effect, recheck lease on the service
    /// owner and revalidate its generation/clear epoch/UUID. Cancellation of a
    /// panel is Import(Ok(None)); errors are typed and content-free.
    fn request(&self, request: Request, inbox: Inbox);
}
pub struct Request {
    pub ticket: Ticket,
    pub lease: Lease,
    pub kind: RequestKind,
}
pub enum RequestKind {
    Read,
    CopyEvents(String),
    CopyLatency(String),
    ClearEvents,
    ClearPerformance,
    ClearLatency,
    Recover { allow_reinitialize: bool },
    ImportReport,
    ReadCapture { capture_id: String },
    DeleteCapture { capture_id: String },
    UploadCapture { capture_id: String },
}
#[derive(Default)]
pub(crate) struct Authority(Arc<Mutex<BTreeMap<Lane, Ticket>>>);
pub struct Lease {
    authority: Weak<Mutex<BTreeMap<Lane, Ticket>>>,
    ticket: Ticket,
}
impl Lease {
    pub fn is_current(&self) -> bool {
        self.authority.upgrade().is_some_and(|a| {
            a.lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&self.ticket.lane)
                == Some(&self.ticket)
        })
    }
}
impl Authority {
    pub fn sync(&self, model: &Model) {
        let mut a = self.0.lock().unwrap_or_else(|p| p.into_inner());
        a.clear();
        for lane in Lane::ALL {
            let t = model.ticket(lane);
            if model.current(t) {
                a.insert(lane, t);
            }
        }
    }
    pub fn revoke(&self) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }
    pub fn lease(&self, ticket: Ticket) -> Lease {
        Lease {
            authority: Arc::downgrade(&self.0),
            ticket,
        }
    }
}
