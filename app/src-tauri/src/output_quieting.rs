//! Mute speakers while dictating (opt-in `muteSpeakersWhileDictating`).
//!
//! While a live dictation runs, Murmur mutes the default output device so
//! playback from the speakers does not reach the microphone, then unmutes it
//! when the dictation ends. It never pauses or resumes media (the reverted
//! browser-media design in #842 could start playback the user had paused), and
//! it never opens an audio unit: every operation is a Core Audio HAL property
//! read or write on the output device, so it cannot reintroduce the
//! voice-processing ducking from #177.
//!
//! Rules, each covered by the tests below:
//! - Only the mute property changes. The volume level is never written, so a
//!   restore returns the exact prior level.
//! - The mute waits for the start cue: the main window reports when the cue
//!   has played out (or that there is none). A hard cap mutes anyway if that
//!   report never arrives.
//! - Engage is skipped when the output is headphones (built-in jack) or
//!   Bluetooth (AirPods, headsets), when the device has no settable mute (many
//!   HDMI/DisplayPort outputs), or when it is already muted. Each generation
//!   decides once.
//! - Restore touches only the device Murmur muted, found again by its UID. If
//!   the user unmuted it during the dictation (volume keys unmute), the restore
//!   leaves it alone. A new default output is never touched.
//! - After a successful mute, a local record (device UID, mute elements, mute
//!   time) is written to `output-quieting.json`. The next launch restores from
//!   it if the app died mid-dictation, but only while the record is younger
//!   than [`RECORD_MAX_AGE`]. An older record is dropped without touching the
//!   device: an unexpected unmute is worse than a Mac left muted.
//! - A restore that cannot finish (device gone, read or write failed) is
//!   retried on a short backoff until the record expires.
//! - All work runs on one background thread. The dictation status seam only
//!   sends a message, so start and stop never wait on Core Audio.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::state::DictationStatus;

const RECORD_FILE_NAME: &str = "output-quieting.json";
const RECORD_SCHEMA_VERSION: u32 = 2;
const RECORD_MAX_BYTES: u64 = 8 * 1024;
const RECORD_MAX_ENTRIES: usize = 4;
const RECORD_MAX_ELEMENTS: usize = 8;

/// A record older than this is never acted on. Long enough to relaunch after
/// a crash; short enough that a later deliberate mute is unlikely to be undone.
pub(crate) const RECORD_MAX_AGE: Duration = Duration::from_secs(10 * 60);

/// Mute at the latest this long after the start if the main window never
/// reports that the start cue finished (webview busy, cue never played).
pub(crate) const START_CUE_CAP: Duration = Duration::from_millis(500);

/// Backoff for restores that could not finish. The last delay repeats until
/// the record reaches [`RECORD_MAX_AGE`], so retrying always ends.
const RETRY_DELAYS: [Duration; 7] = [
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(15),
    Duration::from_secs(30),
];

/// Bound on how long app exit waits for the worker to unmute.
const SHUTDOWN_WAIT: Duration = Duration::from_millis(750);

/// `'hdpn'`: the built-in output's headphone-jack data source.
const DATA_SOURCE_HEADPHONES: u32 = u32::from_be_bytes(*b"hdpn");

/// Apple Silicon Macs publish the headphone jack as its own built-in device.
/// Its UID is fixed and not localized. Not observed on hardware in this repo's
/// tests; the `'hdpn'` data source check also applies to it.
const BUILT_IN_HEADPHONE_UID: &str = "BuiltInHeadphoneOutputDevice";

// ---------------------------------------------------------------------------
// Device model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transport {
    BuiltIn,
    Bluetooth,
    Usb,
    Hdmi,
    DisplayPort,
    AirPlay,
    Aggregate,
    Virtual,
    Thunderbolt,
    Other,
    Unknown,
}

impl Transport {
    /// Maps `kAudioDevicePropertyTransportType`. Literal four-char codes keep
    /// this testable on every platform; a macOS test pins them to the SDK.
    pub(crate) fn from_code(code: Option<u32>) -> Self {
        let Some(code) = code else {
            return Self::Unknown;
        };
        match &code.to_be_bytes() {
            b"bltn" => Self::BuiltIn,
            b"blue" | b"blea" => Self::Bluetooth,
            b"usb " => Self::Usb,
            b"hdmi" => Self::Hdmi,
            b"dprt" => Self::DisplayPort,
            b"airp" => Self::AirPlay,
            b"grup" | b"fgrp" => Self::Aggregate,
            b"virt" => Self::Virtual,
            b"thun" => Self::Thunderbolt,
            _ => Self::Other,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::BuiltIn => "built_in",
            Self::Bluetooth => "bluetooth",
            Self::Usb => "usb",
            Self::Hdmi => "hdmi",
            Self::DisplayPort => "displayport",
            Self::AirPlay => "airplay",
            Self::Aggregate => "aggregate",
            Self::Virtual => "virtual",
            Self::Thunderbolt => "thunderbolt",
            Self::Other => "other",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutputDevice {
    pub id: u32,
    /// Core Audio UID. For USB devices it can contain a product name and a
    /// serial number. Persisted locally (mode 0600) only; never logged.
    pub uid: String,
    pub transport: Transport,
    /// The device's current output data source is the headphone jack.
    pub headphone_source: bool,
}

/// Why an output is (or is not) considered audible to the microphone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Speakers,
    Headphones,
    Bluetooth,
}

/// Skip only on evidence that the microphone cannot hear the output. When the
/// route is unclear (unknown transport, aggregate, virtual) Murmur still mutes:
/// the user asked for playback to stay out of transcripts, and a needless mute
/// is undone at stop, while a wrong skip silently defeats the setting. Bluetooth
/// counts as headphones because on a Mac it is nearly always AirPods or a
/// headset; Bluetooth speakers are a documented limit. USB headsets are muted.
fn route(device: &OutputDevice) -> Route {
    if device.transport == Transport::Bluetooth {
        Route::Bluetooth
    } else if device.headphone_source
        || (device.transport == Transport::BuiltIn && device.uid == BUILT_IN_HEADPHONE_UID)
    {
        Route::Headphones
    } else {
        Route::Speakers
    }
}

/// The narrow HAL surface the controller needs. Every method fails closed.
pub(crate) trait OutputHal {
    fn default_output(&mut self) -> Option<OutputDevice>;
    fn device_for_uid(&mut self, uid: &str) -> Option<u32>;
    /// Mute elements Murmur can set: the main element, or each stereo channel.
    /// `None` means the device has no software mute.
    fn mute_elements(&mut self, device: u32) -> Option<Vec<u32>>;
    /// One entry per element, `None` if any read fails.
    fn muted(&mut self, device: u32, elements: &[u32]) -> Option<Vec<bool>>;
    fn set_muted(&mut self, device: u32, elements: &[u32], muted: bool) -> bool;
}

// ---------------------------------------------------------------------------
// Durable record
// ---------------------------------------------------------------------------

/// "Murmur muted these elements on this device at this time." Written only
/// after the mute succeeded, so its existence means the mute was applied. No
/// audio, transcript, clipboard, selection, or app identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct QuietRecord {
    pub device_uid: String,
    pub elements: Vec<u32>,
    /// Wall-clock start of the expiry window, Unix milliseconds: the time of
    /// the mute, or the dictation's end when its live restore failed.
    pub muted_at_unix_ms: u64,
}

impl QuietRecord {
    /// Young enough to act on. A time in the future (clock moved back) counts
    /// as expired: when in doubt, leave the device alone.
    fn is_fresh(&self, now_unix_ms: u64) -> bool {
        now_unix_ms >= self.muted_at_unix_ms
            && now_unix_ms - self.muted_at_unix_ms <= RECORD_MAX_AGE.as_millis() as u64
    }
}

#[derive(Serialize, Deserialize)]
struct PersistedRecords {
    schema_version: u32,
    entries: Vec<QuietRecord>,
}

pub(crate) trait RecordStore {
    fn load(&mut self) -> Vec<QuietRecord>;
    /// Replace the durable set. An empty set removes the file.
    fn save(&mut self, records: &[QuietRecord]) -> bool;
}

fn encode_records(records: &[QuietRecord]) -> Option<String> {
    serde_json::to_string(&PersistedRecords {
        schema_version: RECORD_SCHEMA_VERSION,
        entries: records.to_vec(),
    })
    .ok()
}

/// A different schema (including the v1 records without a mute time) is
/// dropped, never acted on.
fn decode_records(contents: &str) -> Vec<QuietRecord> {
    let Ok(document) = serde_json::from_str::<PersistedRecords>(contents) else {
        return Vec::new();
    };
    if document.schema_version != RECORD_SCHEMA_VERSION {
        return Vec::new();
    }
    document
        .entries
        .into_iter()
        .filter(|record| {
            !record.device_uid.is_empty()
                && !record.elements.is_empty()
                && record.elements.len() <= RECORD_MAX_ELEMENTS
        })
        .take(RECORD_MAX_ENTRIES)
        .collect()
}

pub(crate) struct FileStore {
    path: Option<PathBuf>,
}

impl FileStore {
    pub(crate) fn new(data_dir: Option<PathBuf>) -> Self {
        Self {
            path: data_dir.map(|dir| dir.join(RECORD_FILE_NAME)),
        }
    }
}

/// `fsync` on macOS does not flush the drive's cache; `F_FULLFSYNC` does.
/// Falls back to `sync_all` where the filesystem refuses it.
fn full_sync(file: &std::fs::File) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::io::AsRawFd;
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_FULLFSYNC) } == 0 {
            return Ok(());
        }
    }
    file.sync_all()
}

fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let temp = path.with_file_name(format!(".{RECORD_FILE_NAME}.murmur-tmp"));
    let result = (|| {
        let mut file = std::fs::File::create(&temp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(contents.as_bytes())?;
        full_sync(&file)?;
        std::fs::rename(&temp, path)?;
        // Persist the rename itself.
        full_sync(&std::fs::File::open(parent)?)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

impl RecordStore for FileStore {
    fn load(&mut self) -> Vec<QuietRecord> {
        let Some(path) = self.path.as_deref() else {
            return Vec::new();
        };
        match std::fs::metadata(path) {
            Ok(metadata) if metadata.len() <= RECORD_MAX_BYTES => {}
            _ => return Vec::new(),
        }
        std::fs::read_to_string(path)
            .map(|contents| decode_records(&contents))
            .unwrap_or_default()
    }

    fn save(&mut self, records: &[QuietRecord]) -> bool {
        let Some(path) = self.path.as_deref() else {
            return false;
        };
        if records.is_empty() {
            return match std::fs::remove_file(path) {
                Ok(()) => true,
                Err(error) => error.kind() == std::io::ErrorKind::NotFound,
            };
        }
        let Some(contents) = encode_records(records) else {
            return false;
        };
        write_atomically(path, &contents).is_ok()
    }
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Engaged,
    Restored,
    RestoredAfterExit,
    UserChanged,
    Expired,
    SkippedNoOutput,
    SkippedHeadphones,
    SkippedBluetooth,
    SkippedNoMuteControl,
    SkippedAlreadyMuted,
    SkippedRecordUnavailable,
    MuteFailed,
    DeviceMissing,
    RestoreFailed,
    Unreadable,
}

impl Outcome {
    fn code(self) -> &'static str {
        match self {
            Self::Engaged => "engaged",
            Self::Restored => "restored",
            Self::RestoredAfterExit => "restored_after_exit",
            Self::UserChanged => "user_changed",
            Self::Expired => "expired",
            Self::SkippedNoOutput => "skipped_no_output",
            Self::SkippedHeadphones => "skipped_headphones",
            Self::SkippedBluetooth => "skipped_bluetooth",
            Self::SkippedNoMuteControl => "skipped_no_mute_control",
            Self::SkippedAlreadyMuted => "skipped_already_muted",
            Self::SkippedRecordUnavailable => "skipped_record_unavailable",
            Self::MuteFailed => "mute_failed",
            Self::DeviceMissing => "device_missing",
            Self::RestoreFailed => "restore_failed",
            Self::Unreadable => "unreadable",
        }
    }

    /// The restore could not finish; the record stays for a retry.
    fn keeps_record(self) -> bool {
        matches!(
            self,
            Self::DeviceMissing | Self::RestoreFailed | Self::Unreadable
        )
    }
}

/// What released a pending mute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Trigger {
    /// The main window reported the start cue finished, or that none played.
    CueFinished,
    /// [`START_CUE_CAP`] elapsed without that report.
    Cap,
}

impl Trigger {
    fn label(self) -> &'static str {
        match self {
            Self::CueFinished => "cue_finished",
            Self::Cap => "cap",
        }
    }
}

/// Content-free evidence for one decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Event {
    pub recording_id: u64,
    pub outcome: Outcome,
    pub transport: Option<Transport>,
    pub trigger: Option<Trigger>,
    /// Engage only: time from the dictation start to the mute decision.
    pub since_start_ms: Option<u64>,
    /// Time spent in Core Audio and the record store for this decision.
    pub hal_us: u64,
}

/// The controller's view of "now": a monotonic instant for scheduling and a
/// wall-clock time for record age.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Now {
    pub instant: Instant,
    pub unix_ms: u64,
}

impl Now {
    pub(crate) fn current() -> Self {
        Self {
            instant: Instant::now(),
            unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis() as u64)
                .unwrap_or(0),
        }
    }
}

#[derive(Debug)]
pub(crate) enum Command {
    /// Idle -> Starting: freeze the setting for this generation.
    Arm {
        recording_id: u64,
        enabled: bool,
        at: Instant,
    },
    /// The main window finished (or skipped) this generation's start cue.
    CueFinished {
        recording_id: u64,
    },
    /// Left live capture: Recovering, Processing, or Idle.
    Release {
        recording_id: u64,
    },
    Shutdown(mpsc::Sender<()>),
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    recording_id: u64,
    started: Instant,
    cap: Instant,
}

#[derive(Debug, Clone)]
struct Engaged {
    recording_id: u64,
    record: QuietRecord,
}

#[derive(Debug, Clone, Copy)]
struct Retry {
    due: Instant,
    attempt: usize,
}

pub(crate) struct Controller<H, S> {
    hal: H,
    store: S,
    /// At most one undecided mute: created at Arm, consumed by the first of
    /// cue-finished, cap, or release. Each generation therefore decides once.
    pending: Option<Pending>,
    engaged: Option<Engaged>,
    /// Records whose restore could not finish yet. Retried on [`RETRY_DELAYS`]
    /// and at the next start until they expire.
    outstanding: Vec<QuietRecord>,
    retry: Option<Retry>,
}

impl<H: OutputHal, S: RecordStore> Controller<H, S> {
    pub(crate) fn new(hal: H, store: S) -> Self {
        Self {
            hal,
            store,
            pending: None,
            engaged: None,
            outstanding: Vec::new(),
            retry: None,
        }
    }

    /// Launch recovery: restore what a previous process left muted, if the
    /// record is still fresh.
    pub(crate) fn recover_after_exit(&mut self, now: Now) -> Vec<Event> {
        self.outstanding = self.store.load();
        if self.outstanding.is_empty() {
            // Clear a stale or unreadable file so it is not reread forever.
            let _ = self.persist();
            return Vec::new();
        }
        self.retry_outstanding(0, Outcome::RestoredAfterExit, now)
    }

