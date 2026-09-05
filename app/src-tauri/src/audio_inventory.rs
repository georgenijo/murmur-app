//! One shared, bounded microphone inventory for every local consumer.
//!
//! Reads never enumerate an already-attempted cache. On macOS, a supervised
//! passive capture worker is the primary topology/default-input invalidation
//! source. A backend-owned five-minute timer is only its bounded watchdog.
//! Refreshes use the same idle HAL boundary as capture startup and defer until
//! the owned capture worker returns to Idle. One app-owned reaper service must
//! start before any inventory helper; startup fails closed if it cannot.
//! Unsupported platforms publish an immediate unavailable snapshot and never
//! try to spawn the macOS helper or its reaper service.

use crate::audio::{AudioDeviceDescriptor, EnumeratedAudioInputInventory};
use crate::microphone_auto::{self, SmartAutoRequest, SmartAutoSelection};
use crate::MutexExt;
use murmur_capture_helper_protocol::ProductionLidState;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;
#[cfg(target_os = "macos")]
use std::time::Instant;
use tauri::Emitter;

const INVENTORY_SCHEMA_VERSION: u8 = 2;
#[cfg(target_os = "macos")]
const TOPOLOGY_REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);
const INITIAL_REFRESH_WAIT: Duration = Duration::from_secs(8);
const MAX_DEVICE_COUNT: usize = 256;
const MAX_DEVICE_ID_BYTES: usize = 4_096;
const MAX_DEVICE_NAME_BYTES: usize = 512;
const INVENTORY_EVENT: &str = "audio-input-inventory-changed";
#[cfg(target_os = "macos")]
const MAX_CONSECUTIVE_WATCH_FAILURES: usize = 3;
#[cfg(target_os = "macos")]
const WATCH_RESTART_BASE_DELAY: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AudioInputInventoryStatus {
    Available,
    Stale,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AudioInputInventoryErrorCode {
    NotInitialized,
    CaptureActive,
    RefreshPending,
    EnumerationFailed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioInputInventorySnapshot {
    schema_version: u8,
    revision: u64,
    status: AudioInputInventoryStatus,
    devices: Vec<AudioDeviceDescriptor>,
    default_input_id: Option<String>,
    lid_state: ProductionLidState,
    error_code: Option<AudioInputInventoryErrorCode>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AudioInputInventoryAggregate {
    pub(crate) default_input_available: bool,
    pub(crate) input_device_count: usize,
    pub(crate) enumeration_ok: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AudioInputTopology {
    devices: Vec<AudioDeviceDescriptor>,
    default_input_id: Option<String>,
    lid_state: ProductionLidState,
}

#[derive(Default)]
struct InventoryState {
    revision: u64,
    topology: Option<AudioInputTopology>,
    latest_error: Option<AudioInputInventoryErrorCode>,
    attempted: bool,
    pending: bool,
    in_flight: bool,
    deferred_until_idle: bool,
    invalidated: bool,
    invalidation_epoch: u64,
    claimed_epoch: u64,
    shutting_down: bool,
}

#[derive(Default)]
struct AudioInputInventoryCoordinator {
    state: Mutex<InventoryState>,
    changed: Condvar,
}

impl AudioInputInventoryCoordinator {
    fn snapshot_locked(state: &InventoryState) -> AudioInputInventorySnapshot {
        let (status, error_code) = if state.invalidated {
            (
                if state.topology.is_some() {
                    AudioInputInventoryStatus::Stale
                } else {
                    AudioInputInventoryStatus::Unavailable
                },
                Some(AudioInputInventoryErrorCode::RefreshPending),
            )
        } else if !state.attempted {
            (
                AudioInputInventoryStatus::Unavailable,
                Some(AudioInputInventoryErrorCode::NotInitialized),
            )
        } else {
            match (&state.topology, state.latest_error) {
                (Some(_), None) => (AudioInputInventoryStatus::Available, None),
                (Some(_), Some(error)) => (AudioInputInventoryStatus::Stale, Some(error)),
                (None, Some(error)) => (AudioInputInventoryStatus::Unavailable, Some(error)),
                (None, None) => (
                    AudioInputInventoryStatus::Unavailable,
                    Some(AudioInputInventoryErrorCode::EnumerationFailed),
                ),
            }
        };
        let (devices, default_input_id, lid_state) = state
            .topology
            .as_ref()
            .map(|topology| {
                (
                    topology.devices.clone(),
                    topology.default_input_id.clone(),
                    topology.lid_state,
                )
            })
            .unwrap_or((Vec::new(), None, ProductionLidState::Unknown));
        AudioInputInventorySnapshot {
            schema_version: INVENTORY_SCHEMA_VERSION,
            revision: state.revision,
            status,
            devices,
            default_input_id,
            lid_state,
            error_code,
        }
    }

    fn snapshot(&self) -> AudioInputInventorySnapshot {
        Self::snapshot_locked(&self.state.lock_or_recover())
    }

    fn request_refresh(&self) -> bool {
        let mut state = self.state.lock_or_recover();
        if state.shutting_down || state.in_flight {
            return false;
        }
        state.pending = true;
        true
    }

    fn ensure_initial_refresh_requested(&self) -> bool {
        let mut state = self.state.lock_or_recover();
        if state.shutting_down || state.attempted || state.pending || state.in_flight {
            return false;
        }
        state.pending = true;
        true
    }

    fn invalidate(&self) -> Option<(AudioInputInventorySnapshot, bool)> {
        let mut state = self.state.lock_or_recover();
        if state.shutting_down {
            return None;
        }
        let before = Self::snapshot_locked(&state);
        state.invalidation_epoch = state.invalidation_epoch.wrapping_add(1);
        state.invalidated = true;
        state.pending = true;
        let after_without_revision = Self::snapshot_locked(&state);
        let changed = before.status != after_without_revision.status
            || before.error_code != after_without_revision.error_code;
        if changed {
            state.revision = state.revision.wrapping_add(1).max(1);
        }
        let after = Self::snapshot_locked(&state);
        self.changed.notify_all();
        Some((after, changed))
    }

    fn claim_refresh(&self, after_idle: bool) -> bool {
        let mut state = self.state.lock_or_recover();
        if state.shutting_down
            || !state.pending
            || state.in_flight
            || (state.deferred_until_idle && !after_idle)
        {
            return false;
        }
        state.pending = false;
        state.in_flight = true;
        state.deferred_until_idle = false;
        state.claimed_epoch = state.invalidation_epoch;
        true
    }

    fn defer_refresh_until_idle(&self) {
        let mut state = self.state.lock_or_recover();
        state.pending = true;
        state.in_flight = false;
        state.deferred_until_idle = true;
        self.changed.notify_all();
    }

    fn release_refresh_claim(&self) {
        let mut state = self.state.lock_or_recover();
        state.pending = true;
        state.in_flight = false;
        self.changed.notify_all();
    }

    fn finish_refresh(
        &self,
        result: Result<EnumeratedAudioInputInventory, String>,
    ) -> (AudioInputInventorySnapshot, bool) {
        let mut state = self.state.lock_or_recover();
        let before = Self::snapshot_locked(&state);
        state.attempted = true;
        state.in_flight = false;
        let invalidated_during_refresh = state.claimed_epoch != state.invalidation_epoch;
        match result.and_then(normalize_inventory) {
            Ok(topology) => {
                state.topology = Some(topology);
                state.latest_error = None;
                state.invalidated = invalidated_during_refresh;
            }
            Err(_) => {
                // Retain the last topology only as explicitly stale display
                // data. Consumers cannot treat it as authoritative presence.
                state.latest_error = Some(AudioInputInventoryErrorCode::EnumerationFailed);
                state.invalidated = false;
            }
        }
        let after_without_revision = Self::snapshot_locked(&state);
        let changed = before.status != after_without_revision.status
            || before.devices != after_without_revision.devices
            || before.default_input_id != after_without_revision.default_input_id
            || before.lid_state != after_without_revision.lid_state
            || before.error_code != after_without_revision.error_code;
        if changed {
            state.revision = state.revision.wrapping_add(1).max(1);
        }
        let after = Self::snapshot_locked(&state);
        self.changed.notify_all();
        (after, changed)
    }

    fn wait_for_initial_refresh(&self, timeout: Duration) -> AudioInputInventorySnapshot {
        let state = self.state.lock_or_recover();
        if state.attempted || !state.in_flight {
            return Self::snapshot_locked(&state);
        }
        let (state, _) = self
            .changed
            .wait_timeout_while(state, timeout, |state| !state.attempted && state.in_flight)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Self::snapshot_locked(&state)
    }

    fn begin_shutdown(&self) {
        let mut state = self.state.lock_or_recover();
        state.shutting_down = true;
        state.pending = false;
        self.changed.notify_all();
    }
}

fn normalize_inventory(
    inventory: EnumeratedAudioInputInventory,
) -> Result<AudioInputTopology, String> {
    if inventory.devices.len() > MAX_DEVICE_COUNT {
        return Err("microphone inventory exceeds the supported bound".to_string());
    }
    let mut devices = inventory.devices;
    if devices.iter().any(|device| {
        device.id.is_empty()
            || device.name.is_empty()
            || device.id.contains('\0')
            || device.name.contains('\0')
            || device.id.len() > MAX_DEVICE_ID_BYTES
            || device.name.len() > MAX_DEVICE_NAME_BYTES
    }) {
        return Err("microphone inventory contains an invalid descriptor".to_string());
    }
    devices.sort_by(|left, right| left.id.cmp(&right.id).then(left.name.cmp(&right.name)));
    if devices.windows(2).any(|pair| pair[0].id == pair[1].id) {
        return Err("microphone inventory contains duplicate stable identifiers".to_string());
    }
    let default_input_id = inventory
        .default_input_id
        .filter(|id| devices.iter().any(|device| device.id == *id));
    Ok(AudioInputTopology {
        devices,
        default_input_id,
        lid_state: inventory.lid_state,
    })
}

fn coordinator() -> &'static AudioInputInventoryCoordinator {
    static COORDINATOR: OnceLock<AudioInputInventoryCoordinator> = OnceLock::new();
    COORDINATOR.get_or_init(AudioInputInventoryCoordinator::default)
}

fn app_handle_slot() -> &'static Mutex<Option<tauri::AppHandle>> {
    static APP_HANDLE: OnceLock<Mutex<Option<tauri::AppHandle>>> = OnceLock::new();
    APP_HANDLE.get_or_init(|| Mutex::new(None))
}

struct InputTopologyWatchSupervisor {
    shutdown: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

struct InventoryRefreshTimer {
    stop: Arc<(Mutex<bool>, Condvar)>,
    join: Option<JoinHandle<()>>,
}

fn watch_supervisor_slot() -> &'static Mutex<Option<InputTopologyWatchSupervisor>> {
    static SUPERVISOR: OnceLock<Mutex<Option<InputTopologyWatchSupervisor>>> = OnceLock::new();
    SUPERVISOR.get_or_init(|| Mutex::new(None))
}

fn refresh_timer_slot() -> &'static Mutex<Option<InventoryRefreshTimer>> {
    static TIMER: OnceLock<Mutex<Option<InventoryRefreshTimer>>> = OnceLock::new();
    TIMER.get_or_init(|| Mutex::new(None))
}

fn refresh_threads() -> &'static Mutex<Vec<JoinHandle<()>>> {
    static THREADS: OnceLock<Mutex<Vec<JoinHandle<()>>>> = OnceLock::new();
    THREADS.get_or_init(|| Mutex::new(Vec::new()))
}

