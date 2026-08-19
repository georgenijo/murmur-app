//! System audio-graph introspection for capture-hang diagnostics.
//!
//! A capture hang where the worker's `AudioOutputUnitStart` sits queued inside
//! coreaudiod means something else is holding that server's engine queue. The
//! existing hang bundle can describe the machine but not name the holder, so
//! this module adds the two missing views:
//!
//! * [`snapshot`] asks the public Core Audio HAL which process objects, taps,
//!   and devices exist and which of them have live IO right now.
//! * [`internal_owners_report`] reports which Murmur subsystems hold an audio
//!   context, from cheap non-blocking state reads only (no HAL calls).
//!
//! Hang safety: when coreaudiod is wedged the HAL property calls block too, so
//! every HAL query runs on a dedicated throwaway thread behind a hard deadline.
//! On timeout the thread is abandoned and the section body records the timeout,
//! which is itself diagnostic evidence. Nothing here may run on the capture
//! supervisor thread or gate the fallback/kill sequence.
//!
//! Privacy: the rendered report names devices, taps, and processes, so it is
//! only ever placed in the server-armed hang bundle (same consent boundary as
//! the existing `system_profiler` sections). The structured event emitted from
//! every install carries [`AudioGraphCounts`] only — integers and one boolean.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::hang_diagnostics::{run_capped, truncate_at_boundary};

/// Hard deadline for the whole HAL query. Two seconds is far longer than a
/// healthy HAL round trip and far shorter than a capture attempt budget.
const QUERY_DEADLINE: Duration = Duration::from_secs(2);
const QUERY_TIMED_OUT_BODY: &str = "<audio graph query timed out after 2s>";
const PROCESS_NAME_DEADLINE: Duration = Duration::from_secs(5);

/// Object-list bounds. A healthy machine reports a handful of each; these caps
/// only stop a pathological HAL from producing an unbounded report.
const MAX_PROCESS_OBJECTS: usize = 256;
const MAX_TAP_OBJECTS: usize = 64;
const MAX_DEVICE_OBJECTS: usize = 64;
const MAX_SUB_DEVICES: usize = 64;
const MAX_TEXT_CHARS: usize = 128;
const REPORT_CAP_BYTES: usize = 64_000;

/// Highest capture ID already observed this process lifetime. A capture attempt
/// contributes at most one `audio.system_audio_graph_observed` event.
static LAST_OBSERVED_CAPTURE_ID: AtomicU64 = AtomicU64::new(0);
static APP_HANDLE: OnceLock<Mutex<Option<tauri::AppHandle>>> = OnceLock::new();

// ---------------------------------------------------------------------------
// Observation model (platform independent, so rendering and counting are
// testable on any OS with injected data)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProcessObservation {
    pub(crate) object_id: u32,
    pub(crate) pid: Option<i32>,
    pub(crate) running: Option<bool>,
    pub(crate) running_input: Option<bool>,
    pub(crate) running_output: Option<bool>,
}

