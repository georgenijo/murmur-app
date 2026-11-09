//! Revisioned projection only. No capture, clipboard, permission or store owner.
use super::schema::{Page, Row};
use serde_json::Value;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub attachment: u64,
    pub generation: u64,
    pub revision: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    Unavailable,
    Invalid,
    Conflict,
    Denied,
}
impl Problem {
    pub fn message(self) -> &'static str {
        match self {
            Self::Unavailable => {
                "Settings could not be saved. Your previous choices are unchanged. Retry when the service is available."
            }
            Self::Invalid => "Enter a valid value in the displayed range.",
            Self::Conflict => {
                "Settings changed elsewhere. Reload and review your choice before saving again."
            }
            Self::Denied => {
                "That action is currently unavailable. Check the permission or stop the active recording."
            }
        }
    }
}
#[derive(Clone)]
pub struct Device {
    pub id: String,
    pub label: String,
    pub available: bool,
    pub continuity: bool,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Install {
    #[default]
    Unknown,
    Missing,
    Installed,
    Busy,
    Failed,
}
impl Install {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Checking availability…",
            Self::Missing => "Not installed",
            Self::Installed => "Installed locally",
            Self::Busy => "Working…",
            Self::Failed => "Installation failed — retry available",
        }
    }
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Update {
    #[default]
    Idle,
    Checking,
    Current,
    Available,
    Downloading,
    Ready,
    Failed,
}
impl Update {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Check for a newer Murmur release",
            Self::Checking => "Checking for updates…",
            Self::Current => "Murmur is up to date",
            Self::Available => "An update is available",
            Self::Downloading => "Downloading update…",
            Self::Ready => "Update ready to restart",
            Self::Failed => "Update check failed. Retry is available.",
        }
    }
}
#[derive(Clone, Copy)]
pub struct Calibration {
    pub requested: i64,
    pub applied: i64,
    pub min: i64,
    pub max: i64,
}
/// Live microphone test feedback owned by the preview service.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct MicTest {
    /// `Some(slow)` while the input is being claimed; `slow` once it exceeds
    /// the service's still-connecting threshold.
    pub connecting: Option<bool>,
    pub live: bool,
    /// Failure explanation. `retry` is true when no capture is owned, so a new
    /// test may safely claim the input.
    pub error: Option<String>,
    pub retry: bool,
    pub vad_unavailable: bool,
}
/// Smart Auto selection status. Inactive covers off and ready: like the
/// shipped pane, a ready selection needs no feedback.
#[derive(Clone, Default, PartialEq, Eq)]
pub enum SmartAuto {
    #[default]
    Inactive,
    Probing {
        device: String,
        phase: &'static str,
    },
    Blocked {
        message: String,
        cooldown_secs: Option<u32>,
        /// A background check may be requested now.
        retry: bool,
        /// The last request for one was refused.
        retry_failed: bool,
    },
    Unavailable,
}
/// Microphone startup diagnostic: progress while running, then its report.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Startup {
    pub progress: Option<(String, u8)>,
    pub cancelling: bool,
    pub error: Option<String>,
    pub summary: Vec<String>,
}
#[derive(Clone, Default)]
pub struct Runtime {
    pub calibration: Option<Calibration>,
    pub login_requires_approval: bool,
    pub login_enabled: Option<bool>,
    pub recording: bool,
    pub internal_build: bool,
    pub delivery_isolated: bool,
    pub english_only: bool,
    pub notchpill: Option<bool>,
    pub speakers_supported: Option<bool>,
    pub accessibility: Option<bool>,
    pub devices: Vec<Device>,
    pub inventory_stale: bool,
    pub meter: Option<(f32, f32)>,
    pub monitor_paused: bool,
    pub mic: MicTest,
    pub smart_auto: SmartAuto,
    pub startup: Startup,
    pub smart_turn: Install,
    pub speakers: Install,
    pub model_install: Install,
    pub update: Update,
    pub update_percent: Option<u8>,
    pub installed_models: Option<Vec<String>>,
    pub startup_running: bool,
    pub lab_running: bool,
    pub lab_progress: Option<u8>,
    pub reports: Vec<String>,
    pub version: String,
}
#[derive(Clone)]
pub struct Snapshot {
    pub revision: u64,
    pub settings: Value,
    pub disclosure: Value,
    pub runtime: Runtime,
}
pub fn valid(s: &Snapshot) -> bool {
    let r = &s.runtime;
    let sources = murmur_core::state::settings(s.settings.clone()).is_ok_and(|v| v == s.settings)
        && murmur_core::state::normalize("murmur.settings-disclosure.v1", s.disclosure.clone())
            .is_ok()
        && serde_json::to_vec(&s.settings)
            .is_ok_and(|v| v.len() <= murmur_core::state::source_size_limit("dictation-settings"));
    let disclosure = serde_json::to_vec(&s.disclosure).is_ok_and(|v| v.len() <= 2048)
        && s.disclosure["panes"].as_object().is_some_and(|p| {
            p.keys().all(|k| {
                Page::NAV
                    .into_iter()
                    .chain([Page::Transcription, Page::Shortcuts])
                    .any(|page| page.id() == k)
                    || k == "app-overrides"
            })
        });
    let devices = r.devices.len() <= 32
        && r.devices.iter().all(|d| {
            !d.id.is_empty()
                && d.id.len() <= 4096
                && !d.id.contains('\0')
                && !d.label.is_empty()
                && d.label.chars().count() <= 256
                && !d.label.contains('\0')
        })
        && r.devices
            .iter()
            .enumerate()
            .all(|(i, d)| !r.devices[..i].iter().any(|p| p.id == d.id));
    let models = r.installed_models.as_ref().is_none_or(|models| models.len() <= 7 && models.iter().all(|id| {
        super::schema::ROWS.iter().find(|r| r.key == "model").is_some_and(|r| matches!(r.kind, super::schema::Kind::Choice(options) if options.iter().any(|(v,_)| *v == id)))
    }));
    sources
        && r.calibration.is_none_or(|c| {
            (-12..=12).contains(&c.requested)
                && (-12..=12).contains(&c.min)
                && (-12..=12).contains(&c.max)
                && c.min <= c.max
                && (c.min..=c.max).contains(&c.applied)
        })
        && disclosure
        && devices
        && models
        && r.meter.is_none_or(|(r, p)| {
            r.is_finite() && p.is_finite() && (0. ..=1.).contains(&r) && (0. ..=1.).contains(&p)
        })
        && r.update_percent.is_none_or(|p| p <= 100)
        && r.lab_progress.is_none_or(|p| p <= 100)
        && r.mic
            .error
            .as_ref()
            .is_none_or(|s| s.chars().count() <= 512)
        && match &r.smart_auto {
            SmartAuto::Blocked { message, .. } => message.chars().count() <= 1024,
            SmartAuto::Probing { device, .. } => device.chars().count() <= 256,
            _ => true,
        }
        && r.startup
            .progress
            .as_ref()
            .is_none_or(|(label, percent)| label.chars().count() <= 256 && *percent <= 100)
        && r.startup
            .error
            .as_ref()
            .is_none_or(|s| s.chars().count() <= 512)
        && r.startup.summary.len() <= 32
        && r.startup.summary.iter().all(|s| s.chars().count() <= 512)
        && r.reports.len() <= 10
        && r.reports.iter().all(|s| s.chars().count() <= 128)
        && r.version.len() <= 64
}

