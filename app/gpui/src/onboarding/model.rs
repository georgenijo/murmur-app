//! Wizard projection. Existing core schemas and service owners remain authoritative.
use crate::workspace::Projection;
pub use crate::workspace::{Event, Problem, Session, Ticket};
use serde_json::Value;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Welcome,
    Microphone,
    Accessibility,
    SystemAudio,
    Calendar,
    Model,
    Hotkey,
    Done,
}
impl Step {
    pub const ALL: [Self; 8] = [
        Self::Welcome,
        Self::Microphone,
        Self::Accessibility,
        Self::SystemAudio,
        Self::Calendar,
        Self::Model,
        Self::Hotkey,
        Self::Done,
    ];
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap()
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Welcome => "Welcome to Murmur",
            Self::Microphone => "Microphone access",
            Self::Accessibility => "Accessibility access",
            Self::SystemAudio => "System audio access",
            Self::Calendar => "Calendar access",
            Self::Model => "Transcription model",
            Self::Hotkey => "Recording shortcut",
            Self::Done => "You're all set",
        }
    }
    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Welcome => {
                "Dictation runs on your Mac. Saved history and retained meeting audio stay local."
            }
            Self::Microphone => {
                "Murmur records your speech for local transcription. Audio is discarded after transcription."
            }
            Self::Accessibility => {
                "Use the global recording key and auto-paste while Murmur is in the background."
            }
            Self::SystemAudio => {
                "Optional for meetings. Capture audio from your Mac alongside your microphone."
            }
            Self::Calendar => {
                "Optional for meetings. Offer event names and attendees for you to confirm, without changing your calendar."
            }
            Self::Model => "Download a local model once, then transcribe offline.",
            Self::Hotkey => {
                "Choose how Murmur listens. You can change both options later in Recording settings."
            }
            Self::Done => "Here's how everything looks.",
        }
    }
    pub fn permission(self) -> Option<Permission> {
        match self {
            Self::Microphone => Some(Permission::Microphone),
            Self::Accessibility => Some(Permission::Accessibility),
            Self::SystemAudio => Some(Permission::SystemAudio),
            Self::Calendar => Some(Permission::Calendar),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permission {
    Microphone,
    Accessibility,
    SystemAudio,
    Calendar,
}
impl Permission {
    pub fn index(self) -> usize {
        self as usize
    }
    pub fn label(self) -> &'static str {
        ["Microphone", "Accessibility", "System audio", "Calendar"][self.index()]
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Access {
    #[default]
    Unknown,
    NotDetermined,
    Granted,
    Denied,
    Restricted,
    Unsupported,
}
impl Access {
    pub fn label(self) -> &'static str {
        match self {
            Self::Granted => "Allowed",
            Self::Denied => "Denied",
            Self::Restricted => "Restricted",
            Self::Unsupported => "Unavailable",
            _ => "Not set",
        }
    }
}
#[derive(Clone, Debug)]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    pub size: String,
    pub detail: String,
    pub installed: bool,
}
pub const CURATED: [&str; 4] = [
    "parakeet-tdt-0.6b-v3-coreml",
    "parakeet-tdt-0.6b-v2-fp16",
    "large-v3-turbo",
    "base.en",
];
#[derive(Clone)]
pub struct Snapshot {
    pub revision: u64,
    pub settings: Value,
    pub complete: bool,
    pub permissions: [Access; 4],
    pub catalog: Option<Vec<ModelInfo>>,
    pub catalog_error: bool,
    pub download_percent: Option<u8>,
    pub installing: bool,
    pub needs_relaunch: bool,
    pub recommended: Option<String>,
    pub warned_models: Vec<String>,
    pub hardware_summary: Option<String>,
}
impl Snapshot {
    pub fn installed(&self, id: &str) -> bool {
        self.catalog
            .as_ref()
            .is_some_and(|c| c.iter().any(|m| m.id == id && m.installed))
    }
    pub fn preferred(&self) -> String {
        let initial = self.settings["model"].as_str().unwrap_or(CURATED[0]);
        if self.installed(initial) {
            return initial.into();
        }
        self.catalog
            .as_ref()
            .and_then(|c| c.iter().find(|m| m.installed))
            .map(|m| m.id.clone())
            .unwrap_or_else(|| {
                if CURATED.contains(&initial) {
                    initial.into()
                } else {
                    CURATED[0].into()
                }
            })
    }
    pub fn grandfathered(&self) -> bool {
        self.permissions[0] == Access::Granted
            && self.permissions[1] == Access::Granted
            && self
                .catalog
                .as_ref()
                .is_some_and(|c| c.iter().any(|m| m.installed))
    }
}
impl Projection for Snapshot {
    fn revision(&self) -> u64 {
        self.revision
    }
    fn bytes(&self) -> usize {
        serde_json::to_vec(&self.settings)
            .map_or(usize::MAX, |v| v.len())
            .saturating_add(16384)
    }
    fn valid(&self) -> bool {
        let known = |id: &str| {
            crate::settings::schema::ROWS.iter().find(|r|r.key == "model").is_some_and(|r| matches!(r.kind, crate::settings::schema::Kind::Choice(options) if options.iter().any(|(v,_)| *v == id)))
        };
        murmur_core::state::normalize("dictation-settings", self.settings.clone())
            .is_ok_and(|v| v == self.settings)
            && self.bytes() <= murmur_core::state::source_size_limit("dictation-settings") + 16384
            && self.download_percent.is_none_or(|p| p <= 100)
            && self.catalog.as_ref().is_none_or(|c| {
                c.len() <= 7
                    && c.iter().enumerate().all(|(i, m)| {
                        known(&m.id)
                            && !c[..i].iter().any(|p| p.id == m.id)
                            && [&m.label, &m.size, &m.detail]
                                .iter()
                                .all(|s| s.len() <= 512 && !s.contains('\0'))
                    })
            })
            && self.recommended.as_ref().is_none_or(|id| known(id))
            && self.warned_models.len() <= 7
            && self.warned_models.iter().all(|id| known(id))
            && self
                .hardware_summary
                .as_ref()
                .is_none_or(|s| s.len() <= 512)
    }
}
