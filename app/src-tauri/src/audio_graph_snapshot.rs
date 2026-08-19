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
//! Timing: the observation runs *before* the hung worker is killed, because the
//! field evidence says the blocker frequently clears the moment the killed
//! client's transport tears down — a post-kill graph would routinely observe an
//! already-drained queue. The live-hang snapshot is therefore taken once at the
//! timeout and cached by capture ID ([`observe_capture_timeout`]); the armed
//! bundle claims it with [`take_live_hang_report`] instead of re-probing, and
//! adds a second fresh post-kill snapshot so the before/after difference is
//! itself evidence.
//!
//! Privacy: the rendered report names devices, taps, and processes, so it is
//! only ever placed in the server-armed hang bundle (same consent boundary as
//! the existing `system_profiler` sections) and is only ever *rendered* on an
//! armed install. The structured event emitted from every install carries
//! [`AudioGraphCounts`] only — integers and booleans.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::hang_diagnostics::{run_capped, truncate_at_boundary};
use crate::MutexExt;

/// Hard deadline for the HAL query itself. Two seconds is far longer than a
/// healthy HAL round trip and far shorter than a capture attempt budget.
const QUERY_DEADLINE: Duration = Duration::from_secs(2);
const QUERY_TIMED_OUT_BODY: &str = "<audio graph query timed out after 2s>";
const PROBE_UNAVAILABLE_BODY: &str = "<audio graph probe thread could not be started>";
/// Name resolution is a plain `ps` call that cannot touch coreaudiod, so it
/// cannot hang for the reason under investigation. It is still capped, and it
/// is deliberately excluded from the reported `elapsed_ms`, which measures the
/// HAL query alone.
const PROCESS_NAME_DEADLINE: Duration = Duration::from_secs(2);
/// How long the armed bundle waits for an in-flight live-hang probe. The probe
/// is bounded by `QUERY_DEADLINE`, so this only covers scheduling slack.
const LIVE_SNAPSHOT_WAIT: Duration = Duration::from_secs(3);

/// Object-list bounds. A healthy machine reports a handful of each; these caps
/// only stop a pathological HAL from producing an unbounded report — and they
/// bound the *allocation*, not just the rendering.
const MAX_PROCESS_OBJECTS: usize = 256;
const MAX_TAP_OBJECTS: usize = 64;
const MAX_DEVICE_OBJECTS: usize = 64;
const MAX_TEXT_CHARS: usize = 128;
const REPORT_CAP_BYTES: usize = 64_000;

/// Highest capture ID already observed this process lifetime. A capture attempt
/// contributes at most one `audio.system_audio_graph_observed` event.
static LAST_OBSERVED_CAPTURE_ID: AtomicU64 = AtomicU64::new(0);
static APP_HANDLE: OnceLock<Mutex<Option<tauri::AppHandle>>> = OnceLock::new();
/// The live-hang snapshot taken before the kill, waiting to be claimed by the
/// armed bundle. Only populated on armed installs (an unarmed install never
/// renders an identity-bearing report at all).
#[allow(clippy::type_complexity)]
static LIVE_SNAPSHOT: OnceLock<(Mutex<Option<LiveSnapshotSlot>>, Condvar)> = OnceLock::new();

/// A pre-kill observation for one capture attempt. `report` is `None` while the
/// probe is still in flight, so a waiting bundle can tell "not finished yet"
/// from "finished with nothing".
struct LiveSnapshotSlot {
    capture_id: u64,
    report: Option<String>,
}

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
/// unarmed install: integers and booleans, no PIDs, names, or UIDs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AudioGraphCounts {
    pub(crate) running_audio_process_count: u64,
    pub(crate) tap_count: u64,
    pub(crate) device_count: u64,
    pub(crate) devices_running_count: u64,
    /// The HAL query was started but did not answer within `QUERY_DEADLINE` —
    /// direct evidence of a wedged audio server.
    pub(crate) query_timed_out: bool,
    /// The probe thread could not be started at all. Deliberately distinct
    /// from `query_timed_out`: a wedge we never observed must not be reported
    /// as one we did.
    pub(crate) probe_unavailable: bool,
    /// Wall-clock time of the HAL query alone. Name resolution and rendering
    /// happen afterwards and are excluded, so this stays comparable to
    /// `QUERY_DEADLINE`.
    pub(crate) elapsed_ms: u64,
}

