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
/// Passive updater state for the Home header indicator, projected from the real
/// updater snapshot. Copy matches the shipped `UpdateIndicator`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Update {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(String),
    Preparing,
    Downloading(Option<u8>),
    Ready(String),
    Restarting,
    Failed {
        check: bool,
    },
}
/// What a click on the indicator asks the updater owner to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateIntent {
    Open,
    Check,
    Restart,
}
impl Update {
    pub fn from_status(status: &murmur_core::updater::Status) -> Self {
        use murmur_core::updater::Status;
        // Versions come from a signed feed but are still bounded for display.
        let version = |v: &str| {
            v.chars()
                .filter(|c| !c.is_control())
                .take(64)
                .collect::<String>()
        };
        match status {
            Status::Idle => Self::Idle,
            Status::Checking => Self::Checking,
            Status::UpToDate => Self::UpToDate,
            Status::Available { version: v, .. } => Self::Available(version(v)),
            Status::Preparing { .. } => Self::Preparing,
            Status::Downloading { progress, .. } => {
                Self::Downloading(progress.map(|p| p.clamp(0., 100.).round() as u8))
            }
            Status::Ready { version: v, .. } => Self::Ready(version(v)),
            Status::Restarting { .. } => Self::Restarting,
            Status::Error { stage, .. } => Self::Failed {
                check: stage == "check",
            },
        }
    }
    /// Project the `updater_snapshot` command result. An unreadable snapshot is a
    /// quiet Idle: the indicator is passive and must not fail the whole Home read.
    pub fn from_wire(snapshot: &Value) -> Self {
        serde_json::from_value::<murmur_core::updater::Snapshot>(snapshot.clone())
            .map_or(Self::Idle, |s| Self::from_status(&s.update_status))
    }
    pub fn busy(&self) -> bool {
        matches!(
            self,
            Self::Checking | Self::Preparing | Self::Downloading(_) | Self::Restarting
        )
    }
    pub fn failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
    /// Phases with indefinite activity that rotate their glyph, as the shipped
    /// indicator spins for checking, preparing and restarting.
    pub fn spins(&self) -> bool {
        matches!(self, Self::Checking | Self::Preparing | Self::Restarting)
    }
    /// Download bar fill, 0..=1; None for any other phase and for a download
    /// whose total is not yet known (see `indeterminate`).
    pub fn progress(&self) -> Option<f32> {
        match self {
            Self::Downloading(Some(p)) => Some(f32::from((*p).min(100)) / 100.),
            _ => None,
        }
    }
    /// A download without a known percentage shows a travelling segment.
    pub fn indeterminate(&self) -> bool {
        matches!(self, Self::Downloading(None))
    }
    pub fn label(&self) -> String {
        match self {
            Self::Available(_) => "View update".into(),
            Self::Ready(_) => "Restart now".into(),
            Self::Checking => "Checking for updates…".into(),
            Self::Preparing => "Preparing update…".into(),
            Self::Downloading(None) => "Downloading update…".into(),
            Self::Downloading(Some(p)) => format!("Downloading update… {p}%"),
            Self::Restarting => "Restarting Murmur…".into(),
            Self::Failed { check: true } => "Retry update check".into(),
            Self::Failed { check: false } => "Update needs attention".into(),
            Self::Idle | Self::UpToDate => "Check for updates".into(),
        }
    }
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::Available(v) => Some(format!("Murmur {v} is available")),
            Self::Ready(v) => Some(format!("Murmur {v} is ready to install")),
            Self::UpToDate => Some("You’re up to date.".into()),
            Self::Failed { check: true } => Some("Couldn’t check for updates. Try again.".into()),
            Self::Failed { check: false } => Some("Open the update dialog to continue.".into()),
            _ => None,
        }
    }
    /// Accessible name and tooltip: "label. detail".
    pub fn summary(&self) -> String {
        match self.detail() {
            Some(detail) => format!("{}. {detail}", self.label()),
            None => self.label(),
        }
    }
    /// Text for the polite live region: busy label, up-to-date, or failure detail.
    pub fn announcement(&self) -> String {
        if self.busy() {
            self.label()
        } else if matches!(self, Self::UpToDate) || self.failed() {
            self.detail().unwrap_or_default()
        } else {
            String::new()
        }
    }
    /// None while busy: clicks are ignored, as in the shipped indicator.
    pub fn intent(&self) -> Option<UpdateIntent> {
        match self {
            _ if self.busy() => None,
            Self::Available(_) | Self::Failed { check: false } => Some(UpdateIntent::Open),
            Self::Ready(_) => Some(UpdateIntent::Restart),
            _ => Some(UpdateIntent::Check),
        }
    }
}
#[derive(Clone)]
pub struct Snapshot {
    pub update: Update,
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