fn emit_inventory(snapshot: &AudioInputInventorySnapshot) {
    let app_handle = app_handle_slot().lock_or_recover().clone();
    if let Some(app_handle) = app_handle {
        let _ = app_handle.emit_to("main", INVENTORY_EVENT, snapshot.clone());
    }
}

fn detached_reap_invalidation(
    coordinator: &AudioInputInventoryCoordinator,
) -> Option<(AudioInputInventorySnapshot, bool)> {
    coordinator.invalidate()
}

/// Retract an authoritative inventory as soon as helper ownership leaves the
/// ordinary bounded termination path. This is deliberately content-free and
/// does not request or spawn a refresh while the helper spawn gate is held.
pub(crate) fn helper_entered_detached_reap() {
    let Some((snapshot, changed)) = detached_reap_invalidation(coordinator()) else {
        return;
    };
    if changed {
        emit_inventory(&snapshot);
    }
}

fn log_refresh(snapshot: &AudioInputInventorySnapshot, changed: bool, reason: &'static str) {
    tracing::info!(
        target: "audio",
        event_code = "audio.input_inventory_refreshed",
        schema_version = snapshot.schema_version,
        revision = snapshot.revision,
        status = ?snapshot.status,
        error_code = ?snapshot.error_code,
        input_device_count = snapshot.devices.len(),
        default_input_available = snapshot.default_input_id.is_some(),
        changed,
        refresh_reason = reason,
        "shared microphone inventory refresh completed"
    );
}

