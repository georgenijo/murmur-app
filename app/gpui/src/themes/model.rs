pub use crate::workspace::{Event, Problem, Session, Ticket};
use crate::{section, workspace::Projection};
use serde_json::Value;
#[derive(Clone)]
pub struct Snapshot {
    pub revision: u64,
    pub appearance: Value,
    pub library: Value,
    pub library_available: bool,
    pub results: Vec<super::open_vsx::Extension>,
    pub import_preview: Option<Vec<Value>>,
    pub notice: Option<String>,
}
impl Projection for Snapshot {
    fn revision(&self) -> u64 {
        self.revision
    }
    fn bytes(&self) -> usize {
        section::bytes(&self.appearance)
            .saturating_add(section::bytes(&self.library))
            .saturating_add(
                self.import_preview
                    .as_ref()
                    .map_or(0, |v| section::bytes(&serde_json::json!(v))),
            )
            .saturating_add(32768)
    }
    fn valid(&self) -> bool {
        murmur_core::state::normalize("murmur-appearance", self.appearance.clone())
            .is_ok_and(|s| s == self.appearance)
            && murmur_core::state::normalize("murmur-theme-library", self.library.clone())
                .is_ok_and(|s| s == self.library)
            && self.results.len() <= 8
            && self.results.iter().all(|e| e.valid())
            && self.bytes() <= 3 * 1024 * 1024
            && self.notice.as_ref().is_none_or(|s| section::text(s, 512))
    }
}
