//! Only synthetic text and in-memory fake side effects. Never production I/O.
use super::{host::*,inbox::Inbox,model::*};
use serde_json::json;
use std::cell::{Cell,RefCell};
pub const NOW:i64=1_799_000_000_000;
pub fn entry(n:u64)->Entry {
    Entry::project(n,json!({"schemaVersion":2,"id":format!("synthetic-{n}"),"pinned":n%40==0,"text":if n==200 {"Synthetic release notes for the violet lighthouse. ".repeat(150)} else {format!("Synthetic transcript {n}: violet lighthouse release notes.")},
        "rawText":format!("synthetic raw recognition {n}"),"timestamp":NOW-(200_i64-n as i64)*3_600_000,"duration":12.,"source":if n%3==0 {"file"} else {"recording"},"sourceName":if n%3==0 {"synthetic-lighthouse.wav"} else {""},
        "teachingContext":{"bundleId":"synthetic.private.scope","projectRoot":"/synthetic/private"}})).unwrap()
}
pub fn snapshot()->Snapshot {Snapshot{entries:(1..=200).map(entry).collect(),retain_history:true,retry_import:false}}
pub fn clock()->Clock {Clock{now:NOW,today:NOW-12*3_600_000,week:NOW-156*3_600_000,month:NOW-708*3_600_000}}
pub struct FixtureHost {
    pub rows:RefCell<Vec<Entry>>,revision:Cell<u64>,epoch:Cell<u64>,
    pub clipboard:RefCell<String>,pub saved:RefCell<Vec<(Format,String)>>,
    pub calls:RefCell<Vec<&'static str>>,pub stops:std::rc::Rc<Cell<usize>>,
    pub cancel_save:Cell<bool>,
}
impl Default for FixtureHost {fn default()->Self {Self{rows:RefCell::new(snapshot().entries),revision:Cell::new(0),epoch:Cell::new(0),clipboard:RefCell::new(String::new()),saved:RefCell::new(vec![]),calls:RefCell::new(vec![]),stops:Default::default(),cancel_save:Cell::new(false)}}}
struct Stop(std::rc::Rc<Cell<usize>>);
impl Subscription for Stop {fn stop(&mut self) {self.0.set(self.0.get()+1);}}
impl Host for FixtureHost {
    fn subscribe(&self,c:murmur_core::events::Consumer,_:u64,_:Inbox)->Box<dyn Subscription> {assert_eq!(c,murmur_core::events::Consumer::Main);self.calls.borrow_mut().push("subscribe");Box::new(Stop(self.stops.clone()))}
    fn request(&self,r:Request,inbox:Inbox) {
        if !r.lease.is_current()||inbox.is_closed() {return;}
        self.calls.borrow_mut().push("request");
        let read=matches!(&r.kind,RequestKind::Read);
        let mut changed=false;let mut outcome=Outcome::Opened;
        match r.kind {
            RequestKind::Read=>(),
            RequestKind::CopyRow{entry,markdown}=>{
                *self.clipboard.borrow_mut()=if markdown {export(&[&entry],Format::Markdown,"2027-01-03 12:00:00",|_|"2027-01-03 12:00:00".into()).unwrap()} else {entry.text};outcome=Outcome::Copied;
            }
            RequestKind::Export{entries,format,save}=>{
                if save&&self.cancel_save.get() {outcome=Outcome::Cancelled;}
                else {let refs=entries.iter().collect::<Vec<_>>();if let Some(text)=export(&refs,format,"2027-01-03T12:00:00Z",|_|"2027-01-03 12:00:00".into()) {
                    if save {self.saved.borrow_mut().push((format,text));outcome=Outcome::Saved;} else {*self.clipboard.borrow_mut()=text;outcome=Outcome::Copied;}
                } else {outcome=Outcome::Failed;}}
            }
            RequestKind::Pin{entry}=>{if let Some(e)=self.rows.borrow_mut().iter_mut().find(|e|e.key==entry.key) {let mut target=e.target();target["pinned"]=json!(!e.pinned);*e=Entry::project(e.key,target).unwrap();changed=true;}}
            RequestKind::Delete{entries,clear_all}=>{
                if clear_all {self.rows.borrow_mut().clear();self.epoch.set(self.epoch.get()+1);} else {self.rows.borrow_mut().retain(|e|!entries.iter().any(|t|t.key==e.key));}changed=true;
            }
            RequestKind::CorrectAndTeach{..}|RequestKind::TranscribeFile|RequestKind::RetryImport=>(),
        }
        self.revision.set(self.revision.get()+1);
        let event=if changed {Event::Changed{attachment:r.ticket.attachment,revision:self.revision.get(),clear_epoch:self.epoch.get(),snapshot:Snapshot{entries:self.rows.borrow().clone(),retain_history:true,retry_import:false}}}
        else if read {Event::Read{ticket:r.ticket,revision:self.revision.get(),clear_epoch:self.epoch.get(),result:Ok(Snapshot{entries:self.rows.borrow().clone(),retain_history:true,retry_import:false})}}
        else {Event::Action{ticket:r.ticket,outcome}};
        assert!(inbox.publish(event).is_ok());
    }
}
