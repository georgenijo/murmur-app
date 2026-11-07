//! The Phase 2 host injects existing meeting/calendar/export owners, never a second store.
use super::{inbox::Inbox, model::*};
use crate::workspace::Lease;
use serde_json::{Value, json};
#[derive(Clone)]
pub enum Action {
    Read,
    List {
        query: String,
        limit: usize,
    },
    Select(String),
    Start,
    StartSuggested(String),
    Stop {
        generation: u64,
    },
    CheckAccess,
    AudioSettings,
    SaveMetadata {
        id: String,
        title: Option<String>,
        attendees: Vec<String>,
    },
    SaveLabels {
        id: String,
        revision: Option<u64>,
        me: String,
        them: String,
    },
    RenameSpeaker {
        id: String,
        speaker: u32,
        label: String,
    },
    CalendarLookup(String),
    CalendarApply {
        id: String,
        token: String,
    },
    CalendarSettings,
    CalendarReset,
    Copy {
        id: String,
        format: Format,
    },
    Export {
        id: String,
        format: Format,
    },
    BundlePreview(String),
    BundleExport {
        id: String,
        format: Format,
    },
    Delete(String),
    DeleteAll,
    Playback {
        id: String,
        action: PlaybackAction,
    },
    Back,
}
#[derive(Clone, Copy)]
pub enum PlaybackAction {
    Play,
    Pause,
    Seek(u64),
    Channel(Channel),
    Segment { channel: Channel, start_ms: u64 },
    Retry,
}
impl Action {
    /// Exact existing service command for host adapters. Compound operations use
    /// the mapping documented below; no UI accepts a path or Calendar identifier.
    pub fn command(&self) -> Option<(&'static str, Value)> {
        Some(match self {
            Self::Read => ("get_meeting_status", json!({})),
            Self::List { query, limit } => (
                "list_meetings",
                json!({"query":if query.is_empty(){None}else{Some(query)},"offset":0,"limit":limit.min(&100)}),
            ),
            Self::Select(id) => ("get_meeting", json!({"id":id})),
            Self::Start | Self::StartSuggested(_) => return None,
            Self::Stop { .. } => ("stop_meeting", json!({})),
            Self::CheckAccess => ("request_system_audio_permission", json!({})),
            Self::AudioSettings => ("open_system_audio_preferences", json!({})),
            Self::SaveMetadata {
                id,
                title,
                attendees,
            } => (
                "save_meeting_metadata",
                json!({"request":{"sessionId":id,"title":title,"attendees":attendees}}),
            ),
            Self::SaveLabels {
                id,
                revision,
                me,
                them,
            } => (
                "save_meeting_review",
                json!({"request":{"sessionId":id,"expectedReviewRevision":revision,"base":{"kind":"labels_only"},"labels":{"me":me,"them":them},"document":null}}),
            ),
            Self::RenameSpeaker { id, speaker, label } => (
                "rename_meeting_remote_speaker",
                json!({"sessionId":id,"speakerId":speaker,"label":label}),
            ),
            Self::CalendarLookup(id) => ("get_meeting_calendar_events", json!({"sessionId":id})),
            Self::CalendarApply { id, token } => (
                "apply_meeting_calendar_event",
                json!({"sessionId":id,"selectionToken":token}),
            ),
            Self::CalendarSettings => ("open_calendar_preferences", json!({})),
            Self::CalendarReset => ("reset_calendar_permission", json!({})),
            Self::Copy { id, format } => (
                "get_meeting_review_export",
                json!({"id":id,"format":format.wire()}),
            ),
            Self::Export { id, format } => (
                "save_meeting_review_export",
                json!({"id":id,"format":format.wire()}),
            ),
            Self::BundlePreview(id) => ("get_meeting_bundle_preview", json!({"id":id})),
            Self::BundleExport { id, format } => (
                "save_meeting_bundle_export",
                json!({"id":id,"format":format.wire()}),
            ),
            Self::Delete(id) => ("delete_meeting", json!({"id":id})),
            Self::DeleteAll => ("delete_all_meetings", json!({})),
            Self::Playback { .. } | Self::Back => return None,
        })
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
    /// Synchronously register targeted Main before passive Read: hydrate status,
    /// permission (no tap), first 50 sessions, selected detail and audio busy.
    fn subscribe(
        &self,
        consumer: murmur_core::events::Consumer,
        attachment: u64,
        inbox: Inbox,
    ) -> Box<dyn Subscription>;
    /// Offload blocking work. Check lease, exact session/revision and recording
    /// generation before EVERY continuation and side effect. Never reticket replies.
    /// Start calls start_meeting under native admission, resolving model/language/
    /// punctuation, microphone/smartAuto and the existing meetingRetainAudio,
    /// meetingRetentionDays (0 -> null), meetingMaxSessions,
    /// meetingEchoCancellationEnabled and meetingDiarization ONCE at admission.
    /// StartSuggested carries only the accepted opaque suggestion token into the
    /// same start_meeting request; native admission rechecks opt-in, expiry, app
    /// identity and busy ownership before consuming it. Mount never accepts.
    /// Stop checks the requested generation under the recording transition lock.
    /// List rebuilds from offset zero in <=100-row pages, retaining the loaded
    /// window atomically. Events include meeting status/segment/speakers-updated,
    /// audio invalidation and all native busy owners; no inference or text transforms
    /// live here. Capture/recovery/import/preview exclusion stays authoritative.
    /// CalendarLookup explicitly checks get_calendar_permission_status, requests
    /// request_calendar_permission only for notDetermined, then queries. Apply
    /// revalidates its opaque token in Rust. No calendar data survives picker close.
    /// Copy uses existing export renderer then injected native clipboard owner;
    /// Export/BundleExport obtain a native save panel path before calling the
    /// named atomic writer. Cancel is silent. BundlePreview discloses omissions
    /// and size before the panel. Captions require finished capture.
    /// Playback uses playback::Player with the validated manifest/range/busy
    /// service adapter; never Web Audio or paths. It pauses on busy/deletion/hide.
    /// Accepted durable writes survive hide; unaccepted effects and late replies
    /// do not. Window geometry is owned by Rust on the main thread.
    fn request(&self, request: Request, inbox: Inbox);
    /// Synchronous cancellation of only this presentation's native player. Must
    /// invalidate pending reads BEFORE stopping output. Never stops capture.
    fn pause_playback(&self);
    /// Release this presentation's ephemeral Calendar candidates on cancel/hide.
    fn discard_calendar(&self);
    /// Optional nonblocking service pump; production hosts normally publish events.
    fn tick(&self, _attachment: u64, _inbox: Inbox) {}
}
