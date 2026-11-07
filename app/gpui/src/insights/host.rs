pub use crate::section::{Host as SectionHost,Request as SectionRequest,Subscription};
pub type Host=dyn SectionHost<super::model::Snapshot,Action>;
pub type Request=SectionRequest<Action>;
#[derive(Clone,Debug,PartialEq,Eq)]pub enum Action{Read,Back}
impl Action{pub fn command(&self)->Option<(&'static str,serde_json::Value)>{match self{Self::Read=>Some(("get_completion_snapshot",serde_json::json!({}))),Self::Back=>None}}}
// Read the existing durable completion statistics, never recording events or
// History text. Host supplies a local calendar date (not UTC) and mode names.
// Reset remains the existing clear_completion_destination/statistics owner.
