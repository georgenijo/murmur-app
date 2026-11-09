pub use crate::workspace::{Event, Problem, Session, Ticket};
use crate::{section, workspace::Projection};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Install {
    Missing,
    Installing,
    Validating,
    Installed,
    Invalid,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifecycle {
    Unloaded,
    Loading,
    Warming,
    Ready,
    Unloading,
    Failed,
}
#[derive(Clone, Debug)]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    pub size: String,
    pub backend: String,
    pub accelerator: String,
    pub generation: u64,
    pub install: Install,
    pub lifecycle: Lifecycle,
    pub supported: bool,
    pub failure: bool,
    pub capabilities: Vec<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Download,
    Remove,
}
/// The last failed download or removal, explained by the service owner with a
/// static, content-free message. It stays until the next operation starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub model: String,
    pub operation: Operation,
    pub message: &'static str,
    /// Core ML failures offer the explicit Whisper Base / CPU Parakeet choices.
    pub fallbacks: bool,
}
#[derive(Clone)]
pub struct Snapshot {
    pub revision: u64,
    /// Settings owner revision captured with selected model, independent of runtime events.
    pub settings_revision: u64,
    pub selected: String,
    pub catalog: Vec<ModelInfo>,
    pub busy: bool,
    pub attempt: Option<String>,
    pub attempt_model: Option<String>,
    pub percent: Option<u8>,
    pub phase: Option<String>,
    pub recommended: Option<String>,
    pub warned: Vec<String>,
    pub hardware: Option<String>,
    pub failure: Option<Failure>,
}
impl Snapshot {
    /// The language choice depends on the selected model's capability, never
    /// on its name. An unknown model is treated as English-only.
    pub fn english_only(&self) -> bool {
        self.model(&self.selected)
            .is_none_or(|m| !m.capabilities.iter().any(|c| c == "multilingual"))
    }
    pub fn select(&self, id: String) -> super::host::Action {
        super::host::Action::Select {
            id,
            settings_revision: self.settings_revision,
        }
    }
    pub fn merge_runtime(&mut self, update: ModelInfo) -> bool {
        if !known(&update.id) {
            return false;
        }
        if let Some(current) = self.catalog.iter_mut().find(|m| m.id == update.id) {
            if current.generation >= update.generation {
                return false;
            }
            *current = update;
            true
        } else if self.catalog.len() < 7 {
            self.catalog.push(update);
            true
        } else {
            false
        }
    }

    pub fn model(&self, id: &str) -> Option<&ModelInfo> {
        self.catalog.iter().find(|m| m.id == id)
    }
    pub fn can_remove(&self, m: &ModelInfo) -> bool {
        !self.busy
            && self.attempt.is_none()
            && m.id != self.selected
            && m.install == Install::Installed
            && matches!(m.lifecycle, Lifecycle::Unloaded | Lifecycle::Failed)
    }
    pub fn can_download(&self, m: &ModelInfo) -> bool {
        !self.busy
            && self.attempt.is_none()
            && m.supported
            && !matches!(
                m.install,
                Install::Installed | Install::Installing | Install::Validating
            )
    }
}
pub fn known(id: &str) -> bool {
    crate::settings::schema::ROWS.iter().find(|r|r.key=="model").is_some_and(|r|matches!(r.kind,crate::settings::schema::Kind::Choice(o) if o.iter().any(|(v,_)|*v==id)))
}
impl Projection for Snapshot {
    fn revision(&self) -> u64 {
        self.revision
    }
    fn bytes(&self) -> usize {
        16384
    }
    fn valid(&self) -> bool {
        known(&self.selected)
            && self.catalog.len() <= 7
            && self.catalog.iter().enumerate().all(|(i, m)| {
                known(&m.id)
                    && !self.catalog[..i].iter().any(|p| p.id == m.id)
                    && [&m.label, &m.size, &m.backend, &m.accelerator]
                        .iter()
                        .all(|v| section::text(v, 256))
                    && m.capabilities.len() <= 7
                    && m.capabilities.iter().all(|s| section::text(s, 64))
            })
            && self.attempt_model.as_ref().is_none_or(|id| known(id))
            && self.percent.is_none_or(|p| p <= 100)
            && self.phase.as_ref().is_none_or(|s| section::text(s, 128))
            && self.attempt.as_ref().is_none_or(|s| section::text(s, 128))
            && self.hardware.as_ref().is_none_or(|s| section::text(s, 512))
            && self.recommended.as_ref().is_none_or(|id| known(id))
            && self
                .failure
                .as_ref()
                .is_none_or(|f| known(&f.model) && section::text(f.message, 512))
            && self.warned.len() <= 7
            && self.warned.iter().all(|id| known(id))
    }
}
