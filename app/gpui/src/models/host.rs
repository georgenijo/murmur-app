pub use crate::section::{Host as SectionHost, Request as SectionRequest, Subscription};
pub type Host = dyn SectionHost<super::model::Snapshot, Action>;
pub type Request = SectionRequest<Action>;
use serde_json::{Value, json};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Read,
    Select { id: String, settings_revision: u64 },
    Download(String),
    Remove(String),
    Cancel { model: String, request_id: String },
}
impl Action {
    pub fn command(&self) -> Option<(&'static str, Value)> {
        Some(match self {
            Self::Read => ("get_model_runtime_catalog", json!({})),
            Self::Select { .. } => return None,
            Self::Download(id) => ("download_model", json!({"modelName":id})),
            Self::Remove(id) => ("remove_model", json!({"modelName":id})),
            Self::Cancel { model, request_id } => (
                "cancel_model_download",
                json!({"modelName":model,"requestId":request_id}),
            ),
        })
    }
}
// Select uses the existing atomic dictation-settings/model intent and runtime
// configure owner; do not auto-select on install or change the default. Read
// combines the runtime catalog + get_model_hardware_info guidance. Subscribe
// before Read and merge catalog rows by their native generation. Downloads are
// only explicit, correlated by native request/attempt ID, not model ID alone.
// Remove repeats selected/lifecycle/busy checks at the existing runtime owner.