    pub(crate) fn next_due(&self) -> Option<Instant> {
        let pending = self.pending.map(|pending| pending.cap);
        let retry = self.retry.map(|retry| retry.due);
        match (pending, retry) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub(crate) fn handle(&mut self, command: Command, now: Now) -> Vec<Event> {
        let mut events = Vec::new();
        match command {
            Command::Arm {
                recording_id,
                enabled,
                at,
            } => {
                // A previous generation is always released before a new start;
                // if one somehow is not, restore it rather than leak a mute.
                if let Some(engaged) = self.engaged.take() {
                    events.push(self.restore_live(engaged, now));
                }
                self.pending = None;
                events.extend(self.retry_outstanding(recording_id, Outcome::Restored, now));
                if enabled {
                    self.pending = Some(Pending {
                        recording_id,
                        started: at,
                        cap: at + START_CUE_CAP,
                    });
                }
            }
            Command::CueFinished { recording_id } => {
                if let Some(pending) = self
                    .pending
                    .filter(|pending| pending.recording_id == recording_id)
                {
                    self.pending = None;
                    events.push(self.engage(pending, Trigger::CueFinished, now));
                }
            }
            Command::Release { recording_id } => {
                if self
                    .pending
                    .is_some_and(|pending| pending.recording_id == recording_id)
                {
                    self.pending = None;
                }
                if self
                    .engaged
                    .as_ref()
                    .is_some_and(|engaged| engaged.recording_id == recording_id)
                {
                    let engaged = self.engaged.take().expect("checked above");
                    events.push(self.restore_live(engaged, now));
                }
            }
            Command::Shutdown(_) => events.extend(self.shutdown(now)),
        }
        events.extend(self.poll(now));
        events
    }

    /// Run a capped engage or a due retry.
    pub(crate) fn poll(&mut self, now: Now) -> Vec<Event> {
        let mut events = Vec::new();
        if let Some(pending) = self.pending.filter(|pending| pending.cap <= now.instant) {
            self.pending = None;
            events.push(self.engage(pending, Trigger::Cap, now));
        }
        if let Some(retry) = self.retry.filter(|retry| retry.due <= now.instant) {
            self.retry = Some(Retry {
                due: now.instant,
                attempt: retry.attempt + 1,
            });
            events.extend(self.retry_outstanding(0, Outcome::Restored, now));
        }
        events
    }

    /// App exit: never leave the speakers muted behind a clean quit.
    pub(crate) fn shutdown(&mut self, now: Now) -> Vec<Event> {
        self.pending = None;
        self.retry = None;
        match self.engaged.take() {
            Some(engaged) => vec![self.restore_live(engaged, now)],
            None => Vec::new(),
        }
    }

    fn persist(&mut self) -> bool {
        let mut records = self.outstanding.clone();
        if let Some(engaged) = &self.engaged {
            records.push(engaged.record.clone());
        }
        self.store.save(&records)
    }

    fn engage(&mut self, pending: Pending, trigger: Trigger, now: Now) -> Event {
        let started = Instant::now();
        let recording_id = pending.recording_id;
        let since_start_ms = now
            .instant
            .saturating_duration_since(pending.started)
            .as_millis() as u64;
        let event = |outcome, transport| Event {
            recording_id,
            outcome,
            transport,
            trigger: Some(trigger),
            since_start_ms: Some(since_start_ms),
            hal_us: started.elapsed().as_micros() as u64,
        };
        let Some(device) = self.hal.default_output() else {
            return event(Outcome::SkippedNoOutput, None);
        };
        let transport = Some(device.transport);
        match route(&device) {
            Route::Bluetooth => return event(Outcome::SkippedBluetooth, transport),
            Route::Headphones => return event(Outcome::SkippedHeadphones, transport),
            Route::Speakers => {}
        }
        let Some(elements) = self
            .hal
            .mute_elements(device.id)
            .filter(|elements| !elements.is_empty() && elements.len() <= RECORD_MAX_ELEMENTS)
        else {
            return event(Outcome::SkippedNoMuteControl, transport);
        };
        let Some(states) = self
            .hal
            .muted(device.id, &elements)
            .filter(|states| states.len() == elements.len())
        else {
            return event(Outcome::SkippedNoMuteControl, transport);
        };
        // Mute only what is audible now, so a restore can never unmute an
        // element the user had muted before the dictation.
        let to_mute: Vec<u32> = elements
            .iter()
            .zip(&states)
            .filter(|(_, muted)| !**muted)
            .map(|(element, _)| *element)
            .collect();
        if to_mute.is_empty() {
            return event(Outcome::SkippedAlreadyMuted, transport);
        }
        if !self.hal.set_muted(device.id, &to_mute, true) {
            // A partial write leaves some elements muted; they were audible a
            // moment ago, so unmuting them returns the device to its state.
            let _ = self.hal.set_muted(device.id, &to_mute, false);
            return event(Outcome::MuteFailed, transport);
        }
        // The record is written only once the mute is applied. A crash in the
        // moment between leaves the Mac muted, the safe direction.
        self.engaged = Some(Engaged {
            recording_id,
            record: QuietRecord {
                device_uid: device.uid,
                elements: to_mute.clone(),
                muted_at_unix_ms: now.unix_ms,
            },
        });
        if !self.persist() {
            // Without a record a crash could not be recovered: undo now.
            self.engaged = None;
            let _ = self.hal.set_muted(device.id, &to_mute, false);
            let _ = self.persist();
            return event(Outcome::SkippedRecordUnavailable, transport);
        }
        event(Outcome::Engaged, transport)
    }

    /// Restore the live generation's mute. Never subject to expiry: this
    /// process applied it moments or minutes ago and still owns it. If the
    /// restore cannot finish, the record's expiry clock starts now, when this
    /// process lets go of the mute, so a long dictation still gets the full
    /// retry window. Older records for the same device are dropped first:
    /// their mute ended when this dictation took the device over, so retrying
    /// them later could only undo a mute the user set after this restore.
    fn restore_live(&mut self, engaged: Engaged, now: Now) -> Event {
        let mut record = engaged.record;
        self.outstanding
            .retain(|older| older.device_uid != record.device_uid);
        record.muted_at_unix_ms = now.unix_ms;
        let event = self.restore(engaged.recording_id, record, Outcome::Restored);
        let _ = self.persist();
        self.schedule_retry(now);
        event
    }

    /// One restore attempt. Records that cannot finish return to `outstanding`.
    fn restore(&mut self, recording_id: u64, record: QuietRecord, success: Outcome) -> Event {
        let started = Instant::now();
        let outcome = match self.hal.device_for_uid(&record.device_uid) {
            None => Outcome::DeviceMissing,
            Some(device) => match self.hal.muted(device, &record.elements) {
                Some(states) if states.len() == record.elements.len() => {
                    if !states.iter().all(|muted| *muted) {
                        Outcome::UserChanged
                    } else if self.hal.set_muted(device, &record.elements, false) {
                        success
                    } else {
                        Outcome::RestoreFailed
                    }
                }
                // Mid-removal or not yet ready: the device may still be muted.
                _ => Outcome::Unreadable,
            },
        };
        if outcome.keeps_record() {
            self.keep_outstanding(record);
        }
        Event {
            recording_id,
            outcome,
            transport: None,
            trigger: None,
            since_start_ms: None,
            hal_us: started.elapsed().as_micros() as u64,
        }
    }

    fn keep_outstanding(&mut self, record: QuietRecord) {
        self.outstanding.retain(|existing| existing != &record);
        self.outstanding.push(record);
        let excess = self.outstanding.len().saturating_sub(RECORD_MAX_ENTRIES);
        self.outstanding.drain(..excess);
    }

    /// Retry every outstanding record that is still fresh; drop expired ones
    /// without touching their device. A record for the device the live
    /// dictation has muted keeps waiting: its mute is now the dictation's,
    /// and the dictation's own restore schedules the next retry.
    fn retry_outstanding(&mut self, recording_id: u64, success: Outcome, now: Now) -> Vec<Event> {
        let records = std::mem::take(&mut self.outstanding);
        let had_records = !records.is_empty();
        let live_uid = self
            .engaged
            .as_ref()
            .map(|engaged| engaged.record.device_uid.clone());
        let mut events = Vec::with_capacity(records.len());
        for record in records {
            if !record.is_fresh(now.unix_ms) {
                events.push(Event {
                    recording_id,
                    outcome: Outcome::Expired,
                    transport: None,
                    trigger: None,
                    since_start_ms: None,
                    hal_us: 0,
                });
            } else if live_uid.as_deref() == Some(record.device_uid.as_str()) {
                self.keep_outstanding(record);
            } else {
                events.push(self.restore(recording_id, record, success));
            }
        }
        if had_records {
            let _ = self.persist();
        }
        self.schedule_retry(now);
        // A device that stays unplugged is retried quietly; its first miss
        // and its expiry are what get logged.
        if self.retry.is_some_and(|retry| retry.attempt >= 1) {
            events.retain(|event| event.outcome != Outcome::DeviceMissing);
        }
        events
    }

    fn schedule_retry(&mut self, now: Now) {
        if self.outstanding.is_empty() {
            self.retry = None;
            return;
        }
        let attempt = self.retry.map_or(0, |retry| retry.attempt);
        let delay = RETRY_DELAYS[attempt.min(RETRY_DELAYS.len() - 1)];
        self.retry = Some(Retry {
            due: now.instant + delay,
            attempt,
        });
    }
}

// ---------------------------------------------------------------------------
// Status seam and worker
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Signal {
    Arm,
    Release,
}

/// Map a dictation status transition to a controller signal. Fn key-down
/// capture that is discarded never leaves Idle, file transcription goes
/// Idle -> Processing without an Arm, and meetings and the microphone test do
/// not use dictation status, so none of them can engage. Recording is only
/// entered from Starting, so it needs no signal of its own.
pub(crate) fn signal_for(from: DictationStatus, to: DictationStatus) -> Option<Signal> {
    if from == to {
        return None;
    }
    match to {
        DictationStatus::Starting if from == DictationStatus::Idle => Some(Signal::Arm),
        DictationStatus::Starting | DictationStatus::Recording => None,
        DictationStatus::Recovering | DictationStatus::Processing | DictationStatus::Idle => {
            Some(Signal::Release)
        }
    }
}

static WORKER: OnceLock<mpsc::Sender<Command>> = OnceLock::new();

fn send(command: Command) {
    if let Some(sender) = WORKER.get() {
        let _ = sender.send(command);
    }
}

/// Called from the dictation status seam with the dictation lock held. Only
/// sends a message; a missing worker (unit tests, setup failure) is a no-op.
pub(crate) fn observe_transition(
    recording_id: u64,
    from: DictationStatus,
    to: DictationStatus,
    enabled: bool,
) {
    if recording_id == 0 {
        return;
    }
    match signal_for(from, to) {
        Some(Signal::Arm) => send(Command::Arm {
            recording_id,
            enabled,
            at: Instant::now(),
        }),
        Some(Signal::Release) => send(Command::Release { recording_id }),
        None => {}
    }
}

/// The main window finished this generation's start cue, or played none.
pub(crate) fn start_cue_finished(recording_id: u64) {
    if recording_id != 0 {
        send(Command::CueFinished { recording_id });
    }
}

fn log(events: Vec<Event>) {
    for event in events {
        tracing::info!(
            target: "audio",
            event_code = "audio.output_quieting",
            recording_id = event.recording_id,
            outcome = event.outcome.code(),
            transport = event.transport.map(Transport::label).unwrap_or("none"),
            trigger = event.trigger.map(Trigger::label).unwrap_or("none"),
            since_start_ms = event.since_start_ms.unwrap_or(0),
            hal_us = event.hal_us,
            "output quieting decision"
        );
    }
}

fn run<H: OutputHal, S: RecordStore>(
    mut controller: Controller<H, S>,
    rx: mpsc::Receiver<Command>,
) {
    log(controller.recover_after_exit(Now::current()));
    loop {
        let received = match controller.next_due() {
            Some(due) => match rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            },
            None => match rx.recv() {
                Ok(command) => Some(command),
                Err(_) => break,
            },
        };
        match received {
            Some(Command::Shutdown(ack)) => {
                log(controller.shutdown(Now::current()));
                let _ = ack.send(());
                break;
            }
            Some(command) => log(controller.handle(command, Now::current())),
            None => log(controller.poll(Now::current())),
        }
    }
}