fn spawn_pending_refresh(reason: &'static str, after_idle: bool) {
    let mut threads = refresh_threads().lock_or_recover();
    let mut still_running = Vec::with_capacity(threads.len());
    for thread in threads.drain(..) {
        if thread.is_finished() {
            let _ = thread.join();
        } else {
            still_running.push(thread);
        }
    }
    *threads = still_running;
    if !coordinator().claim_refresh(after_idle) {
        return;
    }
    let spawn = std::thread::Builder::new()
        .name("murmur-audio-inventory-refresh".to_string())
        .spawn(move || {
            let Some(result) = crate::audio_lifecycle::with_idle_hal_boundary(|| {
                crate::audio::enumerate_input_devices()
            }) else {
                coordinator().defer_refresh_until_idle();
                tracing::info!(
                    target: "audio",
                    event_code = "audio.input_inventory_deferred",
                    refresh_reason = reason,
                    "microphone inventory refresh deferred while capture owns HAL"
                );
                return;
            };
            let (snapshot, changed) = coordinator().finish_refresh(result);
            log_refresh(&snapshot, changed, reason);
            if changed {
                emit_inventory(&snapshot);
            }
            // If a topology callback arrived during enumeration, its epoch
            // remains pending and the just-finished result stayed stale.
            spawn_pending_refresh("coalescedTopologyChange", false);
        });
    match spawn {
        Ok(thread) => threads.push(thread),
        Err(_) => coordinator().release_refresh_claim(),
    }
}

