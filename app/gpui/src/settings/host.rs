//! Phase 2 supplies these existing owners; this crate creates no second service.
use super::{inbox::Inbox, model::*, schema::Page};
use serde_json::Value;
use std::sync::{Arc, Mutex, Weak};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Vocabulary,
    Aliases,
    Knowledge,
    VoiceCommands,
    AppOverrides,
    Modes,
    Appearance,
    ProjectScan,
    ModelStorage,
    Diagnostics,
    Setup,
    Checklist,
    UpdateDetails,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    Start,
    Stop,
    Success,
    Failure,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Open(Destination),
    ChooseFolder(&'static str),
    CalendarSettings,
    LoginSettings,
    Accessibility,
    CheckUpdate,
    DownloadUpdate,
    RestartUpdate,
    InstallSmartTurn,
    RemoveSmartTurn,
    InstallSpeakers,
    RemoveSpeakers,
    PreviewCue(Cue),
    ResetStatistics,
    RunLab { models: Vec<String> },
    CancelLab,
    ExportReport(usize),
    ClearReports,
    MicrophoneStartup,
    CancelMicrophoneStartup,
    RetryMicrophone,
    RetrySmartAutoProbe,
    Calibration { offset: i64, finish: bool },
    CancelCalibration,
}
pub enum RequestKind {
    Read,
    /// One durable intent: set the stable device ID, finish ID migration and
    /// disable Smart Auto together, including when reselecting the saved ID.
    ManualMicrophone {
        id: String,
    },
    Patch {
        key: &'static str,
        value: Value,
    },
    Disclosure {
        page: Page,
        open: bool,
    },
    Action(Action),
}
pub struct Request {
    pub ticket: Ticket,
    pub lease: Lease,
    pub kind: RequestKind,
}
pub trait Subscription {
    fn stop(&mut self);
}
pub trait Host {
    /// Register targeted Main consumer BEFORE initial read. Never start capture,
    /// calibration, downloads, permission prompts or benchmarks during subscription.
    fn subscribe(
        &self,
        consumer: murmur_core::events::Consumer,
        attachment: u64,
        inbox: Inbox,
    ) -> Box<dyn Subscription>;
    /// Preference/disclosure requests admit synchronously while lease/revision are
    /// current, then the EXISTING owner retains that accepted intent until durable
    /// publication (including close/hide). A UI lease never cancels an accepted
    /// preference. ManualMicrophone is an atomic preference intent too.
    /// Reads, panels and preview actions remain revocable until effect.
    /// Background service work. Under SOURCE_WRITE_LOCK recover/read current
    /// sources, compare accepted intent and native recording ownership,
    /// merge only the requested field (or the ManualMicrophone fields together),
    /// validate and publish via core save_source.
    /// Complete launch-at-login/shortcut registration before publishing; failure
    /// leaves the previous working value selected. Never log private values.
    /// Emit current source/runtime snapshot on success; errors are typed/content-free.
    fn request(&self, request: Request, inbox: Inbox);
    /// A revocable page lifetime controls meter subscription/preview only. Entering
    /// Recording may monitor the exact saved device, yield to dictation and resume;
    /// leaving/hiding/drop must stop it and discard PCM. No monitor persistence.
    /// Calibration cancellation restores the durable position. NSWindow work is
    /// queued on the main thread. Recheck preview leases in async continuations,
    /// including on the main thread immediately before applying geometry. A new
    /// CancelCalibration request revokes any outstanding preview lease and must
    /// restore the durable position even if an earlier preview already moved it.
    /// No global input, OS settings or pasteboard writes.
    fn page(&self, page: Option<Page>, attachment: u64);
}
#[derive(Default)]
pub(crate) struct Authority(Arc<Mutex<Option<Ticket>>>);
pub struct Lease {
    authority: Weak<Mutex<Option<Ticket>>>,
    ticket: Ticket,
}
impl Lease {
    pub fn is_current(&self) -> bool {
        self.authority
            .upgrade()
            .is_some_and(|a| *a.lock().unwrap_or_else(|p| p.into_inner()) == Some(self.ticket))
    }
}
impl Authority {
    pub fn sync(&self, m: &Model) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = m.attached.then_some(m.ticket());
    }
    pub fn revoke(&self) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
    pub fn lease(&self, ticket: Ticket) -> Lease {
        Lease {
            authority: Arc::downgrade(&self.0),
            ticket,
        }
    }
}
