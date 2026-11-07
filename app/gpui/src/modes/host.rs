pub use crate::section::{Host as SectionHost, Request as SectionRequest, Subscription};
pub type Host = dyn SectionHost<super::model::Snapshot, Action>;
pub type Request = SectionRequest<Action>;
use serde_json::{Value, json};
#[derive(Clone)]
pub enum Action {
    Read,
    Patch(Value),
    Probe,
    AddApp { mode_id: String },
    InspectImport,
    Import(super::exchange::Preview),
    Export,
}
impl Action {
    pub fn command(&self) -> Option<(&'static str, Value)> {
        match self {
            Self::Probe => Some(("probe_browser_site", json!({}))),
            _ => None,
        }
    }
}
// Patch is a single existing dictation-settings intent for modes/appProfiles/
// siteModeLookupEnabled/browserSiteRules; compare the captured revision and
// preserve every other setting and profile fine-tuning/root. Do not mutate an
// in-flight DictationContextSnapshot. AddApp uses Settings' existing app editor
// and returns to this selected Mode; navigation cancels that return.
// InspectImport opens a native panel and read_modes_file, then exchange::parse
// and preview against current settings. Keep bytes in memory for confirmation;
// import uses exchange::commit after rereading. Export uses exchange::export
// and save_text_export with a native save panel. No clipboard or browser probing
// during Read/subscribe; Probe is explicit and bounded by its existing owner.