impl ProcessObservation {
    /// True when this process object claims any live audio IO. Missing
    /// properties never invent activity.
    fn has_live_io(&self) -> bool {
        self.running == Some(true)
            || self.running_input == Some(true)
            || self.running_output == Some(true)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TapObservation {
    pub(crate) object_id: u32,
    pub(crate) uid: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DeviceObservation {
    pub(crate) object_id: u32,
    pub(crate) name: Option<String>,
    /// Raw `kAudioDevicePropertyTransportType` FourCC, rendered as its ASCII
    /// tag when printable.
    pub(crate) transport_type: Option<u32>,
    pub(crate) running_somewhere: Option<bool>,
    /// Present only for aggregate devices.
    pub(crate) sub_device_count: Option<usize>,
}

/// Everything one HAL pass learned, plus the notes describing what it could
/// not learn. Never contains partial guesses: a property that failed is `None`
/// and a failed list is a note.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AudioGraphObservation {
    pub(crate) processes: Vec<ProcessObservation>,
    pub(crate) taps: Vec<TapObservation>,
    pub(crate) devices: Vec<DeviceObservation>,
    pub(crate) processes_truncated: bool,
    pub(crate) taps_truncated: bool,
    pub(crate) devices_truncated: bool,
    pub(crate) notes: Vec<String>,
}

/// The content-free aggregate. This is the only shape allowed to leave an
/// unarmed install: integers and one boolean, no PIDs, names, or UIDs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AudioGraphCounts {
    pub(crate) running_audio_process_count: u64,
    pub(crate) tap_count: u64,
    pub(crate) device_count: u64,
    pub(crate) devices_running_count: u64,
    pub(crate) query_timed_out: bool,
    pub(crate) elapsed_ms: u64,
}

impl AudioGraphCounts {
    /// One-line content-free rendering for the armed bundle's hang context.
    pub(crate) fn render_line(&self) -> String {
        format!(
            "running_audio_process_count: {}\ntap_count: {}\ndevice_count: {}\ndevices_running_count: {}\naudio_graph_query_timed_out: {}\naudio_graph_query_elapsed_ms: {}",
            self.running_audio_process_count,
            self.tap_count,
            self.device_count,
            self.devices_running_count,
            self.query_timed_out,
            self.elapsed_ms
        )
    }
}

pub(crate) struct AudioGraphSnapshot {
    pub(crate) counts: AudioGraphCounts,
    /// Identity-bearing rendering. Armed bundle only.
    pub(crate) report: String,
}

// ---------------------------------------------------------------------------
// Pure counting and rendering
// ---------------------------------------------------------------------------

pub(crate) fn counts_for(
    observation: &AudioGraphObservation,
    query_timed_out: bool,
    elapsed_ms: u64,
) -> AudioGraphCounts {
    AudioGraphCounts {
        running_audio_process_count: observation
            .processes
            .iter()
            .filter(|process| process.has_live_io())
            .count() as u64,
        tap_count: observation.taps.len() as u64,
        device_count: observation.devices.len() as u64,
        devices_running_count: observation
            .devices
            .iter()
            .filter(|device| device.running_somewhere == Some(true))
            .count() as u64,
        query_timed_out,
        elapsed_ms,
    }
}

fn flag(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}

/// Render a FourCC as its ASCII tag when every byte is printable, else hex.
fn four_cc(value: u32) -> String {
    let bytes = value.to_be_bytes();
    if bytes.iter().all(|byte| (0x20..=0x7e).contains(byte)) {
        format!("'{}'", String::from_utf8_lossy(&bytes))
    } else {
        format!("0x{value:08x}")
    }
}

/// Clamp untrusted HAL text (device names, tap UIDs) to a bounded single line.
fn bounded_text(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(MAX_TEXT_CHARS)
        .collect();
    cleaned.trim().to_string()
}

pub(crate) fn render_report(
    observation: &AudioGraphObservation,
    process_names: &BTreeMap<i32, String>,
) -> String {
    let counts = counts_for(observation, false, 0);
    let mut report = String::new();

    for note in &observation.notes {
        report.push_str(&format!("note: {note}\n"));
    }

    report.push_str(&format!(
        "process objects: {} ({} with live audio IO){}\n",
        observation.processes.len(),
        counts.running_audio_process_count,
        if observation.processes_truncated {
            " [truncated]"
        } else {
            ""
        }
    ));
    for process in &observation.processes {
        let name = process
            .pid
            .and_then(|pid| process_names.get(&pid))
            .map(|name| bounded_text(name))
            .unwrap_or_else(|| "<unknown>".to_string());
        report.push_str(&format!(
            "  object {} pid {} ({}) running={} input={} output={}\n",
            process.object_id,
            process
                .pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            name,
            flag(process.running),
            flag(process.running_input),
            flag(process.running_output),
        ));
    }

    report.push_str(&format!(
        "\ntap objects: {}{}\n",
        observation.taps.len(),
        if observation.taps_truncated {
            " [truncated]"
        } else {
            ""
        }
    ));
    for tap in &observation.taps {
        report.push_str(&format!(
            "  object {} uid {}\n",
            tap.object_id,
            tap.uid
                .as_deref()
                .map(bounded_text)
                .unwrap_or_else(|| "<unknown>".to_string()),
        ));
    }

    report.push_str(&format!(
        "\ndevices: {} ({} running somewhere){}\n",
        observation.devices.len(),
        counts.devices_running_count,
        if observation.devices_truncated {
            " [truncated]"
        } else {
            ""
        }
    ));
    for device in &observation.devices {
        report.push_str(&format!(
            "  object {} name {} transport {} running_somewhere={}{}\n",
            device.object_id,
            device
                .name
                .as_deref()
                .map(bounded_text)
                .unwrap_or_else(|| "<unknown>".to_string()),
            device
                .transport_type
                .map(four_cc)
                .unwrap_or_else(|| "unknown".to_string()),
            flag(device.running_somewhere),
            device
                .sub_device_count
                .map(|count| format!(" sub_devices={count}"))
                .unwrap_or_default(),
        ));
    }

    truncate_at_boundary(&mut report, REPORT_CAP_BYTES);
    report
}

// ---------------------------------------------------------------------------
// Deadline-guarded query entry point
// ---------------------------------------------------------------------------

/// Query the HAL on a throwaway thread with a hard deadline and render the
/// result. Safe to call from any non-critical-path thread; never called from
/// the capture supervisor.
pub(crate) fn snapshot() -> AudioGraphSnapshot {
    let started = Instant::now();
    let (sender, receiver) = mpsc::channel();
    // Deliberately detached: if coreaudiod is wedged this thread stays blocked
    // inside the HAL until the server recovers. Abandoning it is the whole
    // point — the process must not wait on a hung audio server.
    std::thread::Builder::new()
        .name("murmur-audio-graph-probe".to_string())
        .spawn(move || {
            let _ = sender.send(query_audio_graph());
        })
        .ok();

    match receiver.recv_timeout(QUERY_DEADLINE) {
        Ok(observation) => {
            let elapsed_ms = started.elapsed().as_millis() as u64;
            let names = process_names(&observation);
            AudioGraphSnapshot {
                counts: counts_for(&observation, false, elapsed_ms),
                report: render_report(&observation, &names),
            }
        }
        Err(_) => AudioGraphSnapshot {
            counts: AudioGraphCounts {
                query_timed_out: true,
                elapsed_ms: started.elapsed().as_millis() as u64,
                ..AudioGraphCounts::default()
            },
            report: QUERY_TIMED_OUT_BODY.to_string(),
        },
    }
}

/// Resolve PID to process name with one bounded `ps` call. Runs outside the
/// HAL deadline thread: `ps` never touches coreaudiod, so it cannot hang for
/// the reason we are investigating.
fn process_names(observation: &AudioGraphObservation) -> BTreeMap<i32, String> {
    if !observation
        .processes
        .iter()
        .any(|process| process.pid.is_some())
    {
        return BTreeMap::new();
    }
    parse_process_names(&run_capped(
        "/bin/ps",
        &["-axo", "pid=,comm="],
        PROCESS_NAME_DEADLINE,
    ))
}

/// Parse `ps -axo pid=,comm=` output. Only the trailing path component is
/// kept, so the map holds executable names rather than full paths.
fn parse_process_names(output: &str) -> BTreeMap<i32, String> {
    let mut names = BTreeMap::new();
    for line in output.lines() {
        let line = line.trim_start();
        let Some((pid, command)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(pid) = pid.parse::<i32>() else {
            continue;
        };
        let command = command.trim();
        if command.is_empty() {
            continue;
        }
        let leaf = command.rsplit('/').next().unwrap_or(command);
        names.insert(pid, leaf.to_string());
    }
    names
}

// ---------------------------------------------------------------------------
// Content-free structured event, at most once per capture attempt
// ---------------------------------------------------------------------------

/// True exactly once per new capture ID.
fn take_capture_observation(last: &AtomicU64, capture_id: u64) -> bool {
    capture_id != 0 && last.fetch_max(capture_id, Ordering::Relaxed) < capture_id
}

/// Called from the capture backend timeout path on every install. Spawns its
/// own thread so the kill/fallback sequence is never delayed, and emits at
/// most one content-free event per capture attempt.
pub(crate) fn observe_capture_timeout(capture_id: u64) {
    if !take_capture_observation(&LAST_OBSERVED_CAPTURE_ID, capture_id) {
        return;
    }
    std::thread::Builder::new()
        .name("murmur-audio-graph-observe".to_string())
        .spawn(move || {
            emit_counts(capture_id, snapshot().counts);
        })
        .ok();
}

fn emit_counts(capture_id: u64, counts: AudioGraphCounts) {
    tracing::warn!(
        target: "audio",
        event_code = "audio.system_audio_graph_observed",
        capture_id,
        running_audio_process_count = counts.running_audio_process_count,
        tap_count = counts.tap_count,
        device_count = counts.device_count,
        devices_running_count = counts.devices_running_count,
        query_timed_out = counts.query_timed_out,
        elapsed_ms = counts.elapsed_ms,
        "observed the system audio graph after a capture backend timeout"
    );
}

// ---------------------------------------------------------------------------
// Murmur's own audio owners (no HAL calls)
// ---------------------------------------------------------------------------

fn app_handle_slot() -> &'static Mutex<Option<tauri::AppHandle>> {
    APP_HANDLE.get_or_init(|| Mutex::new(None))
}

/// Wired once from `lib.rs` setup so diagnostics can read managed state
/// without threading an `AppHandle` through the capture path.
pub(crate) fn set_app_handle(handle: tauri::AppHandle) {
    *app_handle_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(handle);
}

fn app_handle() -> Option<tauri::AppHandle> {
    app_handle_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// What Murmur itself holds at collection time. Every read is non-blocking:
/// a contended lock reports `<busy>` rather than waiting, because a diagnostic
/// must never join the queue it is trying to describe.
pub(crate) fn internal_owners_report() -> String {
    let mut report = String::new();

    match crate::audio_lifecycle::diagnostic_snapshot() {
        Some(lifecycle) => report.push_str(&format!(
            "capture lifecycle: phase={} owner={} still_connecting={}\n",
            lifecycle.phase, lifecycle.owner, lifecycle.still_connecting
        )),
        None => report.push_str("capture lifecycle: <supervisor never started>\n"),
    }

    let Some(handle) = app_handle() else {
        report.push_str("murmur subsystems: <app handle unavailable>\n");
        return report;
    };
    let state = tauri::Manager::state::<crate::State>(&handle);

    let preview = state.app_state.microphone_preview.status();
    report.push_str(&format!(
        "microphone preview: state={} preview_id={} still_connecting={}\n",
        preview.state,
        preview
            .preview_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "none".to_string()),
        preview.still_connecting,
    ));

    let meeting = state.meetings.status();
    report.push_str(&format!(
        "meeting capture: phase={:?} generation={} microphone_active={} system_audio_active={} error_code={}\n",
        meeting.phase,
        meeting.generation,
        meeting.microphone_active,
        meeting.system_audio_active,
        meeting.error_code.as_deref().unwrap_or("none"),
    ));
    report.push_str(&format!(
        "meeting flags: active={} inference_active={} summary_active={}\n",
        state.app_state.meeting_active.load(Ordering::SeqCst),
        state
            .app_state
            .meeting_inference_active
            .load(Ordering::SeqCst),
        state
            .app_state
            .meeting_summary_active
            .load(Ordering::SeqCst),
    ));

    let query_status = state.query.status();
    report.push_str(&format!(
        "voice query: status={} active_pass={} holds_microphone={}\n",
        query_status.as_str(),
        state
            .query
            .active_pass_id()
            .map(|id| id.to_string())
            .unwrap_or_else(|| "none".to_string()),
        query_status.blocks_capture(),
    ));

    report.push_str(&format!(
        "dictation: status={}\n",
        state
            .app_state
            .dictation_status_if_uncontended()
            .map(|status| format!("{status:?}"))
            .unwrap_or_else(|| "<busy>".to_string()),
    ));
    report.push_str(&format!(
        "transform: status={} active_pass={}\n",
        state.app_state.transform_status().as_str(),
        state
            .app_state
            .active_transform_pass_id()
            .map(|id| id.to_string())
            .unwrap_or_else(|| "none".to_string()),
    ));
    report.push_str(&format!(
        "file transcription active: {}\n",
        state.app_state.file_transcribing.load(Ordering::SeqCst),
    ));

    match state.app_state.model_runtime.lifecycle_states() {
        Some(states) if states.is_empty() => {
            report.push_str("model runtime: no model has been loaded this session\n")
        }
        Some(states) => {
            report.push_str("model runtime:\n");
            for (model, lifecycle, failure_present) in states {
                report.push_str(&format!(
                    "  {model}: {lifecycle:?} failure_present={failure_present}\n"
                ));
            }
        }
        None => report.push_str("model runtime: <busy>\n"),
    }
    report.push_str(&format!(
        "model runtime active model: {}\n",
        state
            .app_state
            .model_runtime
            .active_model_if_uncontended()
            .map(|model| model.unwrap_or_else(|| "none".to_string()))
            .unwrap_or_else(|| "<busy>".to_string()),
    ));

    truncate_at_boundary(&mut report, REPORT_CAP_BYTES);
    report
}

// ---------------------------------------------------------------------------
// macOS HAL query
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "macos"))]
fn query_audio_graph() -> AudioGraphObservation {
    AudioGraphObservation {
        notes: vec!["<audio graph introspection is macOS only>".to_string()],
        ..AudioGraphObservation::default()
    }
}

