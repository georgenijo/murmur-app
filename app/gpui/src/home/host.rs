//! The host routes to existing services; this view never opens capture/pasteboard.
use super::{inbox::Inbox, model::*};
use crate::{onboarding::model::Permission, workspace::Lease};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Read,
    Start,
    Stop { generation: u64 },
    Open(Destination),
    DismissChecklist,
    Checklist(&'static str),
    RecheckPermissions,
    Hint { id: &'static str, open: bool },
    PermissionSettings(Permission),
    ResetPermission(Permission),
}
pub struct Request {
    pub ticket: Ticket,
    pub lease: Lease,
    pub action: Action,
}
pub trait Subscription {
    fn stop(&mut self);
}
pub trait Host {
    /// Subscribe targeted Main before Read, hydrate current recording ownership.
    fn subscribe(
        &self,
        consumer: murmur_core::events::Consumer,
        attachment: u64,
        inbox: Inbox,
    ) -> Box<dyn Subscription>;
    /// Recheck lease and revision before admission. Start/Stop route to the native
    /// recording controller, freeze configuration once at Start and stop only the
    /// requested generation. Every async continuation rechecks that generation.
    /// Native meeting/import/recovery exclusion applies independently of UI state.
    /// DismissChecklist merges only checklistDismissed in murmur_discovery_v1;
    /// Checklist routes to checklist_destination. Only palette/shortcuts explicit
    /// visits complete immediately; meeting/correction/mode-binding require real
    /// evidence, using the existing baseline/reset semantics, never a click.
    /// Hint applies the existing dismissal once, preserving all baselines, and
    /// optionally routes timing to Recording/hotkey-feedback, teaching to newest
    /// History, and browser modes to Modes. New hints come from targeted events
    /// and content-free owner evidence, never from this view or a polling guess.
    /// Preference intent survives presentation hide after synchronous admission.
    /// Permissions use the onboarding command mapping; unknown microphone is not
    /// denial. No automatic prompts or recording on mount. Opening editors keeps
    /// the originating control; call destination_closed on dismissal for focus.
    /// Existing Rust owners dispatch NSWindow mutations on the main thread.
    fn request(&self, request: Request, inbox: Inbox);
}
