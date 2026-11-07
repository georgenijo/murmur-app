//! Registry comes from the main host's existing actions, not a second definition
//! of capture/history/update behavior. Rebuild labels from current runtime state:
//! starting=>Cancel microphone connection; recording=>Stop recording; otherwise
//! Start recording. Omit dictation when meeting-busy and copy/export when empty.
//! Next-recording rows use enabled backend Modes and preserve the pending token.
pub use crate::section::{Host as SectionHost, Request as SectionRequest, Subscription};
pub type Host = dyn SectionHost<super::model::Snapshot, Action>;
pub type Request = SectionRequest<Action>;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Read,
    Run(String),
}
// Run(id) invokes the registry callback only after verifying that it still
// exists in the current registry. Stop callbacks recheck the displayed recording
// generation; all normal service admission (including busy) remains in force.