#[cfg(target_os = "macos")]
fn request_refresh(reason: &'static str) {
    coordinator().request_refresh();
    spawn_pending_refresh(reason, false);
}

#[cfg(target_os = "macos")]
fn topology_changed() {
    let Some((snapshot, changed)) = coordinator().invalidate() else {
        return;
    };
    if changed {
        emit_inventory(&snapshot);
    }
    spawn_pending_refresh("topologyChanged", false);
}

#[cfg(target_os = "macos")]
fn start_input_topology_watch_supervisor() {
    let mut slot = watch_supervisor_slot().lock_or_recover();
    if slot.is_some() {
        return;
    }
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = Arc::clone(&shutdown);
    let join = std::thread::Builder::new()
        .name("murmur-input-topology-supervisor".to_string())
        .spawn(move || {
            let mut consecutive_failures = 0_usize;
            while !worker_shutdown.load(Ordering::Acquire)
                && consecutive_failures < MAX_CONSECUTIVE_WATCH_FAILURES
            {
                let started = Instant::now();
                let result = crate::audio::run_input_topology_watch(&worker_shutdown, || {
                    topology_changed();
                });
                if worker_shutdown.load(Ordering::Acquire) {
                    break;
                }
                if result.is_err() {
                    topology_changed();
                    consecutive_failures = if started.elapsed() >= TOPOLOGY_REFRESH_INTERVAL {
                        1
                    } else {
                        consecutive_failures + 1
                    };
                    tracing::warn!(
                        target: "audio",
                        event_code = "audio.input_topology_watch_failed",
                        consecutive_failures,
                        "passive microphone topology watcher stopped"
                    );
                    let delay = WATCH_RESTART_BASE_DELAY
                        .checked_mul(1_u32 << (consecutive_failures - 1).min(2))
                        .unwrap_or(WATCH_RESTART_BASE_DELAY);
                    let deadline = Instant::now() + delay;
                    while !worker_shutdown.load(Ordering::Acquire) && Instant::now() < deadline {
                        std::thread::park_timeout(Duration::from_millis(50));
                    }
                }
            }
            if !worker_shutdown.load(Ordering::Acquire) {
                tracing::warn!(
                    target: "audio",
                    event_code = "audio.input_topology_watch_exhausted",
                    "passive microphone topology watcher restart budget exhausted"
                );
            }
        });
    match join {
        Ok(join) => {
            *slot = Some(InputTopologyWatchSupervisor {
                shutdown,
                join: Some(join),
            });
        }
        Err(_) => tracing::warn!(
            target: "audio",
            event_code = "audio.input_topology_watch_spawn_failed",
            "passive microphone topology watcher could not start"
        ),
    }
    let needs_fallback_refresh = slot.is_none();
    drop(slot);
    if needs_fallback_refresh {
        // Preserve the bounded enumeration fallback if even the supervisor
        // thread cannot be created.
        topology_changed();
    }
}

