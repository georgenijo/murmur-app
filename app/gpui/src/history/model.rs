//! Private-text state. Deliberately no Debug or serialization implementations.
pub use murmur_core::history::*;
use std::{collections::BTreeSet, time::{Duration, Instant}};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket { pub attachment:u64, pub epoch:u64, pub revision:u64, pub clear_epoch:u64 }
pub struct Snapshot { pub entries:Vec<Entry>, pub retain_history:bool, pub retry_import:bool }
pub enum Event {
    Read {ticket:Ticket, revision:u64, clear_epoch:u64, result:Result<Snapshot,Problem>},
    Changed {attachment:u64, revision:u64, clear_epoch:u64, snapshot:Snapshot},
    Action {ticket:Ticket, outcome:Outcome},
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Problem { Unavailable, Invalid, NewerSchema }
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Outcome { Copied, Saved, Cancelled, Opened, Failed }
impl Outcome { pub fn message(self)->Option<&'static str> {match self {
    Self::Copied=>Some("Copied"),Self::Saved=>Some("Export saved"),Self::Cancelled=>None,
    Self::Opened=>None,Self::Failed=>Some("History action failed. Try again."),
}}}
#[derive(Clone, PartialEq, Eq)]
pub enum DeleteScope { Row(u64), Shown, All }
pub struct Confirmation {pub scope:DeleteScope, pub ticket:Ticket, pub deadline:Instant, pub keys:Vec<u64>}
#[derive(Default)]
pub struct Model {
    pub entries:Vec<Entry>, pub query:Query, pub clock:Clock, pub matches:Vec<usize>,
    pub selected:Option<u64>, pub detail:Option<u64>, pub retain_history:bool,
    pub retry_import:bool, pub loading:bool, pub problem:Option<Problem>,
    pub notice:Option<&'static str>, pub confirmation:Option<Confirmation>,
    attachment:u64, epoch:u64, revision:u64, clear_epoch:u64, attached:bool,
}
impl Model {
    pub fn attach(&mut self) {self.attachment+=1;self.epoch+=1;self.attached=true;self.loading=true;}
    pub fn detach(&mut self) {self.invalidate();self.attached=false;self.entries=vec![];self.matches=vec![];self.selected=None;self.detail=None;self.query=Query::default();self.notice=None;}
    pub fn ticket(&self)->Ticket {Ticket{attachment:self.attachment,epoch:self.epoch,revision:self.revision,clear_epoch:self.clear_epoch}}
    pub fn current(&self,t:Ticket)->bool {self.attached && t==self.ticket()}
    pub fn invalidate(&mut self) {self.epoch+=1;self.confirmation=None;self.notice=None;}
    pub fn begin(&mut self)->Ticket {self.invalidate();self.ticket()}
    pub fn refresh(&mut self) {
        self.matches=self.query.matching(&self.entries,self.clock);
        if !self.matches.iter().any(|&i|Some(self.entries[i].key)==self.selected) {self.selected=self.matches.first().map(|&i|self.entries[i].key);}
        if !self.entries.iter().any(|e|Some(e.key)==self.detail) {self.detail=None;}
    }
    pub fn set_query(&mut self,q:Query) {if self.query!=q {self.invalidate();self.query=q;self.refresh();}}
    pub fn set_clock(&mut self,clock:Clock) {
        let before=self.matches.clone();self.clock=clock;self.refresh();
        if before!=self.matches {self.invalidate();}
    }
    pub fn selected_entry(&self)->Option<&Entry> {self.entries.iter().find(|e|Some(e.key)==self.selected)}
    pub fn event(&mut self,event:Event)->bool {
        match event {
            Event::Action{ticket,outcome} => {if !self.current(ticket) {return false;} self.notice=outcome.message();true}
            Event::Read{ticket,revision,clear_epoch,result} => {
                if !self.current(ticket)||revision<=self.revision||clear_epoch<self.clear_epoch {return false;}
                match result {Ok(s)=>self.replace(revision,clear_epoch,s),Err(p)=>{self.invalidate();self.revision=revision;self.problem=Some(p);self.loading=false;true}}
            }
            Event::Changed{attachment,revision,clear_epoch,snapshot} => {
                if !self.attached||attachment!=self.attachment||revision<=self.revision||clear_epoch<self.clear_epoch {return false;}
                self.replace(revision,clear_epoch,snapshot)
            }
        }
    }
    fn replace(&mut self,revision:u64,clear_epoch:u64,snapshot:Snapshot)->bool {
        if !valid(&snapshot) {self.invalidate();self.problem=Some(Problem::Invalid);self.loading=false;return false;}
        self.invalidate();self.revision=revision;self.clear_epoch=clear_epoch;self.entries=snapshot.entries;
        self.retain_history=snapshot.retain_history;self.retry_import=snapshot.retry_import;self.loading=false;self.problem=None;self.refresh();true
    }
    pub fn arm(&mut self,scope:DeleteScope,now:Instant)->bool {
        if self.confirmation.as_ref().is_some_and(|c| c.scope==scope && c.ticket==self.ticket() && now<c.deadline) {return true;}
        let keys=match scope {DeleteScope::Row(k)=>vec![k],DeleteScope::Shown=>self.matches.iter().map(|&i|self.entries[i].key).collect(),DeleteScope::All=>self.entries.iter().map(|e|e.key).collect()};
        if keys.is_empty() {return false;}
        self.confirmation=Some(Confirmation{scope,ticket:self.ticket(),deadline:now+Duration::from_secs(4),keys});false
    }
    pub fn expire(&mut self,now:Instant) {if self.confirmation.as_ref().is_some_and(|c|now>=c.deadline) {self.confirmation=None;}}
}
pub fn valid(s:&Snapshot)->bool {
    s.entries.len()<=MAX_ENTRIES && s.entries.iter().map(|e|e.key).collect::<BTreeSet<_>>().len()==s.entries.len()
        && s.entries.iter().map(Entry::retained_bytes).sum::<usize>()<=MAX_SNAPSHOT_BYTES
}
