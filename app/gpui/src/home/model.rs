//! Home contains no transcript or recording owner. History retains its own contract.
pub use crate::workspace::{Event, Problem, Session, Ticket};
use crate::{
    onboarding::model::{Access, Permission},
    workspace::Projection,
};
use serde_json::Value;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Idle,
    Starting,
    Recording,
    Processing,
    Recovering,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    Home,
    Meetings,
    Insights,
    General,
    Palette,
    Modes,
    TeachLatest,
    Shortcuts,
    Setup,
}
pub const ITEMS: [(&str, &str, &str, &str); 5] = [
    (
        "command_palette",
        "Open the command palette",
        "Jump anywhere with ⌘K",
        "Open ⌘K",
    ),
    (
        "mode_binding",
        "Bind a mode to an app",
        "Give one app its own writing behavior",
        "Choose app",
    ),
    (
        "meeting",
        "Record a meeting",
        "Capture local Me and Them transcripts",
        "Open Notetaker",
    ),
    (
        "correction",
        "Teach a correction",
        "Fix the latest dictation and remember it",
        "Teach",
    ),
    (
        "shortcuts",
        "See keyboard shortcuts",
        "Keep every Murmur action close",
        "View",
    ),
];
pub fn checklist_destination(id: &str) -> Option<Destination> {
    Some(match id {
        "command_palette" => Destination::Palette,
        "mode_binding" => Destination::Modes,
        "meeting" => Destination::Meetings,
        "correction" => Destination::TeachLatest,
        "shortcuts" => Destination::Shortcuts,
        _ => return None,
    })
}
pub const HINTS: [(&str, &str, &str, &str); 3] = [
    (
        "double_tap_timing",
        "A double-tap arrived too slowly",
        "The two taps need to land close together. You can turn on visual timing feedback in Recording settings.",
        "Timing settings",
    ),
    (
        "correct_and_teach",
        "Fix a word once, then teach Murmur",
        "Correct & Teach on your latest dictation can remember the spelling for next time.",
        "Correct & Teach",
    ),
    (
        "browser_modes",
        "Use a mode for this site",
        "Murmur can apply a mode to an exact browser site without storing the page or its text.",
        "Set up site modes",
    ),
];
#[derive(Clone)]
pub struct Snapshot {
    pub revision: u64,
    pub settings: Value,
    pub discovery: Value,
    pub status: Status,
    pub initialized: bool,
    pub recording_generation: Option<u64>,
    pub recording_seconds: u64,
    pub audio_level: f32,
    pub meeting_busy: bool,
    pub words_this_month: u64,
    pub recordings_this_month: u64,
    pub permissions: [Access; 4],
    pub permissions_checked: bool,
    pub destination: Option<String>,
}
impl Snapshot {
    pub fn capturing(&self) -> bool {
        matches!(self.status, Status::Starting | Status::Recording)
    }
    pub fn waiting(&self) -> bool {
        matches!(self.status, Status::Processing | Status::Recovering)
            || self.meeting_busy
            || !self.initialized
    }
    pub fn title(&self) -> String {
        match self.status {
            Status::Starting => "Connecting to your microphone…".into(),
            Status::Recording => format!(
                "Listening · {}:{:02}",
                self.recording_seconds / 60,
                self.recording_seconds % 60
            ),
            Status::Processing => "Writing it out…".into(),
            Status::Recovering => "Reconnecting your microphone…".into(),
            Status::Idle if self.meeting_busy => "Notetaker is running".into(),
            Status::Idle if !self.initialized => "Getting ready…".into(),
            _ => "Click to start talking".into(),
        }
    }
    pub fn hint(&self) -> String {
        if matches!(self.status, Status::Recording | Status::Processing)
            && let Some(d) = &self.destination
        {
            return d.clone();
        }
        match self.status {
            Status::Starting => "Click to cancel".into(),
            Status::Recording => "Click to stop".into(),
            Status::Processing => "Your text will be copied when it’s ready".into(),
            Status::Recovering => "This can take a moment".into(),
            Status::Idle if self.meeting_busy => {
                "Dictation is available when the meeting ends".into()
            }
            Status::Idle if !self.initialized => "This takes a moment the first time".into(),
            _ => {
                let key = self.settings["doubleTapKey"]
                    .as_str()
                    .unwrap_or("ControlRight");
                let label = crate::settings::schema::ROWS
                    .iter()
                    .find(|r| r.key == "doubleTapKey")
                    .and_then(|r| {
                        if let crate::settings::schema::Kind::Choice(o) = r.kind {
                            o.iter().find(|(v, _)| *v == key).map(|(_, l)| *l)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(key);
                format!(
                    "or {} {label} in any app",
                    match self.settings["recordingMode"].as_str() {
                        Some("double_tap") => "double-tap",
                        Some("both") => "hold or double-tap",
                        _ => "hold",
                    }
                )
            }
        }
    }
    pub fn warning(&self) -> Option<(&'static str, Permission)> {
        if !self.permissions_checked {
            return None;
        }
        let mic = self.permissions[0] == Access::Denied;
        let ax = self.permissions[1] != Access::Granted;
        if self.permissions[2] == Access::Denied {
            Some((
                "System Audio access was denied; Meeting capture cannot start.",
                Permission::SystemAudio,
            ))
        } else if mic && ax {
            Some((
                "Microphone and Accessibility access are needed.",
                Permission::Microphone,
            ))
        } else if mic {
            Some(("Microphone access was revoked.", Permission::Microphone))
        } else if ax {
            Some((
                "Accessibility access is needed for auto-paste.",
                Permission::Accessibility,
            ))
        } else {
            None
        }
    }
    pub fn checklist(&self) -> bool {
        !self.discovery.is_null() && self.discovery["checklistDismissed"] == false
    }
    pub fn completed(&self, id: &str) -> bool {
        self.discovery["completed"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == id))
    }
}
impl Projection for Snapshot {
    fn revision(&self) -> u64 {
        self.revision
    }
    fn bytes(&self) -> usize {
        serde_json::to_vec(&self.settings)
            .map_or(usize::MAX, |v| v.len())
            .saturating_add(serde_json::to_vec(&self.discovery).map_or(usize::MAX, |v| v.len()))
            .saturating_add(1024)
    }
    fn valid(&self) -> bool {
        murmur_core::state::normalize("dictation-settings", self.settings.clone())
            .is_ok_and(|v| v == self.settings)
            && murmur_core::state::normalize("murmur_discovery_v1", self.discovery.clone())
                .is_ok_and(|v| v == self.discovery)
            && self.bytes()
                <= murmur_core::state::source_size_limit("dictation-settings")
                    + murmur_core::state::source_size_limit("murmur_discovery_v1")
                    + 1024
            && self.audio_level.is_finite()
            && (0. ..=1.).contains(&self.audio_level)
            && (!self.capturing() || self.recording_generation.is_some_and(|g| g > 0))
            && self
                .destination
                .as_ref()
                .is_none_or(|s| s.chars().count() <= 96 && !s.chars().any(char::is_control))
    }
}
