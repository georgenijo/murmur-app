pub use crate::section::{Host as SectionHost, Request as SectionRequest, Subscription};
pub type Host = dyn SectionHost<super::model::Snapshot, Action>;
pub type Request = SectionRequest<Action>;
use super::model::*;
use serde_json::{Value, json};
#[derive(Clone)]
pub enum Action {
    List { filter: Filter, offset: usize },
    Retry,
    Upsert(Value),
    Enable { entry: Value, enabled: bool },
    Delete(Value),
    DeleteAll(u64),
    Collect { filter: Filter, action: BulkAction },
    Apply { bulk: Bulk, action: BulkAction },
    InspectImport,
    Import { preview: Value },
    Export,
    Cancel,
}
impl Action {
    pub fn command(&self) -> Option<(&'static str, Value)> {
        Some(match self {
            Self::List { filter, offset } => {
                ("list_knowledge", json!({"request":filter.wire(*offset)}))
            }
            Self::Retry => ("retry_knowledge_store", json!({})),
            Self::Upsert(draft) => ("upsert_knowledge", json!({"draft":draft})),
            Self::Enable { entry, enabled } => (
                "set_knowledge_enabled",
                json!({"id":entry["id"],"expectedRevision":entry["revision"],"enabled":enabled}),
            ),
            Self::Delete(v) => (
                "delete_knowledge",
                json!({"id":v["id"],"expectedRevision":v["revision"]}),
            ),
            Self::DeleteAll(revision) => {
                ("delete_all_knowledge", json!({"expectedRevision":revision}))
            }
            _ => return None,
        })
    }
}
// Native panels own file paths. InspectImport uses inspect_knowledge_import;
// Import uses import_knowledge_from_file only with the reviewed bounded file
// identity. Export uses export_knowledge_to_file. Panel cancel is a no-op.
// Collect reads ALL matching pages of 50 via Bulk::append before any writes.
// Apply invokes Bulk::commands sequentially, checks the lease before EVERY next
// write, skips unchanged entries, retains original revisions, never retries a
// conflict. Publish succeeded/unchanged/failure counts and failed IDs locally.
// Partial failures continue; leaving stops subsequent writes. Delete-all uses
// existing cleanup semantics. Subscribe includes knowledge-changed/status and
// Read also requests get_knowledge_store_status. Never log records or paths.