impl AudioGraphCounts {
    /// One-line content-free rendering for the armed bundle's hang context.
    pub(crate) fn render_line(&self) -> String {
        format!(
            "running_audio_process_count: {}\ntap_count: {}\ndevice_count: {}\ndevices_running_count: {}\naudio_graph_query_timed_out: {}\naudio_graph_probe_unavailable: {}\naudio_graph_hal_elapsed_ms: {}",
            self.running_audio_process_count,
            self.tap_count,
            self.device_count,
            self.devices_running_count,
            self.query_timed_out,
            self.probe_unavailable,
            self.elapsed_ms
        )
    }
}

pub(crate) struct AudioGraphSnapshot {
    pub(crate) counts: AudioGraphCounts,
    /// Identity-bearing rendering, empty unless [`Detail::Full`] was requested.
    /// Armed installs only.
    pub(crate) report: String,
}

/// How much of an observation to materialize. Unarmed installs never render
/// the identity-bearing report, which also spares them the `ps` call on every
/// capture timeout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Detail {
    CountsOnly,
    Full,
}

// ---------------------------------------------------------------------------
// Pure counting and rendering
// ---------------------------------------------------------------------------

pub(crate) fn counts_for(observation: &AudioGraphObservation, elapsed_ms: u64) -> AudioGraphCounts {
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
        query_timed_out: false,
        probe_unavailable: false,
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
    let counts = counts_for(observation, 0);
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

/// Query the HAL on a throwaway thread with a hard deadline, and render the
/// result when `detail` is [`Detail::Full`]. Safe to call from any
/// non-critical-path thread; never called from the capture supervisor.
///
/// `counts.elapsed_ms` covers the HAL query only. Name resolution and
/// rendering run afterwards, off the deadline, and are excluded.
pub(crate) fn snapshot(detail: Detail) -> AudioGraphSnapshot {
    let started = Instant::now();
    let (sender, receiver) = mpsc::channel();
    // Deliberately detached: if coreaudiod is wedged this thread stays blocked
    // inside the HAL until the server recovers. Abandoning it is the whole
    // point — the process must not wait on a hung audio server.
    let spawned = std::thread::Builder::new()
        .name("murmur-audio-graph-probe".to_string())
        .spawn(move || {
            let _ = sender.send(query_audio_graph());
        });
    if spawned.is_err() {
        // We never observed the HAL, so we must not claim it was wedged.
        return AudioGraphSnapshot {
            counts: AudioGraphCounts {
                probe_unavailable: true,
                elapsed_ms: started.elapsed().as_millis() as u64,
                ..AudioGraphCounts::default()
            },
            report: report_for(detail, PROBE_UNAVAILABLE_BODY),
        };
    }

    match receiver.recv_timeout(QUERY_DEADLINE) {
        Ok(observation) => {
            let counts = counts_for(&observation, started.elapsed().as_millis() as u64);
            let report = match detail {
                Detail::CountsOnly => String::new(),
                Detail::Full => render_report(&observation, &process_names(&observation)),
            };
            AudioGraphSnapshot { counts, report }
        }
        Err(_) => AudioGraphSnapshot {
            counts: AudioGraphCounts {
                query_timed_out: true,
                elapsed_ms: started.elapsed().as_millis() as u64,
                ..AudioGraphCounts::default()
            },
            report: report_for(detail, QUERY_TIMED_OUT_BODY),
        },
    }
}

fn report_for(detail: Detail, body: &str) -> String {
    match detail {
        Detail::CountsOnly => String::new(),
        Detail::Full => body.to_string(),
    }
}

/// How many objects to allocate for a HAL-reported property size, and whether
/// the cap truncated the list.
///
/// Split out of the FFI so the bound is testable on any OS: the HAL reports
/// the size, and a pathological value must never reach `vec!` — a release
/// build aborts on allocation failure, which would turn a diagnostic into a
/// crash.
fn planned_object_read(byte_size: u32, object_size: usize, limit: usize) -> (usize, bool) {
    let reported = byte_size as usize / object_size.max(1);
    (reported.min(limit), reported > limit)
}

/// `CFArrayGetCount` returns a signed `CFIndex`. A negative count is not a
/// value we can believe, so report it as unknown rather than saturating.
fn cf_array_count_to_usize(count: isize) -> Option<usize> {
    usize::try_from(count).ok()
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
// Live-hang observation: content-free event everywhere, cached report if armed
// ---------------------------------------------------------------------------

/// True exactly once per new capture ID.
fn take_capture_observation(last: &AtomicU64, capture_id: u64) -> bool {
    capture_id != 0 && last.fetch_max(capture_id, Ordering::Relaxed) < capture_id
}

fn live_snapshot_cell() -> &'static (Mutex<Option<LiveSnapshotSlot>>, Condvar) {
    LIVE_SNAPSHOT.get_or_init(|| (Mutex::new(None), Condvar::new()))
}

/// Called from the capture backend timeout path on every install, *before* the
/// hung worker is killed. Spawns its own thread so the kill/fallback sequence
/// is never delayed, emits at most one content-free event per capture attempt,
/// and — on an armed install — caches the identity-bearing report for the
/// bundle to claim so the bundle never has to re-probe for the live view.
pub(crate) fn observe_capture_timeout(capture_id: u64) {
    if !take_capture_observation(&LAST_OBSERVED_CAPTURE_ID, capture_id) {
        return;
    }
    // Deciding the detail here, on the caller's side of the spawn, keeps the
    // rule simple: an unarmed install never renders the report at all.
    let detail = if crate::hang_diagnostics::armed() {
        Detail::Full
    } else {
        Detail::CountsOnly
    };
    if detail == Detail::Full {
        // Publish the pending slot before spawning, so a bundle that starts
        // collecting while the probe is still running waits for it instead of
        // concluding there was no live observation.
        let (slot, _) = live_snapshot_cell();
        *slot.lock_or_recover() = Some(LiveSnapshotSlot {
            capture_id,
            report: None,
        });
    }
    let spawned = std::thread::Builder::new()
        .name("murmur-audio-graph-observe".to_string())
        .spawn(move || {
            let observation = snapshot(detail);
            if detail == Detail::Full {
                publish_live_snapshot(capture_id, observation.report);
            }
            emit_counts(capture_id, observation.counts);
        });
    if spawned.is_err() && detail == Detail::Full {
        // Never leave a bundle waiting on a probe that will never run.
        publish_live_snapshot(capture_id, PROBE_UNAVAILABLE_BODY.to_string());
    }
}

fn publish_live_snapshot(capture_id: u64, report: String) {
    let (slot, ready) = live_snapshot_cell();
    let mut guard = slot.lock_or_recover();
    // A newer capture attempt owns the slot now; drop this stale report.
    if guard
        .as_ref()
        .is_some_and(|pending| pending.capture_id == capture_id)
    {
        *guard = Some(LiveSnapshotSlot {
            capture_id,
            report: Some(report),
        });
    }
    drop(guard);
    ready.notify_all();
}

/// Claim the pre-kill report for `capture_id`, waiting briefly if its probe is
/// still in flight. Returns `None` when no live observation exists for this
/// attempt, so the caller can say so explicitly rather than imply one was
/// taken. Consuming the slot keeps a stale report out of a later bundle.
pub(crate) fn take_live_hang_report(capture_id: u64) -> Option<String> {
    let (slot, ready) = live_snapshot_cell();
    let mut guard = slot.lock_or_recover();
    let deadline = Instant::now() + LIVE_SNAPSHOT_WAIT;
    loop {
        match guard.as_ref() {
            // Some other attempt's observation, or none at all.
            None => return None,
            Some(pending) if pending.capture_id != capture_id => return None,
            Some(pending) if pending.report.is_some() => {
                return guard.take().and_then(|pending| pending.report);
            }
            // In flight: wait for it rather than starting a second probe.
            Some(_) => {}
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return None;
        };
        let (next, timeout) = ready
            .wait_timeout(guard, remaining)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard = next;
        if timeout.timed_out() {
            return None;
        }
    }
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
        probe_unavailable = counts.probe_unavailable,
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
    // `state()` panics when the type is unmanaged, and release builds are
    // `panic = "abort"`: a diagnostic must never be able to take the process
    // down, so resolve it fallibly.
    let Some(state) = tauri::Manager::try_state::<crate::State>(&handle) else {
        report.push_str("murmur subsystems: <app state unavailable>\n");
        return report;
    };

    match state.app_state.microphone_preview.status_if_uncontended() {
        Some(preview) => report.push_str(&format!(
            "microphone preview: state={} preview_id={} still_connecting={}\n",
            preview.state,
            preview
                .preview_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "none".to_string()),
            preview.still_connecting,
        )),
        None => report.push_str("microphone preview: <busy>\n"),
    }

    match state.meetings.status_if_uncontended() {
        Some(meeting) => report.push_str(&format!(
            "meeting capture: phase={:?} generation={} microphone_active={} system_audio_active={} error_code={}\n",
            meeting.phase,
            meeting.generation,
            meeting.microphone_active,
            meeting.system_audio_active,
            meeting.error_code.as_deref().unwrap_or("none"),
        )),
        None => report.push_str("meeting capture: <busy>\n"),
    }
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

    // The active pass ID is an atomic, so it is reportable even when the
    // status lock itself is contended.
    let query_pass = state
        .query
        .active_pass_id()
        .map(|id| id.to_string())
        .unwrap_or_else(|| "none".to_string());
    match state.query.status_if_uncontended() {
        Some(status) => report.push_str(&format!(
            "voice query: status={} active_pass={query_pass} holds_microphone={}\n",
            status.as_str(),
            status.blocks_capture(),
        )),
        None => report.push_str(&format!(
            "voice query: status=<busy> active_pass={query_pass}\n"
        )),
    }

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
        state
            .app_state
            .transform_status_if_uncontended()
            .map(|status| status.as_str().to_string())
            .unwrap_or_else(|| "<busy>".to_string()),
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
    use objc2_core_foundation::{CFArray, CFRetained, CFString};

    use super::{
        AudioGraphObservation, DeviceObservation, ProcessObservation, TapObservation,
        MAX_DEVICE_OBJECTS, MAX_PROCESS_OBJECTS, MAX_TAP_OBJECTS,
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
        // Safety hinges on the HAL having written a whole pointer: a short or
        // odd-sized read would leave `value` part stale stack bytes, and
        // `from_raw` would then release memory we never owned. Fail closed.
        let value = read_cf_object::<CFString>(object, selector)?;
        Some(value.to_string())
    }

    /// Read a `CFTypeRef`-valued property, taking ownership of the +1
    /// reference the HAL hands back (these properties are documented as
    /// "the caller is responsible for releasing the returned CFObject").
    fn read_cf_object<T: objc2_core_foundation::Type>(
        object: AudioObjectID,
        selector: u32,
    ) -> Option<CFRetained<T>> {
        let address = global_address(selector);
        let mut value: *const T = std::ptr::null();
        let mut size = std::mem::size_of::<*const T>() as u32;
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
        if status != 0 || size as usize != std::mem::size_of::<*const T>() {
            return None;
        }
        let value = NonNull::new(value.cast_mut())?;
        Some(unsafe { CFRetained::from_raw(value) })
    }

    /// Count the entries of a `CFArray`-valued property.
    ///
    /// `kAudioAggregateDevicePropertyFullSubDeviceList` is documented as "a
    /// CFArray of CFStrings that contain the UIDs" — *not* an `AudioObjectID`
    /// array. Reading it as one both miscounts (a pointer-sized read divided by
    /// four) and leaks the array's +1 reference on every aggregate device.
    /// `CFRetained` releases it for us here.
    fn read_cf_array_count(object: AudioObjectID, selector: u32) -> Option<usize> {
        let array = read_cf_object::<CFArray>(object, selector)?;
        super::cf_array_count_to_usize(array.count())
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
        // Clamp the allocation to the cap BEFORE allocating: a pathological
        // HAL-reported size would otherwise OOM the probe thread, and release
        // builds abort on allocation failure.
        let (count, truncated) = super::planned_object_read(
            byte_size,
            std::mem::size_of::<AudioObjectID>(),
            limit,
        );
        if count == 0 {
            return Some((Vec::new(), truncated));
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
                            read_cf_array_count(
                                object_id,
                                kAudioAggregateDevicePropertyFullSubDeviceList,
                            )
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
        let counts = counts_for(&sample_observation(), 42);
        // Two processes prove live IO: one via `running`, one via output only.
        assert_eq!(counts.running_audio_process_count, 2);
        assert_eq!(counts.tap_count, 1);
        assert_eq!(counts.device_count, 2);
        assert_eq!(counts.devices_running_count, 1);
        assert!(!counts.query_timed_out);
        assert!(!counts.probe_unavailable);
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
        let counts = counts_for(&observation, 0);
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
        assert!(line.contains("audio_graph_probe_unavailable: false"));
        assert!(line.contains("audio_graph_hal_elapsed_ms: 2001"));
        assert!(line.contains("running_audio_process_count: 0"));
        assert_eq!(
            QUERY_TIMED_OUT_BODY,
            "<audio graph query timed out after 2s>"
        );
    }

    #[test]
    fn an_unstartable_probe_is_not_reported_as_an_observed_wedge() {
        // The two failures are distinct evidence and must stay distinguishable
        // in the bundle's hang context, not just in telemetry.
        let unavailable = AudioGraphCounts {
            probe_unavailable: true,
            ..AudioGraphCounts::default()
        };
        let line = unavailable.render_line();
        assert!(line.contains("audio_graph_probe_unavailable: true"));
        assert!(line.contains("audio_graph_query_timed_out: false"));
        assert_ne!(PROBE_UNAVAILABLE_BODY, QUERY_TIMED_OUT_BODY);
    }

    #[test]
    fn the_allocation_cap_is_applied_before_allocating() {
        let object_size = std::mem::size_of::<u32>();
        // A pathological HAL-reported size must not become a 1 GiB `vec!`.
        let (count, truncated) = planned_object_read(u32::MAX, object_size, MAX_PROCESS_OBJECTS);
        assert_eq!(count, MAX_PROCESS_OBJECTS);
        assert!(truncated);
        // Ordinary sizes pass through untouched and are not marked truncated.
        let (count, truncated) = planned_object_read((7 * object_size) as u32, object_size, 64);
        assert_eq!(count, 7);
        assert!(!truncated);
        // Exactly at the cap is not truncation.
        let (count, truncated) = planned_object_read((64 * object_size) as u32, object_size, 64);
        assert_eq!(count, 64);
        assert!(!truncated);
        // An empty list stays empty.
        assert_eq!(planned_object_read(0, object_size, 64), (0, false));
    }

    #[test]
    fn a_cf_array_count_is_only_believed_when_it_is_non_negative() {
        // `kAudioAggregateDevicePropertyFullSubDeviceList` is a CFArray of
        // CFString UIDs, so the sub-device count comes from CFArrayGetCount —
        // a signed CFIndex.
        assert_eq!(cf_array_count_to_usize(0), Some(0));
        assert_eq!(cf_array_count_to_usize(3), Some(3));
        assert_eq!(cf_array_count_to_usize(-1), None);
        assert_eq!(cf_array_count_to_usize(isize::MIN), None);
    }

    #[test]
    fn snapshot_returns_bounded_output_within_its_deadline() {
        let started = Instant::now();
        let snapshot = snapshot(Detail::Full);
        // The deadline plus the bounded `ps` resolution; never unbounded.
        assert!(
            started.elapsed() < QUERY_DEADLINE + PROCESS_NAME_DEADLINE + Duration::from_secs(2)
        );
        assert!(snapshot.report.len() <= REPORT_CAP_BYTES);
        // `elapsed_ms` is the HAL query alone, so it stays inside the deadline
        // even though rendering and name resolution happened afterwards.
        assert!(snapshot.counts.elapsed_ms <= QUERY_DEADLINE.as_millis() as u64 + 500);
        if snapshot.counts.query_timed_out {
            assert_eq!(snapshot.report, QUERY_TIMED_OUT_BODY);
        }
    }

    #[test]
    fn a_counts_only_snapshot_never_materializes_the_identity_bearing_report() {
        // Unarmed installs take this path on every capture timeout; it must
        // not render names, and it must not shell out to `ps`.
        let snapshot = snapshot(Detail::CountsOnly);
        assert!(snapshot.report.is_empty());
    }

    #[test]
    fn the_bundle_claims_the_pre_kill_report_for_its_own_attempt_only() {
        // Simulates the real ordering: the timeout path publishes a pending
        // slot, the probe finishes, then the post-kill bundle claims it.
        let (slot, _) = live_snapshot_cell();
        *slot.lock_or_recover() = Some(LiveSnapshotSlot {
            capture_id: 4_100,
            report: None,
        });
        publish_live_snapshot(4_100, "during-hang graph".to_string());

        // A different attempt must never receive this report.
        assert_eq!(take_live_hang_report(4_101), None);
        // The owning attempt gets it exactly once; the slot is then consumed
        // so a later bundle cannot ship a stale live-hang view.
        assert_eq!(
            take_live_hang_report(4_100).as_deref(),
            Some("during-hang graph")
        );
        assert_eq!(take_live_hang_report(4_100), None);
    }

    #[test]
    fn a_stale_probe_result_cannot_overwrite_a_newer_attempts_slot() {
        let (slot, _) = live_snapshot_cell();
        *slot.lock_or_recover() = Some(LiveSnapshotSlot {
            capture_id: 4_200,
            report: None,
        });
        // An older attempt's probe finishing late must not land in the slot.
        publish_live_snapshot(4_199, "stale graph".to_string());
        assert_eq!(take_live_hang_report(4_200), None);
        assert_eq!(take_live_hang_report(4_199), None);
    }

    #[test]
    fn claiming_a_report_that_was_never_observed_gives_up_rather_than_waiting() {
        let (slot, _) = live_snapshot_cell();
        *slot.lock_or_recover() = None;
        let started = Instant::now();
        assert_eq!(take_live_hang_report(9_999), None);
        // No pending slot means no wait at all — the bundle is never delayed
        // by an observation that was never started.
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn the_owners_report_reports_contention_instead_of_blocking_on_it() {
        // The State-backed subsystems need a live app handle, but the
        // lifecycle line is reachable here and proves the contract's shape:
        // a diagnostic answers, it does not hang.
        let started = Instant::now();
        let report = internal_owners_report();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(report.contains("capture lifecycle:"));
        assert!(report.len() <= REPORT_CAP_BYTES);
    }

    #[test]
    fn a_held_lock_is_reported_as_busy_rather_than_waited_out() {
        // The exact primitive every owners-report reader is built on.
        let lock = Mutex::new(7);
        let held = lock.lock().expect("uncontended");
        let started = Instant::now();
        assert!(lock.try_lock_or_recover().is_none());
        assert!(started.elapsed() < Duration::from_millis(100));
        drop(held);
        assert_eq!(lock.try_lock_or_recover().map(|value| *value), Some(7));
    }

    /// Manual proof on real hardware:
    /// `cargo test print_real_audio_graph_snapshot -- --ignored --nocapture`
    #[test]
    #[ignore = "requires real Core Audio hardware; run manually"]
    fn print_real_audio_graph_snapshot() {
        let snapshot = snapshot(Detail::Full);
        println!("counts: {:?}", snapshot.counts);
        println!("{}", snapshot.report);
        assert!(
            !snapshot.counts.query_timed_out,
            "healthy hardware must answer"
        );
        assert!(!snapshot.counts.probe_unavailable);
        assert!(snapshot.counts.device_count > 0, "a Mac always has devices");
    }
}