#[cfg(not(target_os = "macos"))]
fn start_input_topology_watch_supervisor() {}

#[cfg(target_os = "macos")]
fn start_refresh_timer() {
    let stop = Arc::new((Mutex::new(false), Condvar::new()));
    let worker_stop = Arc::clone(&stop);
    let join = std::thread::Builder::new()
        .name("murmur-audio-inventory-watchdog".to_string())
        .spawn(move || loop {
            let (lock, changed) = &*worker_stop;
            let stopped = lock.lock_or_recover();
            let (stopped, _) = changed
                .wait_timeout_while(stopped, TOPOLOGY_REFRESH_INTERVAL, |stopped| !*stopped)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if *stopped {
                break;
            }
            drop(stopped);
            request_refresh("boundedTtl");
        });
    if let Ok(join) = join {
        *refresh_timer_slot().lock_or_recover() = Some(InventoryRefreshTimer {
            stop,
            join: Some(join),
        });
    }
}

#[cfg(not(target_os = "macos"))]
fn start_refresh_timer() {}

pub(crate) fn initialize(app_handle: tauri::AppHandle) {
    *app_handle_slot().lock_or_recover() = Some(app_handle);
    #[cfg(target_os = "macos")]
    if !crate::audio::start_inventory_helper_reaper_service() {
        coordinator().request_refresh();
        if coordinator().claim_refresh(false) {
            let (snapshot, changed) = coordinator().finish_refresh(Err(
                "microphone inventory reaper service unavailable".to_string(),
            ));
            log_refresh(&snapshot, changed, "reaperServiceUnavailable");
            if changed {
                emit_inventory(&snapshot);
            }
        }
        return;
    }
    #[cfg(not(target_os = "macos"))]
    {
        coordinator().request_refresh();
        if coordinator().claim_refresh(false) {
            let _ = coordinator().finish_refresh(Err("unsupported platform".to_string()));
        }
    }
    start_input_topology_watch_supervisor();
    start_refresh_timer();
}

pub(crate) fn lifecycle_became_idle() {
    spawn_pending_refresh("lifecycleIdle", true);
}

pub(crate) fn shutdown() {
    coordinator().begin_shutdown();
    *app_handle_slot().lock_or_recover() = None;
    let mut supervisor = watch_supervisor_slot().lock_or_recover().take();
    if let Some(supervisor) = supervisor.as_mut() {
        supervisor.shutdown.store(true, Ordering::Release);
        if let Some(join) = supervisor.join.take() {
            let _ = join.join();
        }
    }
    let mut timer = refresh_timer_slot().lock_or_recover().take();
    if let Some(timer) = timer.as_mut() {
        let (stop, changed) = &*timer.stop;
        *stop.lock_or_recover() = true;
        changed.notify_all();
        if let Some(join) = timer.join.take() {
            let _ = join.join();
        }
    }
    let refreshes = std::mem::take(&mut *refresh_threads().lock_or_recover());
    for refresh in refreshes {
        let _ = refresh.join();
    }
    #[cfg(target_os = "macos")]
    crate::audio::shutdown_inventory_helper_reaper_service();
}

