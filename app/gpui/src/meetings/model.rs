//! Private meeting presentation. No Debug or serialization of transcript payloads.
use crate::workspace::Projection;
pub use crate::workspace::{Event, Problem, Session as Workspace, Ticket};
use std::collections::BTreeSet;
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Starting,
    Recording,
    Stopping,
    Processing,
    Failed,
}
impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Ready for a meeting",
            Self::Starting => "Connecting both channels…",
            Self::Recording => "Recording meeting",
            Self::Stopping => "Stopping capture…",
            Self::Processing => "Finishing pending transcript chunks…",
            Self::Failed => "Meeting stopped",
        }
    }
    pub fn active(self) -> bool {
        matches!(self, Self::Starting | Self::Recording | Self::Stopping)
    }
    pub fn busy(self) -> bool {
        self.active() || self == Self::Processing
    }
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Permission {
    #[default]
    Unknown,
    Granted,
    Denied,
    Unsupported,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Channel {
    #[default]
    All,
    Me,
    Them,
}
impl Channel {
    pub fn wire(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Me => "me",
            Self::Them => "them",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "Both channels",
            Self::Me => "Me only",
            Self::Them => "Them only",
        }
    }
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Markdown,
    Text,
    Json,
    Srt,
    Vtt,
}
impl Format {
    pub const ALL: [Self; 5] = [Self::Markdown, Self::Text, Self::Json, Self::Srt, Self::Vtt];
    pub fn wire(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Text => "text",
            Self::Json => "json",
            Self::Srt => "srt",
            Self::Vtt => "vtt",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Markdown => "Markdown",
            Self::Text => "Plain text",
            Self::Json => "JSON",
            Self::Srt => "SubRip (.srt)",
            Self::Vtt => "WebVTT (.vtt)",
        }
    }
    pub fn captions(self) -> bool {
        matches!(self, Self::Srt | Self::Vtt)
    }
}
#[derive(Clone)]
pub struct Meeting {
    pub id: String,
    pub title: Option<String>,
    pub title_source: Option<String>,
    pub attendees: Vec<String>,
    pub started_at_ms: i64,
    pub duration_ms: u64,
    pub status: String,
    pub retain_audio: bool,
    pub segment_count: usize,
    pub preview: String,
    pub search_matches: Vec<String>,
}
#[derive(Clone)]
pub struct Segment {
    pub id: u64,
    pub session_id: String,
    pub channel: Channel,
    pub remote_speaker: Option<u32>,
    pub start_ms: u64,
    pub end_ms: u64,
    pub status: String,
    pub text: String,
    pub audio_available: bool,
}
#[derive(Clone)]
pub struct RemoteSpeaker {
    pub id: u32,
    pub label: String,
}
#[derive(Clone)]
pub struct Detail {
    pub session: Meeting,
    pub segments: Vec<Segment>,
    pub me: String,
    pub them: String,
    pub remote: Vec<RemoteSpeaker>,
    pub review_revision: Option<u64>,
}
impl Detail {
    pub fn label(&self, s: &Segment) -> String {
        if s.channel == Channel::Me {
            self.me.clone()
        } else {
            s.remote_speaker
                .and_then(|id| self.remote.iter().find(|r| r.id == id))
                .map(|r| r.label.clone())
                .unwrap_or_else(|| self.them.clone())
        }
    }
    pub fn finished(&self) -> bool {
        self.session.status != "active"
    }
}
#[derive(Clone)]
pub struct Candidate {
    pub token: String,
    pub title: String,
    pub attendees: Vec<String>,
    pub start_ms: i64,
    pub end_ms: i64,
}
#[derive(Clone, Default)]
pub enum Calendar {
    #[default]
    Closed,
    Choosing(Vec<Candidate>),
    Unavailable {
        message: &'static str,
        denied: bool,
        unsupported: bool,
    },
}
#[derive(Clone)]
pub struct Bundle {
    pub audio_segments: u64,
    pub audio_bytes: u64,
    pub unavailable: u64,
    pub max_bytes: u64,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum AudioState {
    #[default]
    Loading,
    Ready,
    Playing,
    Paused,
    Buffering,
    NotRetained,
    NotFinished,
    NoAudio,
    Error,
}
#[derive(Clone, Default)]
pub struct Audio {
    pub state: AudioState,
    pub duration_ms: u64,
    pub position_ms: u64,
    pub channel: Channel,
}
#[derive(Clone)]
pub struct Snapshot {
    pub revision: u64,
    pub phase: Phase,
    pub recording_generation: u64,
    pub recording_session: Option<String>,
    pub elapsed_ms: u64,
    pub microphone_active: bool,
    pub system_audio_active: bool,
    pub capture_busy: bool,
    pub permission: Permission,
    pub needs_relaunch: bool,
    pub failure: Option<&'static str>,
    pub echo_notice: Option<String>,
    pub meetings: Vec<Meeting>,
    pub query: String,
    pub total: usize,
    pub detail: Option<Detail>,
    pub calendar: Calendar,
    pub bundle: Option<Bundle>,
    pub audio: Audio,
    pub notice: Option<String>,
}
impl Snapshot {
    pub fn busy(&self) -> bool {
        self.phase.busy() || self.capture_busy
    }
    /// A targeted live event must still own this recording and selected session.
    /// This changes presentation only; durable publication/transforms stay in core.
    pub fn live_segment(&mut self, generation: u64, segment: Segment) -> bool {
        if generation != self.recording_generation
            || !self.phase.busy()
            || self.recording_session.as_deref() != Some(segment.session_id.as_str())
        {
            return false;
        }
        let Some(detail) = self
            .detail
            .as_mut()
            .filter(|d| d.session.id == segment.session_id)
        else {
            return false;
        };
        if let Some(existing) = detail.segments.iter_mut().find(|s| s.id == segment.id) {
            *existing = segment;
        } else {
            detail.segments.push(segment);
        }
        detail.segments.sort_by_key(|s| (s.start_ms, s.id));
        if detail.segments.len() > 200 {
            detail.segments.drain(..detail.segments.len() - 200);
        }
        true
    }
}
fn text_ok(s: &str, max: usize) -> bool {
    s.len() <= max && !s.contains('\0')
}
impl Projection for Snapshot {
    fn revision(&self) -> u64 {
        self.revision
    }
    fn bytes(&self) -> usize {
        let meeting = |m: &Meeting| {
            m.id.len()
                + m.title.as_ref().map_or(0, String::len)
                + m.preview.len()
                + m.attendees.iter().map(String::len).sum::<usize>()
                + 512
        };
        self.meetings.iter().map(meeting).sum::<usize>()
            + self.query.len()
            + self.echo_notice.as_ref().map_or(0, String::len)
            + self.notice.as_ref().map_or(0, String::len)
            + self.detail.as_ref().map_or(0, |d| {
                meeting(&d.session)
                    + d.segments
                        .iter()
                        .map(|s| s.text.len() + s.session_id.len() + 128)
                        .sum::<usize>()
                    + d.remote.iter().map(|r| r.label.len() + 16).sum::<usize>()
                    + d.me.len()
                    + d.them.len()
            })
            + match &self.calendar {
                Calendar::Choosing(c) => c
                    .iter()
                    .map(|c| {
                        c.token.len()
                            + c.title.len()
                            + c.attendees.iter().map(String::len).sum::<usize>()
                            + 128
                    })
                    .sum(),
                _ => 0,
            }
            + 2048
    }
    fn valid(&self) -> bool {
        let meeting = |m: &Meeting| {
            !m.id.is_empty()
                && text_ok(&m.id, 128)
                && m.title.as_ref().is_none_or(|t| t.chars().count() <= 200)
                && m.attendees.len() <= 100
                && m.attendees.iter().all(|s| s.chars().count() <= 200)
                && matches!(
                    m.status.as_str(),
                    "active" | "complete" | "interrupted" | "failed"
                )
        };
        self.bytes() <= 16 * 1024 * 1024
            && self.meetings.len() <= 10000
            && self.total >= self.meetings.len()
            && self.meetings.iter().all(meeting)
            && self
                .meetings
                .iter()
                .map(|s| &s.id)
                .collect::<BTreeSet<_>>()
                .len()
                == self.meetings.len()
            && text_ok(&self.query, 4096)
            && (!self.phase.active() || self.recording_generation > 0)
            && self.audio.position_ms <= self.audio.duration_ms
            && self.detail.as_ref().is_none_or(|d| {
                meeting(&d.session)
                    && d.me.chars().count() <= 80
                    && d.them.chars().count() <= 80
                    && d.remote.len() <= 100
                    && d.remote
                        .iter()
                        .all(|r| r.id > 0 && r.label.chars().count() <= 80)
                    && d.segments.len() <= 10000
                    && d.segments
                        .iter()
                        .map(|s| s.id)
                        .collect::<BTreeSet<_>>()
                        .len()
                        == d.segments.len()
                    && d.segments.iter().all(|s| {
                        s.session_id == d.session.id
                            && s.channel != Channel::All
                            && s.end_ms >= s.start_ms
                            && text_ok(&s.text, 131072)
                            && matches!(s.status.as_str(), "pending" | "final" | "failed")
                            && (s.channel != Channel::Me || s.remote_speaker.is_none())
                    })
            })
            && match &self.calendar {
                Calendar::Choosing(c) => {
                    self.detail.as_ref().is_some_and(Detail::finished)
                        && c.len() <= 100
                        && c.iter().all(|c| {
                            text_ok(&c.token, 4096)
                                && !c.token.is_empty()
                                && c.title.chars().count() <= 200
                                && c.end_ms > c.start_ms
                                && c.attendees.len() <= 100
                                && c.attendees.iter().all(|s| s.chars().count() <= 200)
                        })
                }
                _ => true,
            }
    }
}
pub fn timestamp(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}
pub fn duration(ms: u64) -> String {
    let m = ms / 60000;
    if m == 0 {
        format!("{} sec", (ms + 500) / 1000)
    } else if m < 60 {
        format!("{m} min")
    } else if m.is_multiple_of(60) {
        format!("{} hr", m / 60)
    } else {
        format!("{} hr {} min", m / 60, m % 60)
    }
}
pub fn error_message(code: &str) -> &'static str {
    match code {
        "unsupported_os" => "Meeting capture requires macOS 14.2 or newer.",
        "system_audio_permission_denied" => "System Audio access was denied.",
        "microphone_permission_denied" => "Microphone access was denied.",
        "microphone_unavailable" => "The selected microphone is unavailable.",
        "system_audio_unavailable" => "System Audio capture is unavailable.",
        "system_audio_callback_stalled" => "System Audio stopped delivering audio.",
        "microphone_callback_stalled" => "The microphone stopped delivering audio.",
        "permission_prompt_timeout" => "The permission request timed out.",
        "capture_setup_timeout" => {
            "Core Audio stalled while starting capture. Quit other audio capture apps and try again."
        }
        "capture_backlog" => "Audio processing could not keep up safely.",
        "capture_stop_timeout" => {
            "The capture worker did not stop cleanly. Murmur forced it to exit."
        }
        "termination_unconfirmed" => {
            "The capture worker could not be confirmed stopped. Restart Murmur before recording again."
        }
        "protocol_error" => "The signed capture worker returned an invalid response.",
        "spool_failed" => "A meeting audio chunk could not be stored safely.",
        "transcription_failed" => "A meeting transcript chunk could not be transcribed.",
        "store_unavailable" => "The local meeting transcript store is unavailable.",
        _ => "Meeting capture stopped unexpectedly.",
    }
}
