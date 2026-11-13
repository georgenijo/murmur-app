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
//! - Engage is skipped when the output is headphones (built-in jack data
//!   source) or Bluetooth (AirPods, headsets), when the device has no settable
//!   mute (many HDMI/DisplayPort outputs), or when it is already muted.
//! - Restore touches only the device Murmur muted, found again by its UID. If
//!   the user unmuted it during the dictation (volume keys unmute), the restore
//!   leaves it alone. A new default output is never touched.
//! - Before muting, a content-free record (device UID and mute elements) is
//!   written to `output-quieting.json`. The next launch restores from it if the
//!   app died mid-dictation. If the record cannot be written, nothing is muted.
//! - All work runs on one background thread. The dictation status seam only
//!   sends a message, so start and stop never wait on Core Audio.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::state::DictationStatus;

const RECORD_FILE_NAME: &str = "output-quieting.json";
const RECORD_SCHEMA_VERSION: u32 = 1;
const RECORD_MAX_BYTES: u64 = 8 * 1024;
const RECORD_MAX_ENTRIES: usize = 4;
const RECORD_MAX_ELEMENTS: usize = 8;

/// Murmur's start cue is a 70 ms tone scheduled when the dictation is
/// accepted. Muting waits this long after the start so the cue stays audible.
pub(crate) const START_CUE_GUARD: Duration = Duration::from_millis(150);

/// Bound on how long app exit waits for the worker to unmute.
const SHUTDOWN_WAIT: Duration = Duration::from_millis(750);