pub(crate) fn get_inventory() -> AudioInputInventorySnapshot {
    coordinator().ensure_initial_refresh_requested();
    // Also drains a pending claim left by a bounded thread-spawn failure.
    // Normal warm reads remain a mutex-only no-op because pending is false.
    spawn_pending_refresh("firstReader", false);
    let snapshot = coordinator().wait_for_initial_refresh(INITIAL_REFRESH_WAIT);
    if snapshot.error_code == Some(AudioInputInventoryErrorCode::NotInitialized)
        && crate::audio_lifecycle::is_audio_active()
    {
        return AudioInputInventorySnapshot {
            error_code: Some(AudioInputInventoryErrorCode::CaptureActive),
            ..snapshot
        };
    }
    snapshot
}

pub(crate) fn available_devices() -> Result<Vec<AudioDeviceDescriptor>, String> {
    let snapshot = get_inventory();
    if snapshot.status == AudioInputInventoryStatus::Available {
        Ok(snapshot.devices)
    } else {
        Err("The microphone inventory is not currently available".to_string())
    }
}

/// Resolve only from the current authoritative cache. This never asks Core
/// Audio for a fresh list or opens a device, so a Smart Auto choice is bounded
/// to the same idle-only inventory contract as every other consumer.
pub(crate) fn resolve_smart_auto(request: &SmartAutoRequest) -> Result<SmartAutoSelection, String> {
    let state = coordinator().state.lock_or_recover();
    if state.invalidated || state.latest_error.is_some() || !state.attempted {
        return Err("Smart Auto needs a current microphone inventory.".to_string());
    }
    let Some(topology) = state.topology.as_ref() else {
        return Err("Smart Auto needs a current microphone inventory.".to_string());
    };
    microphone_auto::select(
        request,
        &topology.devices,
        topology.default_input_id.as_deref(),
        // Do not reuse the inventory's lid state: closing a MacBook lid can
        // leave both its built-in input and macOS default unchanged.
        microphone_auto::current_lid_state(),
    )
    .map_err(str::to_string)
}

pub(crate) fn privacy_aggregate() -> AudioInputInventoryAggregate {
    privacy_aggregate_for_snapshot(&coordinator().snapshot())
}