#[cfg(target_os = "macos")]
fn query_audio_graph() -> AudioGraphObservation {
    hal::query()
}

#[cfg(target_os = "macos")]
mod hal {
    //! Public Core Audio HAL property reads.
    //!
    //! Every selector below is imported from `objc2-core-audio` rather than
    //! hand-written, so the FourCC values come from Apple's own headers. The
    //! verified SDK values (MacOSX.sdk `AudioHardware.h` / `AudioHardwareBase.h`)
    //! are `kAudioHardwarePropertyDevices` `'dev#'`,
    //! `kAudioHardwarePropertyProcessObjectList` `'prs#'`,
    //! `kAudioHardwarePropertyTapList` `'tps#'`, `kAudioProcessPropertyPID`
    //! `'ppid'`, `kAudioProcessPropertyIsRunning` `'pir?'`,
    //! `kAudioProcessPropertyIsRunningInput` `'piri'`,
    //! `kAudioProcessPropertyIsRunningOutput` `'piro'`,
    //! `kAudioTapPropertyUID` `'tuid'`, `kAudioObjectPropertyName` `'lnam'`,
    //! `kAudioDevicePropertyTransportType` `'tran'`,
    //! `kAudioDevicePropertyDeviceIsRunningSomewhere` `'gone'`, and
    //! `kAudioAggregateDevicePropertyFullSubDeviceList` `'grup'`.

