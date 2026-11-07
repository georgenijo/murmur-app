pub use crate::section::{Host as SectionHost, Request as SectionRequest, Subscription};
pub type Host = dyn SectionHost<super::model::Snapshot, Action>;
pub type Request = SectionRequest<Action>;
use super::open_vsx::Extension;
use serde_json::Value;
#[derive(Clone)]
pub enum Action {
    Read,
    Appearance(Value),
    Install(Vec<Value>),
    Remove(Vec<String>),
    Duplicate(String),
    ExportCurrent,
    OpenSource(String),
    ExportEntry(String),
    InspectLocal,
    ConfirmImport(Vec<Value>),
    Search {
        query: String,
        sort: String,
    },
    Add(Extension),
    Update {
        extension: Extension,
        expected: Vec<Value>,
    },
    Cancel,
}
// Read uses existing murmur-appearance and murmur-theme-library sources. Writes
// compare source revisions under SOURCE_WRITE_LOCK and save_source; preserve
// unknown fields and rebuild derived caches from authoritative theme config.
// Theme library unavailable never resets active appearance. Removal leaves the
// compiled active half intact and sets its selection ownership to custom.
// Existing theme-library.json migration/durability remains the host's owner.
// Appearance calls publish one appearance-changed revision and apply the native
// Appearance owner only after persistence. Native window chrome uses GPUI's
// main-thread API. OS changes are read-only, never persistence/events.
// InspectLocal and Export use native panels + read_theme_file and
// write_theme_file. Compile::compile creates portable cache tables. Imported
// bytes stay in the preview until confirmation/cancel. No clipboard access.
// Search/Add call open_vsx on a blocking worker only after explicit action;
// recheck lease after every response and immediately before publishing/committing.
// Add previews entries; Update requires confirmation and library::install with
// expected collection identity. No auto-updates/polling or script execution.