/// `'hdpn'`: the built-in output's headphone-jack data source.
const DATA_SOURCE_HEADPHONES: u32 = u32::from_be_bytes(*b"hdpn");

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
    /// Stable Core Audio UID. Persisted locally only; never logged.
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
/// headset; Bluetooth speakers are a documented limit.
fn route(device: &OutputDevice) -> Route {
    if device.transport == Transport::Bluetooth {
        Route::Bluetooth
    } else if device.headphone_source {
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

/// "Murmur muted these elements on this device." No audio, transcript,
/// clipboard, or app identity: only a device UID and element numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct QuietRecord {
    pub device_uid: String,
    pub elements: Vec<u32>,
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

fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_file_name(format!(".{RECORD_FILE_NAME}.murmur-tmp"));
    let result = (|| {
        let mut file = std::fs::File::create(&temp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(contents.as_bytes())?;
        // The record must survive a crash that follows the mute.
        file.sync_all()?;
        std::fs::rename(&temp, path)
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
}

/// Content-free evidence for one decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Event {
    pub recording_id: u64,
    pub outcome: Outcome,
    pub transport: Option<Transport>,
    /// Time spent in Core Audio for this decision.
    pub hal_us: u64,
}

#[derive(Debug)]
pub(crate) enum Command {
    /// Idle -> Starting: freeze the setting for this generation.
    Arm {
        recording_id: u64,
        enabled: bool,
        at: Instant,
    },
    /// Entered Recording (including after a recovery).
    Engage {
        recording_id: u64,
        at: Instant,
    },
    /// Left live capture. `terminal` ends the generation.
    Release {
        recording_id: u64,
        terminal: bool,
    },
    Shutdown(mpsc::Sender<()>),
}

#[derive(Debug, Clone, Copy)]
struct Armed {
    recording_id: u64,
    enabled: bool,
    at: Instant,
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    recording_id: u64,
    due: Instant,
}

#[derive(Debug, Clone)]
struct Engaged {
    recording_id: u64,
    record: QuietRecord,
}

pub(crate) struct Controller<H, S> {
    hal: H,
    store: S,
    armed: Option<Armed>,
    pending: Option<Pending>,
    engaged: Option<Engaged>,
    /// Records whose restore could not finish yet (device unplugged, write
    /// failed). Retried at the next dictation start and the next launch.
    outstanding: Vec<QuietRecord>,
}

impl<H: OutputHal, S: RecordStore> Controller<H, S> {
    pub(crate) fn new(hal: H, store: S) -> Self {
        Self {
            hal,
            store,
            armed: None,
            pending: None,
            engaged: None,
            outstanding: Vec::new(),
        }
    }

    /// Launch recovery: restore anything a previous process left muted.
    pub(crate) fn recover_after_exit(&mut self) -> Vec<Event> {
        self.outstanding = self.store.load();
        if self.outstanding.is_empty() {
            // Clear a stale or unreadable file so it is not reread forever.
            let _ = self.persist();
            return Vec::new();
        }
        self.retry_outstanding(0, Outcome::RestoredAfterExit)
    }

    pub(crate) fn next_due(&self) -> Option<Instant> {
        self.pending.map(|pending| pending.due)
    }

    pub(crate) fn handle(&mut self, command: Command, now: Instant) -> Vec<Event> {
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
                    events.push(self.restore(
                        engaged.recording_id,
                        engaged.record,
                        Outcome::Restored,
                    ));
                }
                self.pending = None;
                events.extend(self.retry_outstanding(recording_id, Outcome::Restored));
                self.armed = Some(Armed {
                    recording_id,
                    enabled,
                    at,
                });
                if enabled {
                    self.pending = Some(Pending {
                        recording_id,
                        due: at + START_CUE_GUARD,
                    });
                }
            }
            Command::Engage { recording_id, at } => {
                if let Some(armed) = self.armed {
                    if armed.recording_id == recording_id
                        && armed.enabled
                        && self.engaged.is_none()
                        && self.pending.is_none()
                    {
                        self.pending = Some(Pending {
                            recording_id,
                            due: at.max(armed.at + START_CUE_GUARD),
                        });
                    }
                }
            }
            Command::Release {
                recording_id,
                terminal,
            } => {
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
                    events.push(self.restore(recording_id, engaged.record, Outcome::Restored));
                }
                if terminal
                    && self
                        .armed
                        .is_some_and(|armed| armed.recording_id == recording_id)
                {
                    self.armed = None;
                }
            }
            Command::Shutdown(_) => {
                events.extend(self.shutdown());
            }
        }
        events.extend(self.poll(now));
        events
    }

    /// Run a due engage.
    pub(crate) fn poll(&mut self, now: Instant) -> Vec<Event> {
        match self.pending {
            Some(pending) if pending.due <= now => {
                self.pending = None;
                vec![self.engage(pending.recording_id)]
            }
            _ => Vec::new(),
        }
    }

    /// App exit: never leave the speakers muted behind a clean quit.
    pub(crate) fn shutdown(&mut self) -> Vec<Event> {
        self.pending = None;
        self.armed = None;
        match self.engaged.take() {
            Some(engaged) => {
                vec![self.restore(engaged.recording_id, engaged.record, Outcome::Restored)]
            }
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

    fn engage(&mut self, recording_id: u64) -> Event {
        let started = Instant::now();
        let event = |outcome, transport| Event {
            recording_id,
            outcome,
            transport,
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
        let record = QuietRecord {
            device_uid: device.uid.clone(),
            elements: to_mute,
        };
        // Write the record first: a crash after the mute must still find it.
        self.engaged = Some(Engaged {
            recording_id,
            record: record.clone(),
        });
        if !self.persist() {
            self.engaged = None;
            let _ = self.persist();
            return event(Outcome::SkippedRecordUnavailable, transport);
        }
        if !self.hal.set_muted(device.id, &record.elements, true) {
            // A partial write leaves some elements muted; they were audible a
            // moment ago, so unmuting them returns the device to its state.
            let _ = self.hal.set_muted(device.id, &record.elements, false);
            self.engaged = None;
            let _ = self.persist();
            return event(Outcome::MuteFailed, transport);
        }
        event(Outcome::Engaged, transport)
    }

    /// Restore one record. Leaves the device alone if the user changed it.
    fn restore(&mut self, recording_id: u64, record: QuietRecord, success: Outcome) -> Event {
        let started = Instant::now();
        let outcome = match self.hal.device_for_uid(&record.device_uid) {
            None => {
                self.keep_outstanding(record);
                Outcome::DeviceMissing
            }
            Some(device) => match self.hal.muted(device, &record.elements) {
                Some(states) if states.len() == record.elements.len() => {
                    if states.iter().all(|muted| *muted) {
                        if self.hal.set_muted(device, &record.elements, false) {
                            success
                        } else {
                            self.keep_outstanding(record);
                            Outcome::RestoreFailed
                        }
                    } else {
                        Outcome::UserChanged
                    }
                }
                _ => Outcome::Unreadable,
            },
        };
        let _ = self.persist();
        Event {
            recording_id,
            outcome,
            transport: None,
            hal_us: started.elapsed().as_micros() as u64,
        }
    }

    fn keep_outstanding(&mut self, record: QuietRecord) {
        self.outstanding.retain(|existing| existing != &record);
        self.outstanding.push(record);
        let excess = self.outstanding.len().saturating_sub(RECORD_MAX_ENTRIES);
        self.outstanding.drain(..excess);
    }

    fn retry_outstanding(&mut self, recording_id: u64, success: Outcome) -> Vec<Event> {
        let records = std::mem::take(&mut self.outstanding);
        let mut events: Vec<Event> = records
            .into_iter()
            .map(|record| self.restore(recording_id, record, success))
            .collect();
        // A device that stays unplugged is retried silently at each start;
        // launch recovery (recording 0) still reports it once.
        events.retain(|event| event.outcome != Outcome::DeviceMissing || recording_id == 0);
        events
    }
}

// ---------------------------------------------------------------------------
// Status seam and worker
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Signal {
    Arm,
    Engage,
    Release { terminal: bool },
}

/// Map a dictation status transition to a controller signal. Fn key-down
/// capture that is discarded never leaves Idle, file transcription goes
/// Idle -> Processing without an Arm, and meetings and the microphone test do
/// not use dictation status, so none of them can engage.
pub(crate) fn signal_for(from: DictationStatus, to: DictationStatus) -> Option<Signal> {
    if from == to {
        return None;
    }
    match to {
        DictationStatus::Starting if from == DictationStatus::Idle => Some(Signal::Arm),
        DictationStatus::Starting => None,
        DictationStatus::Recording => Some(Signal::Engage),
        // Capture is interrupted: there is nothing to protect until it resumes.
        DictationStatus::Recovering => Some(Signal::Release { terminal: false }),
        DictationStatus::Processing | DictationStatus::Idle => {
            Some(Signal::Release { terminal: true })
        }
    }
}

static WORKER: OnceLock<mpsc::Sender<Command>> = OnceLock::new();

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
    let Some(signal) = signal_for(from, to) else {
        return;
    };
    let Some(sender) = WORKER.get() else {
        return;
    };
    let at = Instant::now();
    let command = match signal {
        Signal::Arm => Command::Arm {
            recording_id,
            enabled,
            at,
        },
        Signal::Engage => Command::Engage { recording_id, at },
        Signal::Release { terminal } => Command::Release {
            recording_id,
            terminal,
        },
    };
    let _ = sender.send(command);
}

fn log(events: Vec<Event>) {
    for event in events {
        tracing::info!(
            target: "audio",
            event_code = "audio.output_quieting",
            recording_id = event.recording_id,
            outcome = event.outcome.code(),
            transport = event.transport.map(Transport::label).unwrap_or("none"),
            hal_us = event.hal_us,
            "output quieting decision"
        );
    }
}

fn run<H: OutputHal, S: RecordStore>(
    mut controller: Controller<H, S>,
    rx: mpsc::Receiver<Command>,
) {
    log(controller.recover_after_exit());
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
                log(controller.shutdown());
                let _ = ack.send(());
                break;
            }
            Some(command) => log(controller.handle(command, Instant::now())),
            None => log(controller.poll(Instant::now())),
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
    }

    #[derive(Default)]
    struct FakeHal {
        devices: HashMap<u32, FakeDevice>,
        default: Option<u32>,
        fail_set: bool,
        writes: Vec<(u32, Vec<u32>, bool)>,
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
                },
            );
        }

        fn is_muted(&self, id: u32) -> bool {
            self.devices[&id].muted.values().all(|muted| *muted)
        }

        fn user_sets(&mut self, id: u32, muted: bool) {
            for value in self.devices.get_mut(&id).unwrap().muted.values_mut() {
                *value = muted;
            }
        }
    }

    impl OutputHal for FakeHal {
        fn default_output(&mut self) -> Option<OutputDevice> {
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
            elements
                .iter()
                .map(|element| device.muted.get(element).copied())
                .collect()
        }

        fn set_muted(&mut self, device: u32, elements: &[u32], muted: bool) -> bool {
            self.writes.push((device, elements.to_vec(), muted));
            if self.fail_set && muted {
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

    fn controller(hal: FakeHal) -> TestController {
        Controller::new(hal, MemoryStore::default())
    }

    fn outcomes(events: &[Event]) -> Vec<Outcome> {
        events.iter().map(|event| event.outcome).collect()
    }

    /// Arm at `t0`, enter Recording, then run the due engage.
    fn start(controller: &mut TestController, rid: u64, enabled: bool, t0: Instant) -> Vec<Event> {
        let mut events = controller.handle(
            Command::Arm {
                recording_id: rid,
                enabled,
                at: t0,
            },
            t0,
        );
        events.extend(controller.handle(
            Command::Engage {
                recording_id: rid,
                at: t0 + Duration::from_millis(20),
            },
            t0 + Duration::from_millis(20),
        ));
        events.extend(controller.poll(t0 + START_CUE_GUARD));
        events
    }

    fn release(controller: &mut TestController, rid: u64) -> Vec<Event> {
        controller.handle(
            Command::Release {
                recording_id: rid,
                terminal: true,
            },
            Instant::now(),
        )
    }

    #[test]
    fn engages_after_the_start_cue_and_restores_at_stop() {
        let mut controller = controller(FakeHal::speakers());
        let t0 = Instant::now();
        controller.handle(
            Command::Arm {
                recording_id: 7,
                enabled: true,
                at: t0,
            },
            t0,
        );
        // Before the cue guard elapses nothing is muted.
        assert!(controller.poll(t0 + Duration::from_millis(100)).is_empty());
        assert!(!controller.hal.is_muted(1));
        assert_eq!(controller.next_due(), Some(t0 + START_CUE_GUARD));

        let events = controller.poll(t0 + START_CUE_GUARD);
        assert_eq!(outcomes(&events), vec![Outcome::Engaged]);
        assert_eq!(events[0].transport, Some(Transport::BuiltIn));
        assert!(controller.hal.is_muted(1));
        assert_eq!(
            controller.store.records,
            vec![QuietRecord {
                device_uid: "speakers".into(),
                elements: vec![0]
            }]
        );

        assert_eq!(
            outcomes(&release(&mut controller, 7)),
            vec![Outcome::Restored]
        );
        assert!(!controller.hal.is_muted(1));
        assert!(controller.store.records.is_empty());
    }

    #[test]
    fn the_record_is_written_before_the_mute() {
        let mut controller = controller(FakeHal::speakers());
        controller.store.fail_save = true;
        let events = start(&mut controller, 1, true, Instant::now());
        assert_eq!(outcomes(&events), vec![Outcome::SkippedRecordUnavailable]);
        assert!(
            controller.hal.writes.is_empty(),
            "no mute without a durable record"
        );
        assert!(!controller.hal.is_muted(1));
    }

    #[test]
    fn disabled_setting_never_touches_the_device() {
        let mut controller = controller(FakeHal::speakers());
        let events = start(&mut controller, 1, false, Instant::now());
        assert!(events.is_empty());
        assert!(release(&mut controller, 1).is_empty());
        assert!(controller.hal.writes.is_empty());
        assert_eq!(
            controller.store.saves, 0,
            "a disabled dictation does no disk work"
        );
    }

    #[test]
    fn a_stop_before_the_cue_guard_cancels_the_engage() {
        let mut controller = controller(FakeHal::speakers());
        let t0 = Instant::now();
        controller.handle(
            Command::Arm {
                recording_id: 3,
                enabled: true,
                at: t0,
            },
            t0,
        );
        // Quick cancel (or a start that failed) while still Starting.
        assert!(release(&mut controller, 3).is_empty());
        assert!(controller.next_due().is_none());
        assert!(controller.poll(t0 + Duration::from_secs(1)).is_empty());
        // A late Recording for the finished generation cannot re-arm it.
        controller.handle(
            Command::Engage {
                recording_id: 3,
                at: t0,
            },
            t0,
        );
        assert!(controller.poll(t0 + Duration::from_secs(1)).is_empty());
        assert!(controller.hal.writes.is_empty());
    }

    #[test]
    fn user_unmuting_during_the_dictation_is_left_alone() {
        let mut controller = controller(FakeHal::speakers());
        start(&mut controller, 2, true, Instant::now());
        assert!(controller.hal.is_muted(1));
        // Volume keys unmute and change the level.
        controller.hal.user_sets(1, false);
        let writes_before = controller.hal.writes.len();
        assert_eq!(
            outcomes(&release(&mut controller, 2)),
            vec![Outcome::UserChanged]
        );
        assert_eq!(
            controller.hal.writes.len(),
            writes_before,
            "no write after a user change"
        );
        assert!(controller.store.records.is_empty());
    }

    #[test]
    fn a_device_change_restores_the_original_and_leaves_the_new_one() {
        let mut hal = FakeHal::speakers();
        hal.add(2, "display", Transport::Hdmi, false, Some(vec![0]));
        let mut controller = controller(hal);
        start(&mut controller, 4, true, Instant::now());
        assert!(controller.hal.is_muted(1));
        // The user switches output mid-dictation.
        controller.hal.default = Some(2);
        assert_eq!(
            outcomes(&release(&mut controller, 4)),
            vec![Outcome::Restored]
        );
        assert!(!controller.hal.is_muted(1));
        assert!(!controller.hal.is_muted(2));
        assert!(controller
            .hal
            .writes
            .iter()
            .all(|(device, _, _)| *device == 1));
    }

    #[test]
    fn no_software_mute_means_no_change() {
        let mut hal = FakeHal::default();
        hal.add(5, "hdmi", Transport::Hdmi, false, None);
        hal.default = Some(5);
        let mut controller = controller(hal);
        let events = start(&mut controller, 1, true, Instant::now());
        assert_eq!(outcomes(&events), vec![Outcome::SkippedNoMuteControl]);
        assert!(controller.hal.writes.is_empty());
        assert!(controller.store.records.is_empty());
        assert!(release(&mut controller, 1).is_empty());
    }

    #[test]
    fn per_channel_mute_is_used_when_there_is_no_main_element() {
        let mut hal = FakeHal::default();
        hal.add(6, "usb", Transport::Usb, false, Some(vec![1, 2]));
        hal.default = Some(6);
        let mut controller = controller(hal);
        start(&mut controller, 1, true, Instant::now());
        assert!(controller.hal.is_muted(6));
        assert_eq!(controller.store.records[0].elements, vec![1, 2]);
        release(&mut controller, 1);
        assert!(!controller.hal.is_muted(6));
    }

    #[test]
    fn headphones_and_bluetooth_are_skipped() {
        for (transport, headphone_source, expected) in [
            (Transport::BuiltIn, true, Outcome::SkippedHeadphones),
            (Transport::Bluetooth, false, Outcome::SkippedBluetooth),
        ] {
            let mut hal = FakeHal::default();
            hal.add(9, "out", transport, headphone_source, Some(vec![0]));
            hal.default = Some(9);
            let mut controller = controller(hal);
            let events = start(&mut controller, 1, true, Instant::now());
            assert_eq!(outcomes(&events), vec![expected]);
            assert!(controller.hal.writes.is_empty());
        }
    }

    #[test]
    fn unclear_routes_still_mute() {
        for transport in [
            Transport::Unknown,
            Transport::Aggregate,
            Transport::Virtual,
            Transport::AirPlay,
            Transport::DisplayPort,
        ] {
            let mut hal = FakeHal::default();
            hal.add(9, "out", transport, false, Some(vec![0]));
            hal.default = Some(9);
            let mut controller = controller(hal);
            let events = start(&mut controller, 1, true, Instant::now());
            assert_eq!(outcomes(&events), vec![Outcome::Engaged], "{transport:?}");
        }
    }

    #[test]
    fn an_already_muted_output_is_not_recorded_or_unmuted() {
        let mut hal = FakeHal::speakers();
        hal.user_sets(1, true);
        let mut controller = controller(hal);
        let events = start(&mut controller, 1, true, Instant::now());
        assert_eq!(outcomes(&events), vec![Outcome::SkippedAlreadyMuted]);
        assert!(release(&mut controller, 1).is_empty());
        assert!(controller.hal.is_muted(1), "the user's mute stays");
        assert!(controller.hal.writes.is_empty());
    }

    #[test]
    fn only_audible_channels_are_muted_and_restored() {
        let mut hal = FakeHal::default();
        hal.add(6, "usb", Transport::Usb, false, Some(vec![1, 2]));
        hal.default = Some(6);
        hal.devices.get_mut(&6).unwrap().muted.insert(2, true);
        let mut controller = controller(hal);
        start(&mut controller, 1, true, Instant::now());
        assert_eq!(controller.store.records[0].elements, vec![1]);
        release(&mut controller, 1);
        assert!(!controller.hal.devices[&6].muted[&1]);
        assert!(controller.hal.devices[&6].muted[&2]);
    }

    #[test]
    fn a_failed_mute_clears_the_record() {
        let mut hal = FakeHal::speakers();
        hal.fail_set = true;
        let mut controller = controller(hal);
        let events = start(&mut controller, 1, true, Instant::now());
        assert_eq!(outcomes(&events), vec![Outcome::MuteFailed]);
        assert!(controller.store.records.is_empty());
        assert!(release(&mut controller, 1).is_empty());
    }

    #[test]
    fn crash_recovery_restores_from_the_record() {
        let mut hal = FakeHal::speakers();
        hal.user_sets(1, true);
        let mut controller = controller(hal);
        controller.store.records = vec![QuietRecord {
            device_uid: "speakers".into(),
            elements: vec![0],
        }];
        let events = controller.recover_after_exit();
        assert_eq!(outcomes(&events), vec![Outcome::RestoredAfterExit]);
        assert_eq!(events[0].recording_id, 0);
        assert!(!controller.hal.is_muted(1));
        assert!(controller.store.records.is_empty());
    }

    #[test]
    fn crash_recovery_respects_a_user_unmute_and_clears_the_record() {
        let mut controller = controller(FakeHal::speakers());
        controller.store.records = vec![QuietRecord {
            device_uid: "speakers".into(),
            elements: vec![0],
        }];
        assert_eq!(
            outcomes(&controller.recover_after_exit()),
            vec![Outcome::UserChanged]
        );
        assert!(controller.hal.writes.is_empty());
        assert!(controller.store.records.is_empty());
    }

    #[test]
    fn a_missing_device_keeps_the_record_and_retries_at_the_next_start() {
        let mut controller = controller(FakeHal::speakers());
        start(&mut controller, 1, true, Instant::now());
        controller.hal.devices.get_mut(&1).unwrap().present = false;
        assert_eq!(
            outcomes(&release(&mut controller, 1)),
            vec![Outcome::DeviceMissing]
        );
        assert_eq!(
            controller.store.records.len(),
            1,
            "kept for the next launch"
        );

        // Plugged back in; the next dictation start restores it first.
        controller.hal.devices.get_mut(&1).unwrap().present = true;
        let t1 = Instant::now();
        let events = controller.handle(
            Command::Arm {
                recording_id: 2,
                enabled: false,
                at: t1,
            },
            t1,
        );
        assert_eq!(outcomes(&events), vec![Outcome::Restored]);
        assert!(!controller.hal.is_muted(1));
        assert!(controller.store.records.is_empty());
    }

    #[test]
    fn stale_generations_cannot_release_or_engage_the_current_one() {
        let mut controller = controller(FakeHal::speakers());
        start(&mut controller, 10, true, Instant::now());
        assert!(release(&mut controller, 9).is_empty());
        assert!(controller.hal.is_muted(1));
        assert_eq!(
            outcomes(&release(&mut controller, 10)),
            vec![Outcome::Restored]
        );
    }

    #[test]
    fn a_new_start_restores_a_generation_that_was_never_released() {
        let mut controller = controller(FakeHal::speakers());
        start(&mut controller, 1, true, Instant::now());
        let t1 = Instant::now();
        let events = controller.handle(
            Command::Arm {
                recording_id: 2,
                enabled: true,
                at: t1,
            },
            t1,
        );
        assert_eq!(outcomes(&events), vec![Outcome::Restored]);
        assert!(!controller.hal.is_muted(1));
        assert_eq!(
            outcomes(&controller.poll(t1 + START_CUE_GUARD)),
            vec![Outcome::Engaged]
        );
    }

    #[test]
    fn recovering_releases_and_a_resumed_recording_engages_again() {
        let mut controller = controller(FakeHal::speakers());
        let t0 = Instant::now();
        start(&mut controller, 5, true, t0);
        let events = controller.handle(
            Command::Release {
                recording_id: 5,
                terminal: false,
            },
            t0 + Duration::from_secs(1),
        );
        assert_eq!(outcomes(&events), vec![Outcome::Restored]);
        assert!(!controller.hal.is_muted(1));
        let resumed = t0 + Duration::from_secs(2);
        let events = controller.handle(
            Command::Engage {
                recording_id: 5,
                at: resumed,
            },
            resumed,
        );
        assert_eq!(
            outcomes(&events),
            vec![Outcome::Engaged],
            "guard already elapsed"
        );
        assert!(controller.hal.is_muted(1));
        release(&mut controller, 5);
        assert!(!controller.hal.is_muted(1));
    }

    #[test]
    fn shutdown_restores_an_engaged_mute() {
        let mut controller = controller(FakeHal::speakers());
        start(&mut controller, 1, true, Instant::now());
        assert_eq!(outcomes(&controller.shutdown()), vec![Outcome::Restored]);
        assert!(!controller.hal.is_muted(1));
        assert!(controller.store.records.is_empty());
    }

    #[test]
    fn transitions_map_to_signals() {
        use DictationStatus::*;
        assert_eq!(signal_for(Idle, Starting), Some(Signal::Arm));
        assert_eq!(signal_for(Recovering, Starting), None);
        assert_eq!(signal_for(Starting, Recording), Some(Signal::Engage));
        assert_eq!(signal_for(Recovering, Recording), Some(Signal::Engage));
        assert_eq!(
            signal_for(Recording, Recovering),
            Some(Signal::Release { terminal: false })
        );
        for to in [Processing, Idle] {
            for from in [Starting, Recording, Recovering] {
                assert_eq!(
                    signal_for(from, to),
                    Some(Signal::Release { terminal: true })
                );
            }
        }
        // File transcription never arms.
        assert_eq!(
            signal_for(Idle, Processing),
            Some(Signal::Release { terminal: true })
        );
        assert_eq!(signal_for(Recording, Recording), None);
    }

    #[test]
    fn route_classification() {
        let device = |transport, headphone_source| OutputDevice {
            id: 1,
            uid: "x".into(),
            transport,
            headphone_source,
        };
        assert_eq!(route(&device(Transport::BuiltIn, false)), Route::Speakers);
        assert_eq!(route(&device(Transport::BuiltIn, true)), Route::Headphones);
        assert_eq!(
            route(&device(Transport::Bluetooth, false)),
            Route::Bluetooth
        );
        assert_eq!(route(&device(Transport::Usb, false)), Route::Speakers);
        assert_eq!(Transport::from_code(None), Transport::Unknown);
        assert_eq!(
            Transport::from_code(Some(u32::from_be_bytes(*b"zzzz"))),
            Transport::Other
        );
    }

    #[test]
    fn file_store_round_trips_a_content_free_record() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = FileStore::new(Some(dir.path().to_path_buf()));
        assert!(store.load().is_empty());
        let record = QuietRecord {
            device_uid: "BuiltInSpeakerDevice".into(),
            elements: vec![0],
        };
        assert!(store.save(std::slice::from_ref(&record)));
        let path = dir.path().join(RECORD_FILE_NAME);
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["schema_version"], 1);
        let keys: Vec<_> = json["entries"][0]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(keys, vec!["device_uid".to_string(), "elements".to_string()]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert_eq!(store.load(), vec![record]);
        assert!(store.save(&[]));
        assert!(!path.exists());
        assert!(store.save(&[]), "clearing a missing file succeeds");
    }

    #[test]
    fn file_store_rejects_bad_or_oversized_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(RECORD_FILE_NAME);
        let mut store = FileStore::new(Some(dir.path().to_path_buf()));
        std::fs::write(
            &path,
            "{\"schema_version\":2,\"entries\":[{\"device_uid\":\"a\",\"elements\":[0]}]}",
        )
        .unwrap();
        assert!(store.load().is_empty());
        std::fs::write(&path, "not json").unwrap();
        assert!(store.load().is_empty());
        std::fs::write(
            &path,
            "{\"schema_version\":1,\"entries\":[{\"device_uid\":\"\",\"elements\":[0]}]}",
        )
        .unwrap();
        assert!(store.load().is_empty());
        std::fs::write(&path, " ".repeat(RECORD_MAX_BYTES as usize + 1)).unwrap();
        assert!(store.load().is_empty());
        // No data directory: nothing can be recorded, so nothing is muted.
        assert!(!FileStore::new(None).save(&[QuietRecord {
            device_uid: "a".into(),
            elements: vec![0]
        }]));
    }

    #[test]
    fn a_stale_unreadable_record_file_is_cleared_at_launch() {
        let mut controller = controller(FakeHal::speakers());
        controller.store.records = Vec::new();
        assert!(controller.recover_after_exit().is_empty());
        assert_eq!(controller.store.saves, 1);
    }
}