    use std::ffi::c_void;
    use std::ptr::NonNull;

    use objc2_core_audio::{
        kAudioAggregateDevicePropertyFullSubDeviceList,
        kAudioDevicePropertyDeviceIsRunningSomewhere, kAudioDevicePropertyTransportType,
        kAudioDeviceTransportTypeAggregate, kAudioHardwarePropertyDevices,
        kAudioHardwarePropertyProcessObjectList, kAudioHardwarePropertyTapList,
        kAudioObjectPropertyElementMain, kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal,
        kAudioObjectSystemObject, kAudioProcessPropertyIsRunning,
        kAudioProcessPropertyIsRunningInput, kAudioProcessPropertyIsRunningOutput,
        kAudioProcessPropertyPID, AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize,
        AudioObjectID, AudioObjectPropertyAddress,
    };
    use objc2_core_foundation::{CFRetained, CFString};

    use super::{
        AudioGraphObservation, DeviceObservation, ProcessObservation, TapObservation,
        MAX_DEVICE_OBJECTS, MAX_PROCESS_OBJECTS, MAX_SUB_DEVICES, MAX_TAP_OBJECTS,
    };

    fn global_address(selector: u32) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        }
    }

    fn system_object() -> AudioObjectID {
        kAudioObjectSystemObject as AudioObjectID
    }

    /// Read a fixed-size POD property. `None` on any non-zero `OSStatus` or
    /// short read: a diagnostic never guesses a value it did not receive.
    fn read_pod<T: Copy + Default>(object: AudioObjectID, selector: u32) -> Option<T> {
        let address = global_address(selector);
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
        if status != 0 || size as usize != std::mem::size_of::<T>() {
            return None;
        }
        Some(value)
    }

    fn read_bool(object: AudioObjectID, selector: u32) -> Option<bool> {
        read_pod::<u32>(object, selector).map(|value| value != 0)
    }

    /// Read a `CFStringRef`-valued property and copy it into an owned `String`.
    fn read_string(object: AudioObjectID, selector: u32) -> Option<String> {
        let address = global_address(selector);
        let mut value: *const CFString = std::ptr::null();
        let mut size = std::mem::size_of::<*const CFString>() as u32;
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
        if status != 0 {
            return None;
        }
        let value = NonNull::new(value.cast_mut())?;
        // The HAL returns a +1 reference for CFString-valued properties.
        let string = unsafe { CFRetained::from_raw(value) };
        Some(string.to_string())
    }

    /// Read an `AudioObjectID` array property, capped at `limit`. Returns the
    /// list plus whether the cap truncated it.
    fn read_object_list(
        object: AudioObjectID,
        selector: u32,
        limit: usize,
    ) -> Option<(Vec<AudioObjectID>, bool)> {
        let address = global_address(selector);
        let mut byte_size: u32 = 0;
        let status = unsafe {
            AudioObjectGetPropertyDataSize(
                object,
                NonNull::from(&address),
                0,
                std::ptr::null(),
                NonNull::from(&mut byte_size),
            )
        };
        if status != 0 {
            return None;
        }
        let count = byte_size as usize / std::mem::size_of::<AudioObjectID>();
        if count == 0 {
            return Some((Vec::new(), false));
        }
        let mut objects = vec![0 as AudioObjectID; count];
        let mut byte_size = (count * std::mem::size_of::<AudioObjectID>()) as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                object,
                NonNull::from(&address),
                0,
                std::ptr::null(),
                NonNull::from(&mut byte_size),
                NonNull::new(objects.as_mut_ptr())?.cast::<c_void>(),
            )
        };
        if status != 0 {
            return None;
        }
        let returned = byte_size as usize / std::mem::size_of::<AudioObjectID>();
        objects.truncate(returned.min(count));
        let truncated = objects.len() > limit;
        objects.truncate(limit);
        Some((objects, truncated))
    }

    pub(super) fn query() -> AudioGraphObservation {
        let mut observation = AudioGraphObservation::default();

        match read_object_list(
            system_object(),
            kAudioHardwarePropertyProcessObjectList,
            MAX_PROCESS_OBJECTS,
        ) {
            Some((objects, truncated)) => {
                observation.processes_truncated = truncated;
                observation.processes = objects
                    .into_iter()
                    .map(|object_id| ProcessObservation {
                        object_id,
                        pid: read_pod::<i32>(object_id, kAudioProcessPropertyPID),
                        running: read_bool(object_id, kAudioProcessPropertyIsRunning),
                        running_input: read_bool(object_id, kAudioProcessPropertyIsRunningInput),
                        running_output: read_bool(object_id, kAudioProcessPropertyIsRunningOutput),
                    })
                    .collect();
            }
            None => observation
                .notes
                .push("process object list unavailable".to_string()),
        }

        match read_object_list(
            system_object(),
            kAudioHardwarePropertyTapList,
            MAX_TAP_OBJECTS,
        ) {
            Some((objects, truncated)) => {
                observation.taps_truncated = truncated;
                observation.taps = objects
                    .into_iter()
                    .map(|object_id| TapObservation {
                        object_id,
                        uid: read_string(object_id, objc2_core_audio::kAudioTapPropertyUID),
                    })
                    .collect();
            }
            None => observation.notes.push("tap list unavailable".to_string()),
        }

        match read_object_list(
            system_object(),
            kAudioHardwarePropertyDevices,
            MAX_DEVICE_OBJECTS,
        ) {
            Some((objects, truncated)) => {
                observation.devices_truncated = truncated;
                observation.devices = objects
                    .into_iter()
                    .map(|object_id| {
                        let transport_type =
                            read_pod::<u32>(object_id, kAudioDevicePropertyTransportType);
                        let sub_device_count = (transport_type
                            == Some(kAudioDeviceTransportTypeAggregate))
                        .then(|| {
                            read_object_list(
                                object_id,
                                kAudioAggregateDevicePropertyFullSubDeviceList,
                                MAX_SUB_DEVICES,
                            )
                            .map(|(objects, _)| objects.len())
                        })
                        .flatten();
                        DeviceObservation {
                            object_id,
                            name: read_string(object_id, kAudioObjectPropertyName),
                            transport_type,
                            running_somewhere: read_bool(
                                object_id,
                                kAudioDevicePropertyDeviceIsRunningSomewhere,
                            ),
                            sub_device_count,
                        }
                    })
                    .collect();
            }
            None => observation
                .notes
                .push("device list unavailable".to_string()),
        }

        observation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_observation() -> AudioGraphObservation {
        AudioGraphObservation {
            processes: vec![
                ProcessObservation {
                    object_id: 70,
                    pid: Some(311),
                    running: Some(true),
                    running_input: Some(true),
                    running_output: Some(false),
                },
                ProcessObservation {
                    object_id: 71,
                    pid: Some(9001),
                    running: Some(false),
                    running_input: Some(false),
                    running_output: Some(false),
                },
                ProcessObservation {
                    object_id: 72,
                    pid: None,
                    running: None,
                    running_input: None,
                    running_output: Some(true),
                },
            ],
            taps: vec![TapObservation {
                object_id: 90,
                uid: Some("A1B2-C3D4".to_string()),
            }],
            devices: vec![
                DeviceObservation {
                    object_id: 44,
                    name: Some("MacBook Pro Microphone".to_string()),
                    transport_type: Some(u32::from_be_bytes(*b"bltn")),
                    running_somewhere: Some(false),
                    sub_device_count: None,
                },
                DeviceObservation {
                    object_id: 61,
                    name: Some("Tap Aggregate".to_string()),
                    transport_type: Some(u32::from_be_bytes(*b"grup")),
                    running_somewhere: Some(true),
                    sub_device_count: Some(2),
                },
            ],
            processes_truncated: false,
            taps_truncated: false,
            devices_truncated: false,
            notes: Vec::new(),
        }
    }

    #[test]
    fn counts_only_admit_proven_live_io_and_running_devices() {
        let counts = counts_for(&sample_observation(), false, 42);
        // Two processes prove live IO: one via `running`, one via output only.
        assert_eq!(counts.running_audio_process_count, 2);
        assert_eq!(counts.tap_count, 1);
        assert_eq!(counts.device_count, 2);
        assert_eq!(counts.devices_running_count, 1);
        assert!(!counts.query_timed_out);
        assert_eq!(counts.elapsed_ms, 42);
    }

    #[test]
    fn unknown_properties_never_invent_activity() {
        let observation = AudioGraphObservation {
            processes: vec![ProcessObservation {
                object_id: 1,
                pid: None,
                running: None,
                running_input: None,
                running_output: None,
            }],
            devices: vec![DeviceObservation {
                object_id: 2,
                name: None,
                transport_type: None,
                running_somewhere: None,
                sub_device_count: None,
            }],
            ..AudioGraphObservation::default()
        };
        let counts = counts_for(&observation, false, 0);
        assert_eq!(counts.running_audio_process_count, 0);
        assert_eq!(counts.devices_running_count, 0);
        assert_eq!(counts.device_count, 1);
    }

    #[test]
    fn report_renders_names_flags_and_four_cc_transports() {
        let mut names = BTreeMap::new();
        names.insert(311, "coreaudiod".to_string());
        let report = render_report(&sample_observation(), &names);
        assert!(report.contains("process objects: 3 (2 with live audio IO)"));
        assert!(report.contains("object 70 pid 311 (coreaudiod) running=yes input=yes output=no"));
        assert!(report.contains("object 71 pid 9001 (<unknown>)"));
        assert!(report.contains("object 72 pid unknown (<unknown>) running=unknown"));
        assert!(report.contains("tap objects: 1"));
        assert!(report.contains("object 90 uid A1B2-C3D4"));
        assert!(report.contains("devices: 2 (1 running somewhere)"));
        assert!(report.contains("transport 'bltn' running_somewhere=no"));
        assert!(report.contains("transport 'grup' running_somewhere=yes sub_devices=2"));
    }

    #[test]
    fn report_marks_truncation_and_notes_and_stays_within_the_cap() {
        let observation = AudioGraphObservation {
            processes: (0..MAX_PROCESS_OBJECTS)
                .map(|index| ProcessObservation {
                    object_id: index as u32,
                    pid: Some(index as i32),
                    running: Some(true),
                    running_input: Some(true),
                    running_output: Some(true),
                })
                .collect(),
            processes_truncated: true,
            taps_truncated: true,
            devices_truncated: true,
            notes: vec!["tap list unavailable".to_string()],
            ..AudioGraphObservation::default()
        };
        let report = render_report(&observation, &BTreeMap::new());
        assert!(report.len() <= REPORT_CAP_BYTES);
        assert!(report.contains("note: tap list unavailable"));
        assert!(report.contains("[truncated]"));
    }

    #[test]
    fn untrusted_hal_text_is_bounded_and_stripped_of_control_characters() {
        let observation = AudioGraphObservation {
            devices: vec![DeviceObservation {
                object_id: 3,
                name: Some(format!("a\nb{}", "x".repeat(500))),
                transport_type: Some(0x0102_0304),
                running_somewhere: Some(true),
                sub_device_count: None,
            }],
            ..AudioGraphObservation::default()
        };
        let report = render_report(&observation, &BTreeMap::new());
        let line = report
            .lines()
            .find(|line| line.contains("object 3"))
            .expect("device line present");
        assert!(!line.contains('\t'));
        assert!(line.contains("a b"));
        // Name clamped to MAX_TEXT_CHARS, not the raw 500-character value.
        assert!(line.len() < 260);
        // Non-printable FourCC falls back to hex rather than mojibake.
        assert!(line.contains("transport 0x01020304"));
    }

    #[test]
    fn process_name_parsing_keeps_the_executable_leaf_and_skips_junk() {
        let names = parse_process_names(
            "  311 /usr/sbin/coreaudiod\n 9001 Murmur\nnot-a-pid whatever\n 42 \n",
        );
        assert_eq!(names.get(&311).map(String::as_str), Some("coreaudiod"));
        assert_eq!(names.get(&9001).map(String::as_str), Some("Murmur"));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn a_capture_attempt_contributes_at_most_one_observation() {
        let last = AtomicU64::new(0);
        assert!(!take_capture_observation(&last, 0)); // no attempt identity
        assert!(take_capture_observation(&last, 5)); // first observation
        assert!(!take_capture_observation(&last, 5)); // same attempt: consumed
        assert!(!take_capture_observation(&last, 3)); // older attempt: never
        assert!(take_capture_observation(&last, 6)); // next attempt: once
        assert!(!take_capture_observation(&last, 6));
    }

    #[test]
    fn a_timed_out_query_reports_the_timeout_rather_than_an_empty_graph() {
        let counts = AudioGraphCounts {
            query_timed_out: true,
            elapsed_ms: 2_001,
            ..AudioGraphCounts::default()
        };
        let line = counts.render_line();
        assert!(line.contains("audio_graph_query_timed_out: true"));
        assert!(line.contains("audio_graph_query_elapsed_ms: 2001"));
        assert!(line.contains("running_audio_process_count: 0"));
        assert_eq!(
            QUERY_TIMED_OUT_BODY,
            "<audio graph query timed out after 2s>"
        );
    }

    #[test]
    fn snapshot_returns_bounded_output_within_its_deadline() {
        let started = Instant::now();
        let snapshot = snapshot();
        // The deadline plus the bounded `ps` resolution; never unbounded.
        assert!(
            started.elapsed() < QUERY_DEADLINE + PROCESS_NAME_DEADLINE + Duration::from_secs(2)
        );
        assert!(snapshot.report.len() <= REPORT_CAP_BYTES);
        if snapshot.counts.query_timed_out {
            assert_eq!(snapshot.report, QUERY_TIMED_OUT_BODY);
        }
    }

    /// Manual proof on real hardware:
    /// `cargo test print_real_audio_graph_snapshot -- --ignored --nocapture`
    #[test]
    #[ignore = "requires real Core Audio hardware; run manually"]
    fn print_real_audio_graph_snapshot() {
        let snapshot = snapshot();
        println!("counts: {:?}", snapshot.counts);
        println!("{}", snapshot.report);
        assert!(
            !snapshot.counts.query_timed_out,
            "healthy hardware must answer"
        );
        assert!(snapshot.counts.device_count > 0, "a Mac always has devices");
    }
}
