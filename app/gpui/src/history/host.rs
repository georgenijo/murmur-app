//! Phase 2 injects existing completion, clipboard, save-panel and route owners.
use super::{inbox::Inbox, model::*};
use std::sync::{Arc,Mutex,Weak};
pub trait Subscription {fn stop(&mut self);}
pub trait Host {
    /// Register the Main consumer synchronously before returning, before reads.
    fn subscribe(&self,consumer:murmur_core::events::Consumer,attachment:u64,inbox:Inbox)->Box<dyn Subscription>;
    /// Offload blocking work. Revalidate lease AND exact target/revision/clear
    /// epoch on the owning service immediately before each side effect. Stop on
    /// terminal inbox closure. Never relabel a late reply with a new ticket.
    fn request(&self,request:Request,inbox:Inbox);
}
pub struct Request {pub ticket:Ticket,pub lease:Lease,pub kind:RequestKind}
pub enum RequestKind {
    Read,
    CopyRow {entry:Entry, markdown:bool},
    Export {entries:Vec<Entry>,format:Format,save:bool},
    Pin {entry:Entry},
    Delete {entries:Vec<Entry>,clear_all:bool},
    CorrectAndTeach {entry:Entry},
    TranscribeFile,
    RetryImport,
}
#[derive(Default)]
pub(crate) struct Authority(Arc<Mutex<Option<Ticket>>>);
pub struct Lease {authority:Weak<Mutex<Option<Ticket>>>,ticket:Ticket}
impl Lease {pub fn is_current(&self)->bool {self.authority.upgrade().is_some_and(|a|*a.lock().unwrap_or_else(|p|p.into_inner())==Some(self.ticket))}}
impl Authority {
    pub fn sync(&self,m:&Model) {*self.0.lock().unwrap_or_else(|p|p.into_inner())=m.current(m.ticket()).then_some(m.ticket());}
    pub fn revoke(&self) {*self.0.lock().unwrap_or_else(|p|p.into_inner())=None;}
    pub fn lease(&self,ticket:Ticket)->Lease {Lease{authority:Arc::downgrade(&self.0),ticket}}
}