pub enum Event {
    Runtime {
        attachment: u64,
        runtime: Box<Runtime>,
    },
    Changed {
        attachment: u64,
        snapshot: Snapshot,
    },
    Reply {
        ticket: Ticket,
        result: Result<Snapshot, Problem>,
    },
}
pub struct Model {
    pub page: Page,
    pub snapshot: Option<Snapshot>,
    pub problem: Option<Problem>,
    pub pending: bool,
    pub query: String,
    pub attached: bool,
    pub generation: u64,
    pub attachment: u64,
    pub revision: u64,
}
impl Default for Model {
    fn default() -> Self {
        Self {
            page: Page::General,
            snapshot: None,
            problem: None,
            pending: false,
            query: String::new(),
            attached: false,
            generation: 0,
            attachment: 0,
            revision: 0,
        }
    }
}
impl Model {
    pub fn ticket(&self) -> Ticket {
        Ticket {
            attachment: self.attachment,
            generation: self.generation,
            revision: self.revision,
        }
    }
    pub fn attach(&mut self) {
        self.attachment += 1;
        self.generation += 1;
        self.attached = true;
        self.page = Page::General;
        self.revision = 0;
    }
    pub fn detach(&mut self) {
        self.attached = false;
        self.generation += 1;
        self.snapshot = None;
        self.query.clear();
        self.pending = false;
        self.problem = None;
    }
    pub fn settings(&self) -> &Value {
        static DEFAULT: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        self.snapshot
            .as_ref()
            .map(|s| &s.settings)
            .unwrap_or_else(|| DEFAULT.get_or_init(murmur_core::state::settings_defaults))
    }
    pub fn runtime(&self) -> Runtime {
        self.snapshot
            .as_ref()
            .map(|s| s.runtime.clone())
            .unwrap_or_default()
    }
    pub fn advanced(&self) -> bool {
        self.snapshot
            .as_ref()
            .is_some_and(|s| s.disclosure["panes"][self.page.id()] == true)
    }
    pub fn disabled(&self, row: Row) -> bool {
        let r = self.runtime();
        let s = self.settings();
        self.pending
            || self.snapshot.is_none()
            || row.capture_locked && r.recording
            || matches!(
                row.key,
                "autoPaste" | "autoPasteDelayMs" | "smartDestinationEnabled"
            ) && (s["saveAudio"] == true || s["saveTranscript"] == true)
            || matches!(
                row.key,
                "autoPaste" | "autoPasteDelayMs" | "smartDestinationEnabled"
            ) && r.delivery_isolated
            || row.key == "language" && r.english_only
            || row.key == "smartAutoStopEnabled"
                && s["smartAutoStopEnabled"] != true
                && (s["autoStopSilenceMs"] == 0 || r.smart_turn != Install::Installed)
    }
    pub fn begin(&mut self) -> Option<Ticket> {
        if !self.attached || self.pending {
            return None;
        }
        self.generation += 1;
        self.pending = true;
        self.problem = None;
        Some(self.ticket())
    }
    pub fn event(&mut self, e: Event) -> bool {
        if let Event::Runtime {
            attachment,
            runtime,
        } = e
        {
            if !self.attached || attachment != self.attachment {
                return false;
            }
            if let Some(snapshot) = self.snapshot.as_mut() {
                let candidate = Snapshot {
                    revision: snapshot.revision,
                    settings: snapshot.settings.clone(),
                    disclosure: snapshot.disclosure.clone(),
                    runtime: *runtime,
                };
                if valid(&candidate) {
                    snapshot.runtime = candidate.runtime;
                    return true;
                }
            }
            return false;
        }
        if !self.attached {
            return false;
        }
        let (snapshot, reply) = match e {
            Event::Runtime { .. } => return false,
            Event::Changed {
                attachment,
                snapshot,
            } => {
                if attachment != self.attachment || snapshot.revision <= self.revision {
                    return false;
                }
                (snapshot, false)
            }
            Event::Reply { ticket, result } => {
                if !self.pending || ticket != self.ticket() {
                    return false;
                }
                match result {
                    Ok(s) => (s, true),
                    Err(p) => {
                        self.problem = Some(p);
                        self.pending = false;
                        return true;
                    }
                }
            }
        };
        if snapshot.revision < self.revision {
            return false;
        }
        if !valid(&snapshot) {
            self.problem = Some(Problem::Invalid);
            if reply {
                self.pending = false;
            }
            return true;
        }
        self.revision = snapshot.revision;
        self.snapshot = Some(snapshot);
        self.problem = None;
        if reply {
            self.pending = false;
        } else if self.pending {
            self.generation += 1;
            self.pending = false;
            self.problem = Some(Problem::Conflict);
        }
        true
    }
}