/// Start the worker and restore anything a previous process left muted. Runs
/// once during setup; recovery happens on the worker, never on setup's thread.
pub(crate) fn initialize(data_dir: Option<PathBuf>) {
    let (tx, rx) = mpsc::channel();
    if WORKER.set(tx).is_err() {
        return;
    }
    let controller = Controller::new(platform_hal(), FileStore::new(data_dir));
    if let Err(error) = std::thread::Builder::new()
        .name("murmur-output-quieting".into())
        .spawn(move || run(controller, rx))
    {
        tracing::warn!(target: "audio", "output quieting worker unavailable: {error}");
    }
}

/// Bounded synchronous restore for `RunEvent::Exit`.
pub(crate) fn shutdown() {
    let Some(sender) = WORKER.get() else {
        return;
    };
    let (ack_tx, ack_rx) = mpsc::channel();
    if sender.send(Command::Shutdown(ack_tx)).is_ok() {
        let _ = ack_rx.recv_timeout(SHUTDOWN_WAIT);
    }
}

#[cfg(target_os = "macos")]
fn platform_hal() -> core_audio::CoreAudioHal {
    core_audio::CoreAudioHal
}

#[cfg(not(target_os = "macos"))]
fn platform_hal() -> NoOutputHal {
    NoOutputHal
}

#[cfg(not(target_os = "macos"))]
pub(crate) struct NoOutputHal;