fn privacy_aggregate_for_snapshot(
    snapshot: &AudioInputInventorySnapshot,
) -> AudioInputInventoryAggregate {
    if snapshot.status == AudioInputInventoryStatus::Available {
        AudioInputInventoryAggregate {
            default_input_available: snapshot.default_input_id.is_some(),
            input_device_count: snapshot.devices.len(),
            enumeration_ok: true,
        }
    } else {
        AudioInputInventoryAggregate {
            default_input_available: false,
            input_device_count: 0,
            enumeration_ok: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    fn device(id: &str, name: &str) -> AudioDeviceDescriptor {
        AudioDeviceDescriptor {
            id: id.to_string(),
            name: name.to_string(),
            kind: murmur_capture_helper_protocol::ProductionDeviceKind::External,
            connected: true,
            has_input: true,
        }
    }

    fn topology(ids: &[(&str, &str)], default: Option<&str>) -> EnumeratedAudioInputInventory {
        EnumeratedAudioInputInventory {
            devices: ids.iter().map(|(id, name)| device(id, name)).collect(),
            default_input_id: default.map(str::to_string),
            lid_state: ProductionLidState::Open,
        }
    }

    #[test]
    fn unchanged_refresh_keeps_revision_and_failed_refresh_becomes_stale() {
        let coordinator = AudioInputInventoryCoordinator::default();
        coordinator.request_refresh();
        assert!(coordinator.claim_refresh(false));
        let (first, changed) = coordinator.finish_refresh(Ok(topology(
            &[("uid-b", "B"), ("uid-a", "A")],
            Some("uid-a"),
        )));
        assert!(changed);
        assert_eq!(first.revision, 1);
        assert_eq!(first.status, AudioInputInventoryStatus::Available);
        assert_eq!(first.devices[0].id, "uid-a");

        coordinator.request_refresh();
        assert!(coordinator.claim_refresh(false));
        let (same, changed) = coordinator.finish_refresh(Ok(topology(
            &[("uid-a", "A"), ("uid-b", "B")],
            Some("uid-a"),
        )));
        assert!(!changed);
        assert_eq!(same.revision, 1);

        coordinator.request_refresh();
        assert!(coordinator.claim_refresh(false));
        let (stale, changed) = coordinator.finish_refresh(Err("private HAL detail".into()));
        assert!(changed);
        assert_eq!(stale.revision, 2);
        assert_eq!(stale.status, AudioInputInventoryStatus::Stale);
        assert_eq!(stale.devices, first.devices);
        assert_eq!(
            stale.error_code,
            Some(AudioInputInventoryErrorCode::EnumerationFailed)
        );
        assert!(!serde_json::to_string(&stale)
            .unwrap()
            .contains("private HAL detail"));
    }

    #[test]
    fn concurrent_readers_share_one_claimed_refresh() {
        let coordinator = Arc::new(AudioInputInventoryCoordinator::default());
        coordinator.request_refresh();
        let start = Arc::new(Barrier::new(3));
        let claims = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let coordinator = Arc::clone(&coordinator);
            let start = Arc::clone(&start);
            let claims = Arc::clone(&claims);
            workers.push(std::thread::spawn(move || {
                start.wait();
                if coordinator.claim_refresh(false) {
                    claims.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(20));
                    coordinator.finish_refresh(Ok(topology(&[("uid", "Mic")], Some("uid"))));
                } else {
                    coordinator.wait_for_initial_refresh(Duration::from_secs(1));
                }
            }));
        }
        start.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(claims.load(Ordering::SeqCst), 1);
        assert_eq!(
            coordinator.snapshot().status,
            AudioInputInventoryStatus::Available
        );
    }

    #[test]
    fn initial_readers_do_not_queue_a_second_refresh_behind_startup() {
        let coordinator = AudioInputInventoryCoordinator::default();
        assert!(coordinator.ensure_initial_refresh_requested());
        assert!(coordinator.claim_refresh(false));
        assert!(!coordinator.ensure_initial_refresh_requested());
        assert!(!coordinator.ensure_initial_refresh_requested());
        coordinator.finish_refresh(Ok(topology(&[("uid", "Mic")], Some("uid"))));
        assert!(!coordinator.claim_refresh(false));
    }

    #[test]
    fn initial_reader_can_reclaim_after_refresh_thread_spawn_failure() {
        let coordinator = AudioInputInventoryCoordinator::default();
        assert!(coordinator.ensure_initial_refresh_requested());
        assert!(coordinator.claim_refresh(false));
        coordinator.release_refresh_claim();
        assert!(!coordinator.ensure_initial_refresh_requested());
        assert!(coordinator.claim_refresh(false));
    }

    #[test]
    fn shutdown_prevents_new_refresh_claims_and_invalidations() {
        let coordinator = AudioInputInventoryCoordinator::default();
        coordinator.request_refresh();
        coordinator.begin_shutdown();
        assert!(!coordinator.claim_refresh(true));
        assert!(!coordinator.request_refresh());
        assert!(!coordinator.ensure_initial_refresh_requested());
        assert!(coordinator.invalidate().is_none());
    }

    #[test]
    fn deferred_refresh_remains_one_pending_claim_until_idle() {
        let coordinator = AudioInputInventoryCoordinator::default();
        coordinator.request_refresh();
        assert!(coordinator.claim_refresh(false));
        coordinator.defer_refresh_until_idle();
        assert!(!coordinator.claim_refresh(false));
        assert!(coordinator.claim_refresh(true));
        assert!(!coordinator.claim_refresh(false));
        coordinator.finish_refresh(Ok(topology(&[], None)));
        assert_eq!(
            coordinator.snapshot().status,
            AudioInputInventoryStatus::Available
        );
    }

    #[test]
    fn unknown_default_is_omitted_and_duplicate_stable_ids_fail_closed() {
        let unknown_default = normalize_inventory(topology(&[("uid-a", "A")], Some("uid-b")))
            .expect("an unknown default is safely omitted from a valid device inventory");
        assert_eq!(unknown_default.default_input_id, None);

        assert!(normalize_inventory(topology(
            &[("uid-a", "A"), ("uid-a", "renamed")],
            Some("uid-a")
        ))
        .is_err());
    }

    #[test]
    fn stale_or_unavailable_snapshots_never_feed_the_privacy_aggregate() {
        let coordinator = AudioInputInventoryCoordinator::default();
        let initial = coordinator.snapshot();
        assert_eq!(initial.status, AudioInputInventoryStatus::Unavailable);
        assert!(!privacy_aggregate_for_snapshot(&initial).enumeration_ok);

        coordinator.request_refresh();
        assert!(coordinator.claim_refresh(false));
        coordinator.finish_refresh(Ok(topology(
            &[("secret-uid", "Private Mic")],
            Some("secret-uid"),
        )));
        coordinator.request_refresh();
        assert!(coordinator.claim_refresh(false));
        let (stale, _) = coordinator.finish_refresh(Err("failure".into()));
        assert_eq!(stale.status, AudioInputInventoryStatus::Stale);
        assert_eq!(stale.devices.len(), 1);
        assert_eq!(
            privacy_aggregate_for_snapshot(&stale),
            AudioInputInventoryAggregate {
                default_input_available: false,
                input_device_count: 0,
                enumeration_ok: false,
            }
        );
    }

    #[test]
    fn topology_change_invalidates_immediately_and_epoch_forces_follow_up() {
        let coordinator = AudioInputInventoryCoordinator::default();
        coordinator.request_refresh();
        assert!(coordinator.claim_refresh(false));
        coordinator.finish_refresh(Ok(topology(&[("uid-a", "A")], Some("uid-a"))));

        coordinator.request_refresh();
        assert!(coordinator.claim_refresh(false));
        let (invalidated, changed) = coordinator.invalidate().unwrap();
        assert!(changed);
        assert_eq!(invalidated.status, AudioInputInventoryStatus::Stale);
        assert_eq!(
            invalidated.error_code,
            Some(AudioInputInventoryErrorCode::RefreshPending)
        );

        let (raced_result, _) =
            coordinator.finish_refresh(Ok(topology(&[("uid-a", "A")], Some("uid-a"))));
        assert_eq!(raced_result.status, AudioInputInventoryStatus::Stale);
        assert!(coordinator.claim_refresh(false));
        assert!(!coordinator.claim_refresh(false));
        let (settled, _) =
            coordinator.finish_refresh(Ok(topology(&[("uid-b", "B")], Some("uid-b"))));
        assert_eq!(settled.status, AudioInputInventoryStatus::Available);
        assert_eq!(settled.devices[0].id, "uid-b");
    }

    #[test]
    fn detached_reap_immediately_retracts_an_available_snapshot() {
        let coordinator = AudioInputInventoryCoordinator::default();
        coordinator.request_refresh();
        assert!(coordinator.claim_refresh(false));
        coordinator.finish_refresh(Ok(topology(&[("uid-a", "A")], Some("uid-a"))));

        let (retracted, changed) = detached_reap_invalidation(&coordinator).unwrap();
        assert!(changed);
        assert_eq!(retracted.status, AudioInputInventoryStatus::Stale);
        assert_eq!(
            retracted.error_code,
            Some(AudioInputInventoryErrorCode::RefreshPending)
        );
    }

    #[test]
    fn detached_reap_epoch_keeps_an_in_flight_result_stale() {
        let coordinator = AudioInputInventoryCoordinator::default();
        coordinator.request_refresh();
        assert!(coordinator.claim_refresh(false));
        coordinator.finish_refresh(Ok(topology(&[("uid-a", "A")], Some("uid-a"))));

        coordinator.request_refresh();
        assert!(coordinator.claim_refresh(false));
        detached_reap_invalidation(&coordinator).unwrap();
        let (raced_result, _) =
            coordinator.finish_refresh(Ok(topology(&[("uid-b", "B")], Some("uid-b"))));

        assert_eq!(raced_result.status, AudioInputInventoryStatus::Stale);
        assert_eq!(
            raced_result.error_code,
            Some(AudioInputInventoryErrorCode::RefreshPending)
        );
        assert!(coordinator.claim_refresh(false));
    }
}
