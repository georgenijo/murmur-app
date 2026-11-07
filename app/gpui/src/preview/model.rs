//! Ephemeral presentation only. Never Debug/serialize transcript-bearing types.
use murmur_core::{dictation::Status, overlay_geometry::*};

pub const MAX_PARTIAL_UNITS: usize = 4096;
pub const MAX_VISIBLE_UNITS: usize = 320;
pub const WIDTH: f64 = 460.;
pub const HEIGHT: f64 = 104.;
pub const LIFT_RESERVE: f64 = 64.;

/// UTF-16 bounds match the old JS contract without splitting a Rust character.
pub fn visible_partial(text: &str) -> String {
    let text = text.trim();
    if text.encode_utf16().count() <= MAX_VISIBLE_UNITS {
        return text.into();
    }
    let mut units = 0;
    let mut start = text.len();
    for (i, c) in text.char_indices().rev() {
        units += c.len_utf16();
        if units > MAX_VISIBLE_UNITS {
            break;
        }
        start = i;
    }
    let tail = &text[start..];
    tail.find(' ').map_or(tail, |i| &tail[i + 1..]).into()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    /// Acknowledged collapsed overlay frame, in logical global top-left points.
    pub frame: OverlayFrame,
    pub above: bool,
    pub lift: f64,
    pub work: PopoverWorkArea,
}
impl Anchor {
    pub fn valid(self) -> bool {
        [
            self.frame.x,
            self.frame.y,
            self.frame.width,
            self.frame.height,
            self.lift,
            self.work.left,
            self.work.right,
            self.work.top,
            self.work.bottom,
        ]
        .iter()
        .all(|n| n.is_finite())
            && self.frame.width > 0.
            && self.frame.height > 0.
            && self.work.right > self.work.left
            && self.work.bottom > self.work.top
            && self.lift >= 0.
            && self.lift <= 136.
    }
    pub fn window(self, reserve: f64) -> Option<OverlayFrame> {
        if !self.valid() {
            return None;
        }
        let width = WIDTH.min(self.work.right - self.work.left);
        let (x, y, height) = dictation_preview_fit_for_frame(
            self.work,
            (width, HEIGHT + reserve),
            Some(self.frame),
            self.above,
        )?;
        Some(OverlayFrame {
            x,
            y,
            width,
            height,
        })
    }
}

/// Captured after layout; invalidated by every content/ownership/consent change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub generation: u64,
    revision: u64,
}

pub enum Event {
    /// The host subscribes first, then starts a durable settings read. A true
    /// notification alone is never consent. Opt-out revokes before that read.
    ConsentChanged {
        opted_out: bool,
    },
    ConsentRead {
        epoch: u64,
        enabled: bool,
    },
    Lifecycle {
        generation: u64,
        status: Status,
    },
    Cancelled(u64),
    Partial {
        generation: u64,
        text: String,
    },
    /// Only a producer with an authoritative boundary may use this variant.
    /// Today's whole-string decoder must continue to use Partial.
    AuthoritativePartial {
        generation: u64,
        stable: String,
        tentative: String,
    },
    Anchor(Anchor),
    /// Detach/display loss empties content and invalidates pending frame work.
    Detached,
}

pub struct Model {
    generation: u64,
    stopped_through: u64,
    recording: bool,
    enabled: bool,
    consent_epoch: u64,
    revision: u64,
    text: String,
    stable_bytes: usize,
    pub anchor: Option<Anchor>,
    /// Retain transparent room across hover-off; the native window never snaps.
    pub reserve: f64,
}
impl Default for Model {
    fn default() -> Self {
        Self {
            generation: 0,
            stopped_through: 0,
            recording: false,
            enabled: false,
            consent_epoch: 0,
            revision: 0,
            text: String::new(),
            stable_bytes: 0,
            anchor: None,
            reserve: LIFT_RESERVE,
        }
    }
}
impl Model {
    pub fn consent_epoch(&self) -> u64 {
        self.consent_epoch
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn stable_bytes(&self) -> usize {
        self.stable_bytes
    }
    pub fn visible(&self) -> bool {
        self.enabled
            && self.recording
            && !self.text.is_empty()
            && self.generation > self.stopped_through
            && self
                .anchor
                .and_then(|a| a.window(self.reserve))
                .is_some_and(|f| f.height > 0.)
    }
    pub fn ticket(&self) -> Option<Ticket> {
        self.visible().then_some(Ticket {
            generation: self.generation,
            revision: self.revision,
        })
    }
    pub fn owns(&self, ticket: Ticket) -> bool {
        self.ticket() == Some(ticket)
    }
    pub fn clear(&mut self) {
        self.text.clear();
        self.stable_bytes = 0;
        // Release the allocation as well; no hidden presentation cache.
        self.text.shrink_to_fit();
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn rejected(&mut self, ticket: Ticket) {
        if self.owns(ticket) {
            self.clear();
        }
    }
    pub fn event(&mut self, event: Event) {
        match event {
            Event::ConsentChanged { opted_out } => {
                self.consent_epoch = self.consent_epoch.wrapping_add(1);
                if opted_out {
                    self.enabled = false;
                    self.clear();
                }
            }
            Event::ConsentRead { epoch, enabled } if epoch == self.consent_epoch => {
                self.enabled = enabled;
                if !enabled {
                    self.clear();
                }
            }
            Event::Lifecycle { generation, status }
                if generation > 0 && generation >= self.generation =>
            {
                if generation > self.generation {
                    self.clear();
                    self.generation = generation;
                }
                self.recording = status == Status::Recording && generation > self.stopped_through;
                if matches!(
                    status,
                    Status::Idle | Status::Processing | Status::Recovering
                ) {
                    self.stopped_through = self.stopped_through.max(generation);
                }
                if !self.recording {
                    self.clear();
                }
            }
            Event::Cancelled(generation) if generation > 0 && generation >= self.generation => {
                self.stopped_through = self.stopped_through.max(generation);
                self.recording = false;
                self.clear();
            }
            Event::Partial { generation, text }
                if self.enabled
                    && self.recording
                    && generation >= self.generation
                    && generation > self.stopped_through
                    && text.encode_utf16().count() <= MAX_PARTIAL_UNITS =>
            {
                let visible = visible_partial(&text);
                if generation == self.generation && self.stable_bytes == 0 && self.text == visible {
                    return;
                }
                // A newer partial may precede its generation notification.
                self.generation = generation;
                self.clear();
                self.text = visible;
            }
            Event::AuthoritativePartial {
                generation,
                stable,
                tentative,
            } if self.enabled
                && self.recording
                && generation >= self.generation
                && generation > self.stopped_through
                && stable.encode_utf16().count() + tentative.encode_utf16().count()
                    <= MAX_PARTIAL_UNITS =>
            {
                self.generation = generation;
                self.clear();
                self.text = visible_partial(&stable);
                self.stable_bytes = self.text.len();
                let tail = visible_partial(&tentative);
                if !self.text.is_empty() && !tail.is_empty() {
                    self.text.push(' ');
                }
                self.text.push_str(&tail);
            }
            Event::Anchor(anchor) if anchor.valid() => {
                self.reserve = self.reserve.max(anchor.lift);
                self.anchor = Some(anchor);
            }
            Event::Anchor(_) | Event::Detached => {
                self.anchor = None;
                self.recording = false;
                self.clear();
            }
            _ => {}
        }
    }
}