#[cfg(not(target_os = "macos"))]
impl OutputHal for NoOutputHal {
    fn default_output(&mut self) -> Option<OutputDevice> {
        None
    }
    fn device_for_uid(&mut self, _uid: &str) -> Option<u32> {
        None
    }
    fn mute_elements(&mut self, _device: u32) -> Option<Vec<u32>> {
        None
    }
    fn muted(&mut self, _device: u32, _elements: &[u32]) -> Option<Vec<bool>> {
        None
    }
    fn set_muted(&mut self, _device: u32, _elements: &[u32], _muted: bool) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Core Audio HAL
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod core_audio {
    //! Property reads and writes only. No audio unit, stream, or IOProc is
    //! created, so nothing here can start capture or voice-processing ducking.

    use std::ffi::c_void;
    use std::ptr::NonNull;

    use objc2_core_audio::{
        kAudioDevicePropertyDataSource, kAudioDevicePropertyDeviceUID, kAudioDevicePropertyMute,
        kAudioDevicePropertyPreferredChannelsForStereo, kAudioDevicePropertyTransportType,
        kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyTranslateUIDToDevice,
        kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject, AudioObjectGetPropertyData,
        AudioObjectHasProperty, AudioObjectID, AudioObjectIsPropertySettable,
        AudioObjectPropertyAddress, AudioObjectSetPropertyData,
    };
    use objc2_core_foundation::{CFRetained, CFString};

    use super::{OutputDevice, OutputHal, Transport, DATA_SOURCE_HEADPHONES};

    pub(crate) struct CoreAudioHal;

    fn address(selector: u32, scope: u32, element: u32) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: scope,
            mElement: element,
        }
    }

    fn system() -> AudioObjectID {
        kAudioObjectSystemObject as AudioObjectID
    }

    fn read<T: Copy + Default>(
        object: AudioObjectID,
        address: AudioObjectPropertyAddress,
    ) -> Option<T> {
        let mut value = T::default();
        let mut size = std::mem::size_of::<T>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                object,
                NonNull::from(&address),
                0,
                std::ptr::null(),
                NonNull::from(&mut size),
                NonNull::from(&mut value).cast::<c_void>(),
            )
        };
        (status == 0 && size as usize == std::mem::size_of::<T>()).then_some(value)
    }

    fn write_u32(object: AudioObjectID, address: AudioObjectPropertyAddress, value: u32) -> bool {
        let status = unsafe {
            AudioObjectSetPropertyData(
                object,
                NonNull::from(&address),
                0,
                std::ptr::null(),
                std::mem::size_of::<u32>() as u32,
                NonNull::from(&value).cast::<c_void>(),
            )
        };
        status == 0
    }

    fn settable(object: AudioObjectID, address: AudioObjectPropertyAddress) -> bool {
        if !unsafe { AudioObjectHasProperty(object, NonNull::from(&address)) } {
            return false;
        }
        let mut is_settable: u8 = 0;
        let status = unsafe {
            AudioObjectIsPropertySettable(
                object,
                NonNull::from(&address),
                NonNull::from(&mut is_settable),
            )
        };
        status == 0 && is_settable != 0
    }

    fn read_uid(device: AudioObjectID) -> Option<String> {
        let address = address(
            kAudioDevicePropertyDeviceUID,
            kAudioObjectPropertyScopeGlobal,
            kAudioObjectPropertyElementMain,
        );
        let mut value: *const CFString = std::ptr::null();
        let mut size = std::mem::size_of::<*const CFString>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                device,
                NonNull::from(&address),
                0,
                std::ptr::null(),
                NonNull::from(&mut size),
                NonNull::from(&mut value).cast::<c_void>(),
            )
        };
        // A short read would leave a stale pointer; never adopt it.
        if status != 0 || size as usize != std::mem::size_of::<*const CFString>() {
            return None;
        }
        let value = NonNull::new(value.cast_mut())?;
        // The HAL returns a +1 reference for this property.
        let value: CFRetained<CFString> = unsafe { CFRetained::from_raw(value) };
        Some(value.to_string())
    }

    fn mute_address(element: u32) -> AudioObjectPropertyAddress {
        address(
            kAudioDevicePropertyMute,
            kAudioObjectPropertyScopeOutput,
            element,
        )
    }

    impl OutputHal for CoreAudioHal {
        fn default_output(&mut self) -> Option<OutputDevice> {
            let id: AudioObjectID = read(
                system(),
                address(
                    kAudioHardwarePropertyDefaultOutputDevice,
                    kAudioObjectPropertyScopeGlobal,
                    kAudioObjectPropertyElementMain,
                ),
            )?;
            if id == 0 {
                return None;
            }
            let uid = read_uid(id)?;
            let transport = Transport::from_code(read(
                id,
                address(
                    kAudioDevicePropertyTransportType,
                    kAudioObjectPropertyScopeGlobal,
                    kAudioObjectPropertyElementMain,
                ),
            ));
            let data_source: Option<u32> = read(
                id,
                address(
                    kAudioDevicePropertyDataSource,
                    kAudioObjectPropertyScopeOutput,
                    kAudioObjectPropertyElementMain,
                ),
            );
            Some(OutputDevice {
                id,
                uid,
                transport,
                headphone_source: data_source == Some(DATA_SOURCE_HEADPHONES),
            })
        }

        fn device_for_uid(&mut self, uid: &str) -> Option<u32> {
            let cf_uid = CFString::from_str(uid);
            let uid_ref: *const CFString = &*cf_uid;
            let address = address(
                kAudioHardwarePropertyTranslateUIDToDevice,
                kAudioObjectPropertyScopeGlobal,
                kAudioObjectPropertyElementMain,
            );
            let mut device: AudioObjectID = 0;
            let mut size = std::mem::size_of::<AudioObjectID>() as u32;
            let status = unsafe {
                AudioObjectGetPropertyData(
                    system(),
                    NonNull::from(&address),
                    std::mem::size_of::<*const CFString>() as u32,
                    (&uid_ref as *const *const CFString).cast::<c_void>(),
                    NonNull::from(&mut size),
                    NonNull::from(&mut device).cast::<c_void>(),
                )
            };
            (status == 0 && device != 0).then_some(device)
        }

        fn mute_elements(&mut self, device: u32) -> Option<Vec<u32>> {
            if settable(device, mute_address(kAudioObjectPropertyElementMain)) {
                return Some(vec![kAudioObjectPropertyElementMain]);
            }
            let channels: [u32; 2] = read(
                device,
                address(
                    kAudioDevicePropertyPreferredChannelsForStereo,
                    kAudioObjectPropertyScopeOutput,
                    kAudioObjectPropertyElementMain,
                ),
            )?;
            let mut elements: Vec<u32> = channels
                .into_iter()
                .filter(|channel| *channel != 0)
                .collect();
            elements.dedup();
            (!elements.is_empty()
                && elements
                    .iter()
                    .all(|element| settable(device, mute_address(*element))))
            .then_some(elements)
        }

        fn muted(&mut self, device: u32, elements: &[u32]) -> Option<Vec<bool>> {
            elements
                .iter()
                .map(|element| read::<u32>(device, mute_address(*element)).map(|value| value != 0))
                .collect()
        }

        fn set_muted(&mut self, device: u32, elements: &[u32], muted: bool) -> bool {
            let mut all = true;
            for element in elements {
                all &= write_u32(device, mute_address(*element), u32::from(muted));
            }
            all
        }
    }

    #[cfg(test)]
    mod tests {
        use super::super::Transport;
        use objc2_core_audio::{
            kAudioDeviceTransportTypeAggregate, kAudioDeviceTransportTypeAirPlay,
            kAudioDeviceTransportTypeAutoAggregate, kAudioDeviceTransportTypeBluetooth,
            kAudioDeviceTransportTypeBluetoothLE, kAudioDeviceTransportTypeBuiltIn,
            kAudioDeviceTransportTypeDisplayPort, kAudioDeviceTransportTypeHDMI,
            kAudioDeviceTransportTypeThunderbolt, kAudioDeviceTransportTypeUSB,
            kAudioDeviceTransportTypeVirtual,
        };

        #[test]
        fn transport_codes_match_the_sdk() {
            for (code, expected) in [
                (kAudioDeviceTransportTypeBuiltIn, Transport::BuiltIn),
                (kAudioDeviceTransportTypeBluetooth, Transport::Bluetooth),
                (kAudioDeviceTransportTypeBluetoothLE, Transport::Bluetooth),
                (kAudioDeviceTransportTypeUSB, Transport::Usb),
                (kAudioDeviceTransportTypeHDMI, Transport::Hdmi),
                (kAudioDeviceTransportTypeDisplayPort, Transport::DisplayPort),
                (kAudioDeviceTransportTypeAirPlay, Transport::AirPlay),
                (kAudioDeviceTransportTypeAggregate, Transport::Aggregate),
                (kAudioDeviceTransportTypeAutoAggregate, Transport::Aggregate),
                (kAudioDeviceTransportTypeVirtual, Transport::Virtual),
                (kAudioDeviceTransportTypeThunderbolt, Transport::Thunderbolt),
            ] {
                assert_eq!(Transport::from_code(Some(code)), expected);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Debug, Clone)]
    struct FakeDevice {
        uid: String,
        transport: Transport,
        headphone_source: bool,
        /// `None`: no settable mute.
        elements: Option<Vec<u32>>,
        muted: HashMap<u32, bool>,
        present: bool,
        /// The device is listed but its mute cannot be read (mid-removal).
        read_fails: bool,
    }

    #[derive(Default)]
    struct FakeHal {
        devices: HashMap<u32, FakeDevice>,
        default: Option<u32>,
        fail_mute: bool,
        fail_unmute: bool,
        writes: Vec<(u32, Vec<u32>, bool)>,
        queries: usize,
    }

    impl FakeHal {
        fn speakers() -> Self {
            let mut hal = Self::default();
            hal.add(1, "speakers", Transport::BuiltIn, false, Some(vec![0]));
            hal.default = Some(1);
            hal
        }

        fn add(
            &mut self,
            id: u32,
            uid: &str,
            transport: Transport,
            headphone_source: bool,
            elements: Option<Vec<u32>>,
        ) {
            let muted = elements
                .iter()
                .flatten()
                .map(|element| (*element, false))
                .collect();
            self.devices.insert(
                id,
                FakeDevice {
                    uid: uid.into(),
                    transport,
                    headphone_source,
                    elements,
                    muted,
                    present: true,
                    read_fails: false,
                },
            );
        }

        fn device(&mut self, id: u32) -> &mut FakeDevice {
            self.devices.get_mut(&id).unwrap()
        }

        fn is_muted(&self, id: u32) -> bool {
            self.devices[&id].muted.values().all(|muted| *muted)
        }

        fn user_sets(&mut self, id: u32, muted: bool) {
            for value in self.device(id).muted.values_mut() {
                *value = muted;
            }
        }
    }

    impl OutputHal for FakeHal {
        fn default_output(&mut self) -> Option<OutputDevice> {
            self.queries += 1;
            let id = self.default?;
            let device = self.devices.get(&id).filter(|device| device.present)?;
            Some(OutputDevice {
                id,
                uid: device.uid.clone(),
                transport: device.transport,
                headphone_source: device.headphone_source,
            })
        }

        fn device_for_uid(&mut self, uid: &str) -> Option<u32> {
            self.devices
                .iter()
                .find(|(_, device)| device.present && device.uid == uid)
                .map(|(id, _)| *id)
        }

        fn mute_elements(&mut self, device: u32) -> Option<Vec<u32>> {
            self.devices.get(&device)?.elements.clone()
        }

        fn muted(&mut self, device: u32, elements: &[u32]) -> Option<Vec<bool>> {
            let device = self.devices.get(&device)?;
            if device.read_fails {
                return None;
            }
            elements
                .iter()
                .map(|element| device.muted.get(element).copied())
                .collect()
        }

        fn set_muted(&mut self, device: u32, elements: &[u32], muted: bool) -> bool {
            self.writes.push((device, elements.to_vec(), muted));
            if (self.fail_mute && muted) || (self.fail_unmute && !muted) {
                return false;
            }
            let Some(device) = self.devices.get_mut(&device) else {
                return false;
            };
            for element in elements {
                device.muted.insert(*element, muted);
            }
            true
        }
    }

    #[derive(Default)]
    struct MemoryStore {
        records: Vec<QuietRecord>,
        fail_save: bool,
        saves: usize,
    }

    impl RecordStore for MemoryStore {
        fn load(&mut self) -> Vec<QuietRecord> {
            self.records.clone()
        }
        fn save(&mut self, records: &[QuietRecord]) -> bool {
            self.saves += 1;
            if self.fail_save {
                return false;
            }
            self.records = records.to_vec();
            true
        }
    }

    type TestController = Controller<FakeHal, MemoryStore>;

    /// A fixed origin; tests move both clocks together.
    struct Clock {
        instant: Instant,
        unix_ms: u64,
    }

    impl Clock {
        fn new() -> Self {
            Self {
                instant: Instant::now(),
                unix_ms: 1_800_000_000_000,
            }
        }
        fn at(&self, offset: Duration) -> Now {
            Now {
                instant: self.instant + offset,
                unix_ms: self.unix_ms + offset.as_millis() as u64,
            }
        }
    }

    const MS: fn(u64) -> Duration = Duration::from_millis;

    fn controller(hal: FakeHal) -> TestController {
        Controller::new(hal, MemoryStore::default())
    }

    fn outcomes(events: &[Event]) -> Vec<Outcome> {
        events.iter().map(|event| event.outcome).collect()
    }

    fn record(uid: &str, muted_at_unix_ms: u64) -> QuietRecord {
        QuietRecord {
            device_uid: uid.into(),
            elements: vec![0],
            muted_at_unix_ms,
        }
    }

    fn arm(
        c: &mut TestController,
        rid: u64,
        enabled: bool,
        clock: &Clock,
        at: Duration,
    ) -> Vec<Event> {
        c.handle(
            Command::Arm {
                recording_id: rid,
                enabled,
                at: clock.at(at).instant,
            },
            clock.at(at),
        )
    }

    fn cue(c: &mut TestController, rid: u64, clock: &Clock, at: Duration) -> Vec<Event> {
        c.handle(Command::CueFinished { recording_id: rid }, clock.at(at))
    }

    fn release(c: &mut TestController, rid: u64, clock: &Clock, at: Duration) -> Vec<Event> {
        c.handle(Command::Release { recording_id: rid }, clock.at(at))
    }

    /// Production ordering: Starting (Arm), the cue report ~110 ms later.
    fn start(c: &mut TestController, rid: u64, clock: &Clock) -> Vec<Event> {
        let mut events = arm(c, rid, true, clock, MS(0));
        events.extend(cue(c, rid, clock, MS(110)));
        events
    }

    #[test]
    fn mutes_when_the_start_cue_finishes_and_restores_at_stop() {
        let clock = Clock::new();
        let mut c = controller(FakeHal::speakers());
        assert!(arm(&mut c, 7, true, &clock, MS(0)).is_empty());
        // Nothing before the cue report, even past its 70 ms tone.
        assert!(c.poll(clock.at(MS(100))).is_empty());
        assert!(!c.hal.is_muted(1));
        assert_eq!(c.next_due(), Some(clock.at(START_CUE_CAP).instant));

        let events = cue(&mut c, 7, &clock, MS(120));
        assert_eq!(outcomes(&events), vec![Outcome::Engaged]);
        assert_eq!(events[0].trigger, Some(Trigger::CueFinished));
        assert_eq!(events[0].since_start_ms, Some(120));
        assert_eq!(events[0].transport, Some(Transport::BuiltIn));
        assert!(c.hal.is_muted(1));
        assert_eq!(
            c.store.records,
            vec![record("speakers", clock.at(MS(120)).unix_ms)]
        );
        assert!(c.next_due().is_none());

        assert_eq!(
            outcomes(&release(&mut c, 7, &clock, MS(4000))),
            vec![Outcome::Restored]
        );
        assert!(!c.hal.is_muted(1));
        assert!(c.store.records.is_empty());
    }

    #[test]
    fn a_missing_cue_report_mutes_at_the_cap() {
        let clock = Clock::new();
        let mut c = controller(FakeHal::speakers());
        arm(&mut c, 1, true, &clock, MS(0));
        assert!(c.poll(clock.at(START_CUE_CAP - MS(1))).is_empty());
        let events = c.poll(clock.at(START_CUE_CAP));
        assert_eq!(outcomes(&events), vec![Outcome::Engaged]);
        assert_eq!(events[0].trigger, Some(Trigger::Cap));
        // A late report changes nothing.
        assert!(cue(&mut c, 1, &clock, MS(900)).is_empty());
        assert_eq!(c.hal.writes.len(), 1);
    }

    #[test]
    fn the_cap_outlasts_the_start_cue() {
        // soundCues.ts: start cue 70 ms; useSoundCues adds output latency and
        // a 30 ms margin. The cap must leave room for a slow webview on top.
        assert!(START_CUE_CAP >= MS(70 + 30) * 4);
    }

    #[test]
    fn each_generation_decides_once() {
        // Headphones: skipped once, and repeated reports never query again.
        let mut hal = FakeHal::default();
        hal.add(9, "out", Transport::BuiltIn, true, Some(vec![0]));
        hal.default = Some(9);
        let clock = Clock::new();
        let mut c = controller(hal);
        let events = start(&mut c, 1, &clock);
        assert_eq!(outcomes(&events), vec![Outcome::SkippedHeadphones]);
        assert!(cue(&mut c, 1, &clock, MS(160)).is_empty());
        assert!(c.poll(clock.at(MS(600))).is_empty());
        assert_eq!(c.hal.queries, 1);

        // A failed mute is not attempted a second time either.
        let mut hal = FakeHal::speakers();
        hal.fail_mute = true;
        let mut c = controller(hal);
        assert_eq!(
            outcomes(&start(&mut c, 1, &clock)),
            vec![Outcome::MuteFailed]
        );
        assert!(cue(&mut c, 1, &clock, MS(160)).is_empty());
        assert!(c.poll(clock.at(MS(600))).is_empty());
        assert_eq!(
            c.hal.writes.iter().filter(|(_, _, muted)| *muted).count(),
            1
        );
    }

    #[test]
    fn the_record_is_written_after_the_mute_and_a_failed_write_undoes_it() {
        let clock = Clock::new();
        let mut c = controller(FakeHal::speakers());
        c.store.fail_save = true;
        let events = start(&mut c, 1, &clock);
        assert_eq!(outcomes(&events), vec![Outcome::SkippedRecordUnavailable]);
        assert_eq!(
            c.hal.writes,
            vec![(1, vec![0], true), (1, vec![0], false)],
            "muted, then undone because a crash could not be recovered"
        );
        assert!(!c.hal.is_muted(1));
        assert!(release(&mut c, 1, &clock, MS(500)).is_empty());
    }

    #[test]
    fn a_failed_mute_writes_no_record() {
        let clock = Clock::new();
        let mut hal = FakeHal::speakers();
        hal.fail_mute = true;
        let mut c = controller(hal);
        assert_eq!(
            outcomes(&start(&mut c, 1, &clock)),
            vec![Outcome::MuteFailed]
        );
        assert_eq!(c.store.saves, 0);
        assert!(release(&mut c, 1, &clock, MS(500)).is_empty());
    }

    #[test]
    fn disabled_setting_never_touches_the_device_or_disk() {
        let clock = Clock::new();
        let mut c = controller(FakeHal::speakers());
        assert!(arm(&mut c, 1, false, &clock, MS(0)).is_empty());
        assert!(cue(&mut c, 1, &clock, MS(100)).is_empty());
        assert!(c.poll(clock.at(MS(900))).is_empty());
        assert!(release(&mut c, 1, &clock, MS(1000)).is_empty());
        assert!(c.hal.writes.is_empty());
        assert_eq!(c.hal.queries, 0);
        assert_eq!(c.store.saves, 0);
    }

    #[test]
    fn a_stop_before_the_cue_finishes_cancels_the_mute() {
        let clock = Clock::new();
        let mut c = controller(FakeHal::speakers());
        arm(&mut c, 3, true, &clock, MS(0));
        assert!(release(&mut c, 3, &clock, MS(60)).is_empty());
        assert!(c.next_due().is_none());
        assert!(cue(&mut c, 3, &clock, MS(110)).is_empty());
        assert!(c.poll(clock.at(MS(900))).is_empty());
        assert!(c.hal.writes.is_empty());
    }

    #[test]
    fn user_unmuting_during_the_dictation_is_left_alone() {
        let clock = Clock::new();
        let mut c = controller(FakeHal::speakers());
        start(&mut c, 2, &clock);
        c.hal.user_sets(1, false);
        let writes_before = c.hal.writes.len();
        assert_eq!(
            outcomes(&release(&mut c, 2, &clock, MS(3000))),
            vec![Outcome::UserChanged]
        );
        assert_eq!(c.hal.writes.len(), writes_before);
        assert!(c.store.records.is_empty());
    }

    #[test]
    fn a_device_change_restores_the_original_and_leaves_the_new_one() {
        let clock = Clock::new();
        let mut hal = FakeHal::speakers();
        hal.add(2, "display", Transport::Hdmi, false, Some(vec![0]));
        let mut c = controller(hal);
        start(&mut c, 4, &clock);
        c.hal.default = Some(2);
        assert_eq!(
            outcomes(&release(&mut c, 4, &clock, MS(3000))),
            vec![Outcome::Restored]
        );
        assert!(!c.hal.is_muted(1));
        assert!(!c.hal.is_muted(2));
        assert!(c.hal.writes.iter().all(|(device, _, _)| *device == 1));
    }

    #[test]
    fn no_software_mute_means_no_change() {
        let clock = Clock::new();
        let mut hal = FakeHal::default();
        hal.add(5, "hdmi", Transport::Hdmi, false, None);
        hal.default = Some(5);
        let mut c = controller(hal);
        assert_eq!(
            outcomes(&start(&mut c, 1, &clock)),
            vec![Outcome::SkippedNoMuteControl]
        );
        assert!(c.hal.writes.is_empty());
        assert_eq!(c.store.saves, 0);
        assert!(release(&mut c, 1, &clock, MS(500)).is_empty());
    }

    #[test]
    fn per_channel_mute_is_used_when_there_is_no_main_element() {
        let clock = Clock::new();
        let mut hal = FakeHal::default();
        hal.add(6, "usb", Transport::Usb, false, Some(vec![1, 2]));
        hal.default = Some(6);
        let mut c = controller(hal);
        start(&mut c, 1, &clock);
        assert!(c.hal.is_muted(6));
        assert_eq!(c.store.records[0].elements, vec![1, 2]);
        release(&mut c, 1, &clock, MS(500));
        assert!(!c.hal.is_muted(6));
    }

    #[test]
    fn headphones_and_bluetooth_are_skipped() {
        for (transport, headphone_source, uid, expected) in [
            (Transport::BuiltIn, true, "out", Outcome::SkippedHeadphones),
            (
                Transport::BuiltIn,
                false,
                BUILT_IN_HEADPHONE_UID,
                Outcome::SkippedHeadphones,
            ),
            (
                Transport::Bluetooth,
                false,
                "out",
                Outcome::SkippedBluetooth,
            ),
        ] {
            let clock = Clock::new();
            let mut hal = FakeHal::default();
            hal.add(9, uid, transport, headphone_source, Some(vec![0]));
            hal.default = Some(9);
            let mut c = controller(hal);
            assert_eq!(outcomes(&start(&mut c, 1, &clock)), vec![expected]);
            assert!(c.hal.writes.is_empty());
        }
    }

    #[test]
    fn unclear_routes_and_usb_headsets_still_mute() {
        for transport in [
            Transport::Unknown,
            Transport::Aggregate,
            Transport::Virtual,
            Transport::AirPlay,
            Transport::DisplayPort,
            Transport::Usb,
        ] {
            let clock = Clock::new();
            let mut hal = FakeHal::default();
            hal.add(9, "out", transport, false, Some(vec![0]));
            hal.default = Some(9);
            let mut c = controller(hal);
            assert_eq!(
                outcomes(&start(&mut c, 1, &clock)),
                vec![Outcome::Engaged],
                "{transport:?}"
            );
        }
    }

    #[test]
    fn an_already_muted_output_is_not_recorded_or_unmuted() {
        let clock = Clock::new();
        let mut hal = FakeHal::speakers();
        hal.user_sets(1, true);
        let mut c = controller(hal);
        assert_eq!(
            outcomes(&start(&mut c, 1, &clock)),
            vec![Outcome::SkippedAlreadyMuted]
        );
        assert!(release(&mut c, 1, &clock, MS(500)).is_empty());
        assert!(c.hal.is_muted(1));
        assert!(c.hal.writes.is_empty());
    }

    #[test]
    fn only_audible_channels_are_muted_and_restored() {
        let clock = Clock::new();
        let mut hal = FakeHal::default();
        hal.add(6, "usb", Transport::Usb, false, Some(vec![1, 2]));
        hal.default = Some(6);
        hal.device(6).muted.insert(2, true);
        let mut c = controller(hal);
        start(&mut c, 1, &clock);
        assert_eq!(c.store.records[0].elements, vec![1]);
        release(&mut c, 1, &clock, MS(500));
        assert!(!c.hal.devices[&6].muted[&1]);
        assert!(c.hal.devices[&6].muted[&2]);
    }

    #[test]
    fn crash_recovery_restores_a_fresh_record() {
        let clock = Clock::new();
        let mut hal = FakeHal::speakers();
        hal.user_sets(1, true);
        let mut c = controller(hal);
        c.store.records = vec![record("speakers", clock.unix_ms)];
        let events = c.recover_after_exit(clock.at(Duration::from_secs(90)));
        assert_eq!(outcomes(&events), vec![Outcome::RestoredAfterExit]);
        assert_eq!(events[0].recording_id, 0);
        assert!(!c.hal.is_muted(1));
        assert!(c.store.records.is_empty());
    }

    #[test]
    fn an_expired_record_is_dropped_without_touching_the_device() {
        let clock = Clock::new();
        let mut hal = FakeHal::speakers();
        // The user muted on purpose after the crash.
        hal.user_sets(1, true);
        let mut c = controller(hal);
        c.store.records = vec![record("speakers", clock.unix_ms)];
        let events = c.recover_after_exit(clock.at(RECORD_MAX_AGE + MS(1)));
        assert_eq!(outcomes(&events), vec![Outcome::Expired]);
        assert!(c.hal.is_muted(1), "a deliberate mute is never undone");
        assert!(c.hal.writes.is_empty());
        assert!(c.store.records.is_empty());
        assert!(c.next_due().is_none());
    }

    #[test]
    fn a_record_from_the_future_counts_as_expired() {
        let clock = Clock::new();
        let mut hal = FakeHal::speakers();
        hal.user_sets(1, true);
        let mut c = controller(hal);
        c.store.records = vec![record("speakers", clock.unix_ms + 60_000)];
        assert_eq!(
            outcomes(&c.recover_after_exit(clock.at(MS(0)))),
            vec![Outcome::Expired]
        );
        assert!(c.hal.writes.is_empty());
    }

    #[test]
    fn crash_recovery_respects_a_user_unmute_and_clears_the_record() {
        let clock = Clock::new();
        let mut c = controller(FakeHal::speakers());
        c.store.records = vec![record("speakers", clock.unix_ms)];
        assert_eq!(
            outcomes(&c.recover_after_exit(clock.at(MS(5000)))),
            vec![Outcome::UserChanged]
        );
        assert!(c.hal.writes.is_empty());
        assert!(c.store.records.is_empty());
    }

    #[test]
    fn an_unreadable_mute_keeps_the_record_and_retries_promptly() {
        let clock = Clock::new();
        let mut c = controller(FakeHal::speakers());
        start(&mut c, 1, &clock);
        // Undocking: the device is still listed but its mute cannot be read.
        c.hal.device(1).read_fails = true;
        assert_eq!(
            outcomes(&release(&mut c, 1, &clock, MS(2000))),
            vec![Outcome::Unreadable]
        );
        assert_eq!(c.store.records.len(), 1, "kept on disk");
        assert!(c.hal.is_muted(1));
        let due = c.next_due().expect("a retry is scheduled");
        assert_eq!(due, clock.at(MS(2000) + RETRY_DELAYS[0]).instant);

        // Still unreadable on the first retry; readable on the next.
        assert_eq!(
            outcomes(&c.poll(clock.at(MS(2500)))),
            vec![Outcome::Unreadable]
        );
        assert_eq!(c.store.records.len(), 1);
        c.hal.device(1).read_fails = false;
        let next = c.next_due().unwrap();
        assert_eq!(next, clock.at(MS(2500) + RETRY_DELAYS[1]).instant);
        assert_eq!(
            outcomes(&c.poll(clock.at(MS(3500)))),
            vec![Outcome::Restored]
        );
        assert!(!c.hal.is_muted(1));
        assert!(c.store.records.is_empty());
        assert!(c.next_due().is_none());
    }

    #[test]
    fn an_unreadable_mute_at_launch_keeps_the_record() {
        let clock = Clock::new();
        let mut hal = FakeHal::speakers();
        hal.user_sets(1, true);
        hal.device(1).read_fails = true;
        let mut c = controller(hal);
        c.store.records = vec![record("speakers", clock.unix_ms)];
        assert_eq!(
            outcomes(&c.recover_after_exit(clock.at(MS(1000)))),
            vec![Outcome::Unreadable]
        );
        assert_eq!(c.store.records.len(), 1);
        c.hal.device(1).read_fails = false;
        assert_eq!(
            outcomes(&c.poll(clock.at(MS(1500)))),
            vec![Outcome::Restored]
        );
        assert!(!c.hal.is_muted(1));
        assert!(c.store.records.is_empty());
    }

    #[test]
    fn a_failed_unmute_is_retried() {
        let clock = Clock::new();
        let mut c = controller(FakeHal::speakers());
        start(&mut c, 1, &clock);
        c.hal.fail_unmute = true;
        assert_eq!(
            outcomes(&release(&mut c, 1, &clock, MS(1000))),
            vec![Outcome::RestoreFailed]
        );
        c.hal.fail_unmute = false;
        assert_eq!(
            outcomes(&c.poll(clock.at(MS(1500)))),
            vec![Outcome::Restored]
        );
        assert!(!c.hal.is_muted(1));
    }

    #[test]
    fn a_replugged_device_is_restored_without_waiting_for_a_dictation() {
        let clock = Clock::new();
        let mut c = controller(FakeHal::speakers());
        start(&mut c, 1, &clock);
        c.hal.device(1).present = false;
        assert_eq!(
            outcomes(&release(&mut c, 1, &clock, MS(1000))),
            vec![Outcome::DeviceMissing]
        );
        // Quiet retries while it stays unplugged.
        assert!(c.poll(clock.at(MS(1500))).is_empty());
        assert!(c.poll(clock.at(MS(2500))).is_empty());
        c.hal.device(1).present = true;
        assert_eq!(
            outcomes(&c.poll(clock.at(MS(4500)))),
            vec![Outcome::Restored]
        );
        assert!(!c.hal.is_muted(1));
        assert!(c.next_due().is_none());
    }

    #[test]
    fn retries_stop_when_the_record_expires() {
        let clock = Clock::new();
        let mut c = controller(FakeHal::speakers());
        start(&mut c, 1, &clock);
        c.hal.device(1).present = false;
        release(&mut c, 1, &clock, MS(1000));
        let mut polls = 0;
        let mut last = Vec::new();
        while let Some(due) = c.next_due() {
            let offset = due.duration_since(clock.instant);
            last = c.poll(clock.at(offset));
            polls += 1;
            assert!(polls < 100, "retrying must end");
        }
        assert_eq!(outcomes(&last), vec![Outcome::Expired]);
        assert!(c.store.records.is_empty());
        // Replugged later: a deliberate mute after replug is left alone.
        c.hal.device(1).present = true;
        assert!(arm(&mut c, 2, false, &clock, RECORD_MAX_AGE * 2).is_empty());
        assert!(c.hal.is_muted(1));
    }

    #[test]
    fn a_missing_device_is_retried_at_the_next_start_too() {
        let clock = Clock::new();
        let mut c = controller(FakeHal::speakers());
        start(&mut c, 1, &clock);
        c.hal.device(1).present = false;
        release(&mut c, 1, &clock, MS(1000));
        c.hal.device(1).present = true;
        // The next dictation starts before the first retry is due.
        let events = arm(&mut c, 2, false, &clock, MS(1200));
        assert_eq!(outcomes(&events), vec![Outcome::Restored]);
        assert!(!c.hal.is_muted(1));
        assert!(c.store.records.is_empty());
    }

    #[test]
    fn a_failed_restore_after_a_long_dictation_still_gets_the_full_retry_window() {
        let clock = Clock::new();
        let mut c = controller(FakeHal::speakers());
        start(&mut c, 1, &clock);
        // Undocked just before stopping a dictation longer than the expiry.
        let stop = RECORD_MAX_AGE + Duration::from_secs(120);
        c.hal.device(1).present = false;
        assert_eq!(
            outcomes(&release(&mut c, 1, &clock, stop)),
            vec![Outcome::DeviceMissing]
        );
        // The expiry clock restarts when this process lets go, and that is
        // what a crash in the retry window would read back.
        assert_eq!(
            c.store.records,
            vec![record("speakers", clock.at(stop).unix_ms)]
        );
        c.hal.device(1).present = true;
        let due = c.next_due().expect("a retry is scheduled");
        assert_eq!(
            outcomes(&c.poll(Now {
                instant: due,
                unix_ms: clock.at(due.duration_since(clock.instant)).unix_ms,
            })),
            vec![Outcome::Restored]
        );
        assert!(!c.hal.is_muted(1));
        assert!(c.store.records.is_empty());
        assert!(c.next_due().is_none());
    }

    #[test]
    fn a_waiting_retry_never_unmutes_a_later_dictation_on_the_same_device() {
        let clock = Clock::new();
        let mut c = controller(FakeHal::speakers());
        start(&mut c, 1, &clock);
        c.hal.device(1).present = false;
        release(&mut c, 1, &clock, MS(1000));
        // Dictation 2 starts while the speakers are still away.
        arm(&mut c, 2, true, &clock, MS(1200));
        // They come back with their mute reset, and dictation 2 mutes them.
        c.hal.device(1).present = true;
        c.hal.user_sets(1, false);
        assert_eq!(
            outcomes(&cue(&mut c, 2, &clock, MS(1310))),
            vec![Outcome::Engaged]
        );
        // Every retry during dictation 2 leaves its mute alone.
        for offset in [MS(2000), MS(4000), MS(10_000), MS(40_000)] {
            assert!(c.poll(clock.at(offset)).is_empty());
            assert!(c.hal.is_muted(1), "unmuted mid-dictation at {offset:?}");
        }
        assert_eq!(
            outcomes(&release(&mut c, 2, &clock, MS(45_000))),
            vec![Outcome::Restored]
        );
        assert!(!c.hal.is_muted(1));
        // Dictation 2's restore replaced the older record for the speakers.
        assert!(c.store.records.is_empty());
        // The user mutes right after (to take a call): no later retry undoes it.
        c.hal.user_sets(1, true);
        let writes = c.hal.writes.len();
        let mut polls = 0;
        while let Some(due) = c.next_due() {
            c.poll(clock.at(due.duration_since(clock.instant)));
            polls += 1;
            assert!(polls < 100, "retrying must end");
        }
        for offset in [MS(46_000), MS(80_000), MS(120_000)] {
            c.poll(clock.at(offset));
        }
        assert!(c.hal.is_muted(1));
        assert_eq!(
            c.hal.writes.len(),
            writes,
            "no unmute after the user's mute"
        );
        assert!(c.store.records.is_empty());
    }

    #[test]
    fn stale_generations_cannot_release_or_engage_the_current_one() {
        let clock = Clock::new();
        let mut c = controller(FakeHal::speakers());
        arm(&mut c, 10, true, &clock, MS(0));
        assert!(cue(&mut c, 9, &clock, MS(100)).is_empty());
        assert!(!c.hal.is_muted(1));
        cue(&mut c, 10, &clock, MS(110));
        assert!(release(&mut c, 9, &clock, MS(500)).is_empty());
        assert!(c.hal.is_muted(1));
        assert_eq!(
            outcomes(&release(&mut c, 10, &clock, MS(900))),
            vec![Outcome::Restored]
        );
    }

    #[test]
    fn a_new_start_restores_a_generation_that_was_never_released() {
        let clock = Clock::new();
        let mut c = controller(FakeHal::speakers());
        start(&mut c, 1, &clock);
        let events = arm(&mut c, 2, true, &clock, MS(2000));
        assert_eq!(outcomes(&events), vec![Outcome::Restored]);
        assert!(!c.hal.is_muted(1));
        assert_eq!(
            outcomes(&cue(&mut c, 2, &clock, MS(2100))),
            vec![Outcome::Engaged]
        );
    }

    #[test]
    fn shutdown_restores_an_engaged_mute() {
        let clock = Clock::new();
        let mut c = controller(FakeHal::speakers());
        start(&mut c, 1, &clock);
        assert_eq!(
            outcomes(&c.shutdown(clock.at(MS(900)))),
            vec![Outcome::Restored]
        );
        assert!(!c.hal.is_muted(1));
        assert!(c.store.records.is_empty());
    }

    #[test]
    fn transitions_map_to_signals() {
        use DictationStatus::*;
        assert_eq!(signal_for(Idle, Starting), Some(Signal::Arm));
        assert_eq!(signal_for(Recovering, Starting), None);
        assert_eq!(signal_for(Starting, Recording), None);
        for to in [Recovering, Processing, Idle] {
            for from in [Starting, Recording, Recovering, Processing] {
                if from != to {
                    assert_eq!(
                        signal_for(from, to),
                        Some(Signal::Release),
                        "{from:?}->{to:?}"
                    );
                }
            }
        }
        // File transcription never arms; its release is a no-op.
        assert_eq!(signal_for(Idle, Processing), Some(Signal::Release));
        assert_eq!(signal_for(Recording, Recording), None);
    }

    #[test]
    fn route_classification() {
        let device = |transport, headphone_source, uid: &str| OutputDevice {
            id: 1,
            uid: uid.into(),
            transport,
            headphone_source,
        };
        assert_eq!(
            route(&device(Transport::BuiltIn, false, "BuiltInSpeakerDevice")),
            Route::Speakers
        );
        assert_eq!(
            route(&device(Transport::BuiltIn, true, "x")),
            Route::Headphones
        );
        assert_eq!(
            route(&device(Transport::BuiltIn, false, BUILT_IN_HEADPHONE_UID)),
            Route::Headphones
        );
        // The UID alone is not trusted on a non-built-in device.
        assert_eq!(
            route(&device(Transport::Usb, false, BUILT_IN_HEADPHONE_UID)),
            Route::Speakers
        );
        assert_eq!(
            route(&device(Transport::Bluetooth, false, "x")),
            Route::Bluetooth
        );
        assert_eq!(Transport::from_code(None), Transport::Unknown);
        assert_eq!(
            Transport::from_code(Some(u32::from_be_bytes(*b"zzzz"))),
            Transport::Other
        );
    }

    #[test]
    fn file_store_round_trips_a_record() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = FileStore::new(Some(dir.path().to_path_buf()));
        assert!(store.load().is_empty());
        let saved = QuietRecord {
            device_uid: "BuiltInSpeakerDevice".into(),
            elements: vec![0],
            muted_at_unix_ms: 1_800_000_000_000,
        };
        assert!(store.save(std::slice::from_ref(&saved)));
        let path = dir.path().join(RECORD_FILE_NAME);
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["schema_version"], 2);
        let keys: Vec<_> = json["entries"][0]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(keys, vec!["device_uid", "elements", "muted_at_unix_ms"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert_eq!(store.load(), vec![saved]);
        assert!(store.save(&[]));
        assert!(!path.exists());
        assert!(store.save(&[]), "clearing a missing file succeeds");
    }

    #[test]
    fn file_store_rejects_old_bad_or_oversized_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(RECORD_FILE_NAME);
        let mut store = FileStore::new(Some(dir.path().to_path_buf()));
        // v1 records carry no mute time; they are never acted on.
        std::fs::write(
            &path,
            "{\"schema_version\":1,\"entries\":[{\"device_uid\":\"a\",\"elements\":[0]}]}",
        )
        .unwrap();
        assert!(store.load().is_empty());
        std::fs::write(&path, "not json").unwrap();
        assert!(store.load().is_empty());
        std::fs::write(
            &path,
            "{\"schema_version\":2,\"entries\":[{\"device_uid\":\"\",\"elements\":[0],\"muted_at_unix_ms\":1}]}",
        )
        .unwrap();
        assert!(store.load().is_empty());
        std::fs::write(&path, " ".repeat(RECORD_MAX_BYTES as usize + 1)).unwrap();
        assert!(store.load().is_empty());
        assert!(!FileStore::new(None).save(&[record("a", 1)]));
    }

    #[test]
    fn a_stale_unreadable_record_file_is_cleared_at_launch() {
        let clock = Clock::new();
        let mut c = controller(FakeHal::speakers());
        assert!(c.recover_after_exit(clock.at(MS(0))).is_empty());
        assert_eq!(c.store.saves, 1);
    }
}
