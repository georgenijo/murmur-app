use super::{host::*,model::*};
use crate::section;
use std::{cell::RefCell,rc::Rc};
pub fn snapshot()->Snapshot{Snapshot{revision:1,commands:[("recording-toggle","Start recording","Recording"),("transcribe-file","Transcribe audio or video file…","Recording"),("paste-last","Paste last or retry delivery","History"),("history-search","Search transcripts","History"),("settings-delivery","Settings: Delivery","Settings"),("settings-appearance","Settings: Appearance","Settings"),("settings-modes","Settings: Modes","Settings"),("check-updates","Check for updates","App")].into_iter().map(|(id,title,section)|Command{id:id.into(),title:title.into(),section:section.into(),keywords:vec!["preferences".into()],hint:None}).collect()}}
#[derive(Default)]pub struct FixtureHost{pub actions:RefCell<Vec<Action>>}
struct Stop;impl Subscription for Stop{fn stop(&mut self){}}
impl SectionHost<Snapshot,Action> for FixtureHost{
 fn subscribe(&self,_:murmur_core::events::Consumer,_:u64,_:crate::workspace::Inbox<Snapshot>)->Box<dyn Subscription>{Box::new(Stop)}
 fn request(&self,r:Request,inbox:crate::workspace::Inbox<Snapshot>){if !r.lease.is_current(){return}self.actions.borrow_mut().push(r.action);section::reply(&inbox,r.ticket,Ok(snapshot()));}
}
pub fn host()->Rc<Host>{Rc::new(FixtureHost::default())}
