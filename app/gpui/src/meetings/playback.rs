//! Bounded native timeline playback, independent of GPUI and Web Audio.
//! Source adapters call the existing checked meeting_audio services. No paths,
//! raw error descriptions, transcript payloads or whole-session audio live here.
use super::model::Channel;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
pub const RATE: u64 = 16000;
const BLOCK: usize = 1600;
const RANGE: usize = 65536;
const MAX_WAV: usize = 512 * 1024;
const PAGE: usize = 128;
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub start_ms: u64,
    pub segment_id: u64,
}
#[derive(Clone)]
pub struct Chunk {
    pub cursor: Cursor,
    pub channel: Channel,
    pub end_ms: u64,
}
pub struct Page {
    pub duration_ms: u64,
    pub chunks: Vec<Chunk>,
    pub next: Option<Cursor>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Unavailable,
    Busy,
    Invalid,
    Revoked,
    Output,
}
pub trait Source: Send + Sync {
    /// Rechecks native busy owners and exact retained/finished session admission.
    /// An unrelated deletion must not revoke this session. No cached permission.
    fn admitted(&self, session: &str) -> Result<(), Error>;
    fn manifest(
        &self,
        session: &str,
        channel: Channel,
        from_ms: u64,
        cursor: Option<Cursor>,
        limit: usize,
    ) -> Result<Page, Error>;
    /// Existing validated reader checks descriptor/path/revision before and after
    /// every <=64 KiB read, including exact session/segment ownership.
    fn range(
        &self,
        session: &str,
        segment: u64,
        offset: usize,
        length: usize,
    ) -> Result<Vec<u8>, Error>;
}
pub trait Sink: Send + Sync {
    /// Fence a newer generation immediately; perform engine teardown on its worker.
    /// Must never wait on engine startup or shutdown (callable from the UI).
    fn reset(&self, generation: u64);
    fn start(&self, generation: u64) -> Result<(), Error>;
    fn enqueue(&self, generation: u64, mono: &[f32]) -> Result<(), Error>;
    fn healthy(&self, generation: u64) -> bool;
    fn queued_frames(&self) -> u64;
    fn played_frames(&self) -> u64;
}
#[derive(Clone)]
pub struct Control {
    epoch: Arc<AtomicU64>,
    sink: Arc<dyn Sink>,
}
impl Control {
    pub fn new(sink: Arc<dyn Sink>) -> Self {
        Self {
            epoch: Arc::new(AtomicU64::new(0)),
            sink,
        }
    }
    /// Callable synchronously from hide/busy/delete while a worker reads bytes.
    pub fn pause(&self) {
        self.next();
    }
    fn next(&self) -> u64 {
        let g = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        self.sink.reset(g);
        g
    }
    fn current(&self, g: u64) -> bool {
        self.epoch.load(Ordering::SeqCst) == g
    }
    fn pause_if_current(&self, generation: u64) {
        // A worker may finish between the UI's new Play/Pause and teardown.
        // Checking then incrementing separately could revoke the newer attempt.
        if self
            .epoch
            .compare_exchange(
                generation,
                generation + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
        {
            self.sink.reset(generation + 1);
        }
    }
}
struct Decoded {
    chunk: Chunk,
    samples: Vec<f32>,
}
#[derive(Default)]
struct Track {
    metadata: VecDeque<Chunk>,
    decoded: VecDeque<Decoded>,
    cursor: Option<Cursor>,
    loaded: bool,
    finished: bool,
}
pub struct Player {
    source: Arc<dyn Source>,
    control: Control,
    session: String,
    channel: Channel,
    generation: u64,
    start_frame: u64,
    next_frame: u64,
    duration_ms: u64,
    tracks: [Track; 2],
    playing: bool,
}
impl Player {
    pub fn new(source: Arc<dyn Source>, control: Control, session: String) -> Self {
        Self {
            source,
            control,
            session,
            channel: Channel::All,
            generation: 0,
            start_frame: 0,
            next_frame: 0,
            duration_ms: 0,
            tracks: Default::default(),
            playing: false,
        }
    }
    fn check(&self) -> Result<(), Error> {
        if !self.control.current(self.generation) {
            return Err(Error::Revoked);
        }
        self.source.admitted(&self.session)?;
        if !self.control.current(self.generation) {
            return Err(Error::Revoked);
        }
        Ok(())
    }
    pub fn position_ms(&self) -> u64 {
        (self.start_frame + self.control.sink.played_frames()).saturating_mul(1000) / RATE
    }
    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }
    pub fn play(&mut self, position_ms: u64, channel: Channel) -> Result<(), Error> {
        let generation = self.control.next();
        self.start(generation, position_ms, channel)
    }
    fn start(&mut self, generation: u64, position_ms: u64, channel: Channel) -> Result<(), Error> {
        self.generation = generation;
        self.playing = false;
        self.channel = channel;
        self.start_frame = position_ms.saturating_mul(RATE) / 1000;
        self.next_frame = self.start_frame;
        self.tracks = Default::default();
        self.duration_ms = 0;
        self.check()?;
        self.control.sink.start(self.generation)?;
        self.check()?;
        self.playing = true;
        Ok(())
    }
    pub fn pause(&mut self) {
        self.start_frame = self.position_ms().saturating_mul(RATE) / 1000;
        self.playing = false;
        self.control.pause();
        self.tracks = Default::default();
    }
    /// Run on an admitted background worker every ~20 ms. At most 300 ms is
    /// queued. Silent gaps use zeros at original offsets; overlapping channels
    /// add at recorded levels, with no gain/normalization or concatenation.
    pub fn pump(&mut self) -> Result<bool, Error> {
        if !self.playing {
            return Ok(false);
        }
        let result = self.pump_inner();
        if result.is_err() {
            self.playing = false;
            self.control.pause_if_current(self.generation);
            self.tracks = Default::default();
        }
        result
    }
    fn pump_inner(&mut self) -> Result<bool, Error> {
        self.check()?;
        if !self.control.sink.healthy(self.generation) {
            return Err(Error::Output);
        }
        if self.control.sink.queued_frames() >= 3 * BLOCK as u64 {
            return Ok(true);
        }
        let channels = match self.channel {
            Channel::All => vec![Channel::Me, Channel::Them],
            c => vec![c],
        };
        let from = self.next_frame * 1000 / RATE;
        for channel in channels {
            let index = if channel == Channel::Me { 0 } else { 1 };
            self.fill(index, channel, from)?;
        }
        self.check()?;
        let end_frame = self.duration_ms * RATE / 1000;
        if self.next_frame >= end_frame {
            if self.control.sink.queued_frames() == 0 {
                self.playing = false;
                self.start_frame = end_frame;
                self.control.pause_if_current(self.generation);
                self.tracks = Default::default();
                return Ok(false);
            }
            return Ok(true);
        }
        let count = (end_frame - self.next_frame).min(BLOCK as u64) as usize;
        let mut block = vec![0f32; count];
        for track in &self.tracks {
            for decoded in &track.decoded {
                let start = decoded.chunk.cursor.start_ms * RATE / 1000;
                for (i, out) in block.iter_mut().enumerate() {
                    if let Some(offset) = (self.next_frame + i as u64)
                        .checked_sub(start)
                        .and_then(|v| usize::try_from(v).ok())
                        && let Some(value) = decoded.samples.get(offset)
                    {
                        *out += value;
                    }
                }
            }
        }
        self.check()?;
        self.control.sink.enqueue(self.generation, &block)?;
        self.next_frame += count as u64;
        Ok(true)
    }
    fn fill(&mut self, index: usize, channel: Channel, from: u64) -> Result<(), Error> {
        self.tracks[index].decoded.retain(|d| d.chunk.end_ms > from);
        // Keep current and next decoded chunk per channel. Metadata stays paged.
        while self.tracks[index].decoded.len() < 2 {
            if self.tracks[index].metadata.is_empty() && !self.tracks[index].finished {
                self.check()?;
                let cursor = self.tracks[index].cursor;
                let page = self
                    .source
                    .manifest(&self.session, channel, from, cursor, PAGE)?;
                self.check()?;
                if page.duration_ms > 7 * 24 * 60 * 60 * 1000
                    || page.chunks.len() > PAGE
                    || page.chunks.iter().any(|c| {
                        c.channel != channel
                            || c.end_ms <= c.cursor.start_ms
                            || c.end_ms - c.cursor.start_ms > 15500
                            || c.end_ms > page.duration_ms
                            || c.end_ms <= from
                            || cursor.is_some_and(|v| !after(c.cursor, v))
                    })
                    || page
                        .chunks
                        .windows(2)
                        .any(|c| !after(c[1].cursor, c[0].cursor))
                    || page
                        .next
                        .is_some_and(|v| page.chunks.last().is_none_or(|c| c.cursor != v))
                    || (self.tracks[index].loaded && page.duration_ms != self.duration_ms)
                {
                    return Err(Error::Invalid);
                }
                if self.duration_ms != 0 && self.duration_ms != page.duration_ms {
                    return Err(Error::Invalid);
                }
                self.duration_ms = page.duration_ms;
                self.tracks[index].loaded = true;
                self.tracks[index].finished = page.next.is_none();
                self.tracks[index].cursor = page.next;
                self.tracks[index].metadata = page.chunks.into();
            }
            let Some(chunk) = self.tracks[index].metadata.front().cloned() else {
                break;
            };
            if chunk.end_ms <= from {
                self.tracks[index].metadata.pop_front();
                continue;
            }
            // Preserve long gaps without eagerly reading distant audio.
            if chunk.cursor.start_ms > from + 200 {
                break;
            }
            let samples = self.read(&chunk)?;
            self.check()?;
            if self.tracks[index]
                .decoded
                .back()
                .is_some_and(|d| d.chunk.end_ms > chunk.cursor.start_ms)
            {
                return Err(Error::Invalid);
            }
            self.tracks[index].metadata.pop_front();
            self.tracks[index]
                .decoded
                .push_back(Decoded { chunk, samples });
        }
        Ok(())
    }
    fn read(&self, chunk: &Chunk) -> Result<Vec<f32>, Error> {
        let mut wav = Vec::new();
        loop {
            self.check()?;
            let part =
                self.source
                    .range(&self.session, chunk.cursor.segment_id, wav.len(), RANGE)?;
            self.check()?;
            if part.len() > RANGE || wav.len() + part.len() > MAX_WAV {
                return Err(Error::Invalid);
            }
            let eof = part.len() < RANGE;
            wav.extend(part);
            if eof {
                break;
            }
        }
        decode(&wav, chunk.end_ms - chunk.cursor.start_ms)
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        self.control.pause_if_current(self.generation);
    }
}
/// One worker per player, with one replaceable pending Play and one progress
/// mailbox. Cancellation fences immediately; no UI call waits on native audio.
#[derive(Clone, Copy)]
pub struct Progress {
    pub position_ms: u64,
    pub playing: bool,
}
type Mailbox = Arc<Mutex<Option<Result<Progress, Error>>>>;
struct PlayRequest {
    generation: u64,
    position_ms: u64,
    channel: Channel,
    mailbox: Mailbox,
}
#[derive(Default)]
struct Requests {
    play: Option<PlayRequest>,
    shutdown: bool,
}
type Pending = Arc<(Mutex<Requests>, Condvar)>;
pub struct Worker {
    source: Arc<dyn Source>,
    control: Control,
    session: String,
    pending: Pending,
    thread: Option<std::thread::JoinHandle<()>>,
    updates: Option<Mailbox>,
    start_ms: u64,
    position_ms: u64,
}
impl Worker {
    pub fn new(source: Arc<dyn Source>, control: Control, session: String) -> Self {
        Self {
            source,
            control,
            session,
            pending: Default::default(),
            thread: None,
            updates: None,
            start_ms: 0,
            position_ms: 0,
        }
    }
    pub fn play(&mut self, position_ms: u64, channel: Channel) -> Result<(), Error> {
        // Reserve on the caller. The worker cannot mint a generation after a
        // newer Play/Pause and thereby regain ownership.
        let generation = self.control.next();
        self.updates = None;
        self.start_ms = position_ms;
        self.position_ms = position_ms;
        if self.thread.is_none() {
            let pending = self.pending.clone();
            let player = Player::new(
                self.source.clone(),
                self.control.clone(),
                self.session.clone(),
            );
            self.thread = Some(
                std::thread::Builder::new()
                    .name("meeting-playback".into())
                    .spawn(move || Self::run(player, pending))
                    .map_err(|_| Error::Output)?,
            );
        }
        let mailbox: Mailbox = Default::default();
        self.pending
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .play = Some(PlayRequest {
            generation,
            position_ms,
            channel,
            mailbox: mailbox.clone(),
        });
        self.updates = Some(mailbox);
        self.pending.1.notify_one();
        Ok(())
    }
    fn run(mut player: Player, pending: Pending) {
        let mut active: Option<PlayRequest> = None;
        loop {
            let request = {
                let mut requests = pending.0.lock().unwrap_or_else(|p| p.into_inner());
                loop {
                    if requests.shutdown {
                        return;
                    }
                    if let Some(request) = requests.play.take() {
                        break Some(request);
                    }
                    if active
                        .as_ref()
                        .is_some_and(|r| player.control.current(r.generation))
                    {
                        break None;
                    }
                    // Pause has already fenced the sink. Release decoded audio
                    // here without issuing a reset against a newer generation.
                    active = None;
                    player.playing = false;
                    player.tracks = Default::default();
                    requests = pending.1.wait(requests).unwrap_or_else(|p| p.into_inner());
                }
            };
            // No request/mailbox lock is held across native or source calls.
            let result = if let Some(request) = request {
                let result = player.start(request.generation, request.position_ms, request.channel);
                active = Some(request);
                result
            } else {
                Ok(())
            };
            if result.is_err() {
                player.control.pause_if_current(player.generation);
            }
            let update = result.and_then(|()| player.pump()).map(|playing| Progress {
                position_ms: player.position_ms(),
                playing,
            });
            let finished = !matches!(update, Ok(Progress { playing: true, .. }));
            if let Some(request) = &active {
                // Old mailboxes are discarded synchronously by Play/Pause.
                *request.mailbox.lock().unwrap_or_else(|p| p.into_inner()) = Some(update);
            }
            if finished {
                active = None;
            }
            let requests = pending.0.lock().unwrap_or_else(|p| p.into_inner());
            if !requests.shutdown && requests.play.is_none() && active.is_some() {
                // Wake promptly for replacement, pause, or shutdown; otherwise
                // pump every ~20 ms with the existing 300 ms queue bound.
                drop(
                    pending
                        .1
                        .wait_timeout(requests, std::time::Duration::from_millis(20)),
                );
            }
        }
    }
    pub fn poll(&mut self) -> Option<Result<Progress, Error>> {
        let update = self
            .updates
            .as_ref()?
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()?;
        if let Ok(progress) = update {
            self.position_ms = progress.position_ms;
        }
        if !matches!(update, Ok(Progress { playing: true, .. })) {
            self.updates = None;
        }
        Some(update)
    }
    pub fn position_ms(&self) -> u64 {
        if self.updates.is_some() {
            self.start_ms
                .saturating_add(self.control.sink.played_frames().saturating_mul(1000) / RATE)
        } else {
            self.position_ms
        }
    }
    pub fn pause(&mut self) {
        self.position_ms = self.position_ms();
        self.control.pause();
        self.pending
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .play = None;
        // Late start/progress/error results have no route back to the UI.
        self.updates = None;
        self.pending.1.notify_one();
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.pause();
        self.pending
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .shutdown = true;
        self.pending.1.notify_one();
        if let Some(thread) = self.thread.take() {
            // Never wait for a stalled Core Audio call. At most this one worker
            // is detached; it releases its player/sink as soon as the call ends.
            if thread.is_finished() {
                let _ = thread.join();
            }
        }
    }
}
fn after(a: Cursor, b: Cursor) -> bool {
    (a.start_ms, a.segment_id) > (b.start_ms, b.segment_id)
}
fn decode(wav: &[u8], duration: u64) -> Result<Vec<f32>, Error> {
    // Capture's canonical RIFF/fmt/data layout, also validated by the service.
    let u16at = |o| u16::from_le_bytes([wav[o], wav[o + 1]]);
    let u32at = |o| u32::from_le_bytes([wav[o], wav[o + 1], wav[o + 2], wav[o + 3]]);
    if wav.len() < 44
        || wav.len() > MAX_WAV
        || &wav[0..4] != b"RIFF"
        || &wav[8..12] != b"WAVE"
        || &wav[12..16] != b"fmt "
        || &wav[36..40] != b"data"
    {
        return Err(Error::Invalid);
    }
    let bytes = u32at(40) as usize;
    if u32at(4) as usize + 8 != wav.len()
        || u32at(16) != 16
        || u16at(20) != 1
        || u16at(22) != 1
        || u32at(24) != RATE as u32
        || u32at(28) != 32000
        || u16at(32) != 2
        || u16at(34) != 16
        || !bytes.is_multiple_of(2)
        || bytes + 44 != wav.len()
        || bytes == 0
        || ((bytes / 2) as i64 - (duration * RATE / 1000) as i64).unsigned_abs() > 32
    {
        return Err(Error::Invalid);
    }
    Ok(wav[44..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|s| i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.)
        .collect())
}
#[cfg(target_os = "macos")]
pub struct NativeSink(std::ptr::NonNull<std::ffi::c_void>);
#[cfg(target_os = "macos")]
// SAFETY: the native object serializes start/reset/enqueue; counters are atomic.
unsafe impl Send for NativeSink {}
#[cfg(target_os = "macos")]
unsafe impl Sync for NativeSink {}
#[cfg(target_os = "macos")]
impl NativeSink {
    pub fn new() -> Result<Self, Error> {
        std::ptr::NonNull::new(unsafe { murmur_meeting_output_new() })
            .map(Self)
            .ok_or(Error::Output)
    }
}
#[cfg(target_os = "macos")]
impl Sink for NativeSink {
    fn reset(&self, g: u64) {
        unsafe { murmur_meeting_output_reset(self.0.as_ptr(), g) }
    }
    fn start(&self, g: u64) -> Result<(), Error> {
        if unsafe { murmur_meeting_output_start(self.0.as_ptr(), g) } {
            Ok(())
        } else {
            Err(Error::Output)
        }
    }
    fn enqueue(&self, g: u64, s: &[f32]) -> Result<(), Error> {
        if s.len() > BLOCK {
            return Err(Error::Invalid);
        }
        if unsafe { murmur_meeting_output_push(self.0.as_ptr(), g, s.as_ptr(), s.len()) } {
            Ok(())
        } else {
            Err(Error::Revoked)
        }
    }
    fn healthy(&self, g: u64) -> bool {
        unsafe { murmur_meeting_output_healthy(self.0.as_ptr(), g) }
    }
    fn queued_frames(&self) -> u64 {
        unsafe { murmur_meeting_output_queued(self.0.as_ptr()) }
    }
    fn played_frames(&self) -> u64 {
        unsafe { murmur_meeting_output_played(self.0.as_ptr()) }
    }
}
#[cfg(target_os = "macos")]
impl Drop for NativeSink {
    fn drop(&mut self) {
        unsafe { murmur_meeting_output_free(self.0.as_ptr()) }
    }
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn murmur_meeting_output_new() -> *mut std::ffi::c_void;
    fn murmur_meeting_output_reset(output: *mut std::ffi::c_void, g: u64);
    fn murmur_meeting_output_start(output: *mut std::ffi::c_void, g: u64) -> bool;
    fn murmur_meeting_output_push(
        output: *mut std::ffi::c_void,
        g: u64,
        s: *const f32,
        len: usize,
    ) -> bool;
    fn murmur_meeting_output_healthy(output: *mut std::ffi::c_void, g: u64) -> bool;
    fn murmur_meeting_output_queued(output: *mut std::ffi::c_void) -> u64;
    fn murmur_meeting_output_played(output: *mut std::ffi::c_void) -> u64;
    fn murmur_meeting_output_free(output: *mut std::ffi::c_void);
}
#[cfg(test)]
mod tests;
