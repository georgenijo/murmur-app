//! Existing native owners are injected at Phase 2; no alternate permission/store API.
use super::{inbox::Inbox, model::*};
pub use crate::workspace::Lease;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Read,
    Probe(Permission),
    Request(Permission),
    OpenSettings(Permission),
    Reset(Permission),
    Catalog,
    Download(String),
    CancelDownload(String),
    Complete {
        model: String,
        recording_mode: String,
        trigger_key: String,
    },
    ResumeExisting,
}
impl Action {
    pub fn command(&self) -> &'static str {
        match self {
            Self::Probe(p) => match p {
                Permission::Microphone => "check_microphone_permission_status",
                Permission::Accessibility => "check_accessibility_permission",
                Permission::SystemAudio => "get_system_audio_permission_status",
                Permission::Calendar => "get_calendar_permission_status",
            },
            Self::Request(p) => match p {
                Permission::Microphone => "request_microphone_access",
                Permission::Accessibility => "request_accessibility_permission",
                Permission::SystemAudio => "request_system_audio_permission",
                Permission::Calendar => "request_calendar_permission",
            },
            Self::OpenSettings(p) => match p {
                Permission::Microphone => "request_microphone_permission",
                Permission::Accessibility => "request_accessibility_permission",
                Permission::SystemAudio => "open_system_audio_preferences",
                Permission::Calendar => "open_calendar_preferences",
            },
            Self::Reset(p) => match p {
                Permission::Microphone => "reset_microphone_permission",
                Permission::Accessibility => "reset_accessibility_permission",
                Permission::Calendar => "reset_calendar_permission",
                Permission::SystemAudio => "open_system_audio_preferences",
            },
            Self::Catalog => "get_model_runtime_catalog",
            Self::Download(_) => "download_model",
            Self::CancelDownload(_) => "cancel_model_download",
            Self::Complete { .. } | Self::ResumeExisting => "resume_dictation_after_setup",
            Self::Read => "load_source_state",
        }
    }
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
    /// Register Main before Read. Passive TCC queries never open audio devices.
    /// Do not touch EventKit until the Calendar visit, except after explicit use.
    fn subscribe(
        &self,
        consumer: murmur_core::events::Consumer,
        attachment: u64,
        inbox: Inbox,
    ) -> Box<dyn Subscription>;
    /// Read only core permission facts on mount; catalog is passive for the gate.
    /// Recheck lease/revision and exact native generation before every effect.
    /// Probe replies carry their original visit ticket. Download progress additionally
    /// checks native request ID + monotonic attempt; cancellation confirms that exact
    /// child's termination before terminal Reply unlocks Back. Cancellation maps
    /// the attachment/model back to its originally retained native request ID,
    /// including retries after a newer source revision revoked the first lease. Hide/drop cancels only
    /// that attachment's download; it never cancels someone else's operation.
    /// Complete validates/reprobes installed model, serially configures the existing
    /// backend and resumes listener admission BEFORE saving settingsVersion=7,
    /// model/recordingMode/doubleTapKey/launchOnboardingComplete, then
    /// murmur_onboarding_complete=true and murmur_discovery_v1's fresh baseline.
    /// Failure must not mark setup complete. Accepted intent belongs to the durable
    /// owner across hide; its UI response remains revocable. ResumeExisting preserves
    /// saved choices and existing discovery, seeding grandfathered evidence if absent.
    /// Blocking work stays off the main thread. No payload/error text is logged.
    fn request(&self, request: Request, inbox: Inbox);
    fn detach(&self, attachment: u64);
}
