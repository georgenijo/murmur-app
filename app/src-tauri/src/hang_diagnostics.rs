//! Server-armed hang diagnostics.
//!
//! Dormant by default on every install. The log receiver's `/ingest` response
//! carries `{"diagnostics": bool}` per install UUID; only when the receiver
//! arms this install does any collection happen. Armed installs capture, at
//! capture-attempt timeout, a native stack sample of the hung capture worker
//! plus system audio context (coreaudiod unified log, audio/Bluetooth
//! topology, installed HAL plug-ins) and upload one bounded text bundle to
//! the receiver's `/bundle` endpoint. This data names devices and installed
//! software; it must only ever be armed for installs whose owner has agreed
//! to diagnostic collection. Disarming happens server-side (no release
//! needed) and takes effect on the next shipper tick.
//!
//! The bundle also carries the two `audio_graph_snapshot` sections: what the
//! Core Audio HAL says has live IO at collection time, and what Murmur itself
//! holds. Both are bounded and deadline-guarded there, not here.

use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::Duration;

use crate::MutexExt;

static ARMED: AtomicBool = AtomicBool::new(false);
static CONFIG: OnceLock<Mutex<Option<(String, String)>>> = OnceLock::new();
// Highest on-demand collection epoch already honored this process lifetime.
// The receiver's reply carries an integer epoch per armed install; a value
// greater than this triggers exactly one probe-bundle collection. The epoch
// selects only WHEN a collection runs — WHAT runs is this compiled-in probe
// list, never server-supplied text.
static LAST_COLLECT_EPOCH: AtomicU64 = AtomicU64::new(0);

const SECTION_CAP_BYTES: usize = 1_500_000;
const SAMPLE_SECONDS: &str = "1";

fn config_cell() -> &'static Mutex<Option<(String, String)>> {
    CONFIG.get_or_init(|| Mutex::new(None))
}

/// Called by the log shipper once per successful ingest: records where a
/// bundle would go, whether the receiver armed this install, and whether an
/// on-demand probe collection was requested.
pub(crate) fn configure(
    install_id: &str,
    bundle_endpoint: &str,
    armed: bool,
    collect_now_epoch: u64,
) {
    *config_cell().lock_or_recover() = Some((install_id.to_string(), bundle_endpoint.to_string()));
    let was = ARMED.swap(armed, Ordering::Relaxed);
    if armed && !was {
        tracing::warn!(
            target: "system",
            "remote hang diagnostics armed by the log receiver for this install"
        );
    } else if !armed && was {
        tracing::info!(target: "system", "remote hang diagnostics disarmed");
    }
    if take_collect_now(armed, collect_now_epoch) {
        tracing::warn!(
            target: "system",
            collect_epoch = collect_now_epoch,
            "on-demand diagnostic probe collection requested by the log receiver"
        );
        std::thread::spawn(move || {
            let bundle = collect_bundle(
                0,
                "none",
                "on_demand",
                "<on-demand collection - no hung worker to sample>",
            );
            ship_bundle(0, bundle);
        });
    }
}

/// True exactly once per new nonzero epoch while armed.
fn take_collect_now(armed: bool, epoch: u64) -> bool {
    if !armed || epoch == 0 {
        return false;
    }
    LAST_COLLECT_EPOCH.fetch_max(epoch, Ordering::Relaxed) < epoch
}

pub(crate) fn armed() -> bool {
    ARMED.load(Ordering::Relaxed)
}

/// Truncate at the nearest char boundary at or below `cap`: command output is
/// lossy-decoded UTF-8, and `String::truncate` panics mid-character.
pub(crate) fn truncate_at_boundary(text: &mut String, cap: usize) {
    if text.len() <= cap {
        return;
    }
    let mut cut = cap;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
}

/// Run a command, returning combined stdout+stderr truncated to the section
/// cap. On deadline the child is left to finish on its own (diagnostic path
/// only; the commands used here all terminate on their own).
pub(crate) fn run_capped(program: &str, args: &[&str], deadline: Duration) -> String {
    let program = program.to_string();
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(Command::new(&program).args(&args).output());
    });
    match receiver.recv_timeout(deadline) {
        Ok(Ok(output)) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.trim().is_empty() {
                text.push_str("\n--- stderr ---\n");
                text.push_str(&stderr);
            }
            truncate_at_boundary(&mut text, SECTION_CAP_BYTES);
            text
        }
        Ok(Err(error)) => format!("<spawn failed: {error}>"),
        Err(_) => "<command deadline exceeded>".to_string(),
    }
}

/// Started when a capture attempt is halfway through its budget without first
/// PCM; samples the still-running worker so the blocked native stack is
/// captured before the supervisor kills it.
pub(crate) struct HangProbe {
    sample_receiver: mpsc::Receiver<String>,
    capture_id: u64,
    backend: &'static str,
}

impl HangProbe {
    pub(crate) fn start(capture_id: u64, backend: &'static str, worker_pid: u32) -> Option<Self> {
        if !armed() {
            return None;
        }
        tracing::info!(
            target: "audio",
            capture_id,
            backend,
            "hang diagnostics: sampling the capture worker's native stack"
        );
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(run_capped(
                "/usr/bin/sample",
                &[&worker_pid.to_string(), SAMPLE_SECONDS],
                Duration::from_secs(10),
            ));
        });
        Some(Self {
            sample_receiver: receiver,
            capture_id,
            backend,
        })
    }

    /// Called after the hung worker's termination is confirmed. Collects the
    /// system context and uploads the bundle from a background thread; the
    /// capture sequence is never delayed by this.
    pub(crate) fn finish_and_ship(self, error_kind: &'static str) {
        let Self {
            sample_receiver,
            capture_id,
            backend,
        } = self;
        std::thread::spawn(move || {
            let sample = sample_receiver
                .recv_timeout(Duration::from_secs(8))
                .unwrap_or_else(|_| "<worker sample unavailable>".to_string());
            let bundle = collect_bundle(capture_id, backend, error_kind, &sample);
            ship_bundle(capture_id, bundle);
        });
    }
}

fn collect_bundle(
    capture_id: u64,
    backend: &'static str,
    error_kind: &'static str,
    worker_sample: &str,
) -> String {
    // Two views, because the field evidence says the blocker often clears the
    // moment the killed client's transport tears down:
    //   * "during hang" is the observation taken before the kill and cached by
    //     capture ID, so it sees the queue while it was actually blocked. It is
    //     claimed rather than re-queried — a fresh probe here would be too late
    //     and would race a second abandoned thread against the first.
    //   * "after kill" is taken now. The difference between the two is itself
    //     evidence about who was holding the engine queue.
    let during_hang = crate::audio_graph_snapshot::take_live_hang_report(capture_id);
    let after_kill = crate::audio_graph_snapshot::snapshot(
        crate::audio_graph_snapshot::Detail::Full,
    );
    let mut bundle = String::new();
    let mut section = |title: &str, body: &str| {
        bundle.push_str(&format!("\n===== {title} =====\n"));
        bundle.push_str(body);
        bundle.push('\n');
    };
    section(
        "hang context",
        &format!(
            "app_version: {}\ncapture_id: {capture_id}\nbackend: {backend}\nerror_kind: {error_kind}\nepoch_ms: {}\nlive_hang_graph_captured: {}\n-- audio graph counts (after kill) --\n{}",
            env!("CARGO_PKG_VERSION"),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or_default(),
            during_hang.is_some(),
            after_kill.counts.render_line(),
        ),
    );
    section("worker native stack sample", worker_sample);
    section(
        "system audio graph (during hang)",
        during_hang.as_deref().unwrap_or(
            "<no pre-kill observation for this capture attempt: the probe did not \
             finish in time, or the attempt was never observed>",
        ),
    );
    section("system audio graph (after kill)", &after_kill.report);
    section(
        "murmur internal audio owners",
        &crate::audio_graph_snapshot::internal_owners_report(),
    );
    section(
        "coreaudiod unified log (last 90s)",
        &run_capped(
            "/usr/bin/log",
            &[
                "show",
                "--style",
                "compact",
                "--last",
                "90s",
                "--predicate",
                "process == \"coreaudiod\"",
            ],
            Duration::from_secs(25),
        ),
    );
    section(
        "audio topology",
        &run_capped(
            "/usr/sbin/system_profiler",
            &["SPAudioDataType", "-detailLevel", "full"],
            Duration::from_secs(15),
        ),
    );
    section(
        "bluetooth state",
        &run_capped(
            "/usr/sbin/system_profiler",
            &["SPBluetoothDataType"],
            Duration::from_secs(15),
        ),
    );
    section(
        "process list",
        &run_capped(
            "/bin/ps",
            &["axo", "pid,ppid,user,lstart,comm"],
            Duration::from_secs(10),
        ),
    );
    section(
        "power assertions",
        &run_capped(
            "/usr/bin/pmset",
            &["-g", "assertions"],
            Duration::from_secs(10),
        ),
    );
    section(
        "system HAL plug-ins",
        &run_capped(
            "/bin/ls",
            &["-la", "/Library/Audio/Plug-Ins/HAL"],
            Duration::from_secs(5),
        ),
    );
    section(
        "user HAL plug-ins",
        &run_capped(
            "/bin/ls",
            &[
                "-la",
                &format!(
                    "{}/Library/Audio/Plug-Ins/HAL",
                    std::env::var("HOME").unwrap_or_default()
                ),
            ],
            Duration::from_secs(5),
        ),
    );
    bundle
}

fn ship_bundle(capture_id: u64, bundle: String) {
    let Some((install_id, endpoint)) = config_cell().lock_or_recover().clone() else {
        return;
    };
    let outcome = tauri::async_runtime::block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .ok()?;
        client
            .post(&endpoint)
            .header(
                "Authorization",
                format!("Bearer {}", crate::log_shipper::TOKEN),
            )
            .header("X-Install-Id", install_id)
            .header("Content-Type", "text/plain")
            .body(bundle)
            .send()
            .await
            .ok()
            .map(|response| response.status().is_success())
    });
    tracing::info!(
        target: "audio",
        capture_id,
        shipped = outcome.unwrap_or(false),
        "hang diagnostics: bundle upload finished"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_is_inert_unless_the_receiver_arms_this_install() {
        configure("test-install", "http://127.0.0.1:9/bundle", false, 0);
        assert!(!armed());
        assert!(HangProbe::start(1, "auhal", std::process::id()).is_none());

        configure("test-install", "http://127.0.0.1:9/bundle", true, 0);
        assert!(armed());
        configure("test-install", "http://127.0.0.1:9/bundle", false, 0);
        assert!(!armed());
    }

    #[test]
    fn collect_now_fires_once_per_new_epoch_and_never_unarmed() {
        assert!(!take_collect_now(false, 7)); // unarmed: never
        assert!(!take_collect_now(true, 0)); // zero epoch: never
        assert!(take_collect_now(true, 7)); // new epoch: once
        assert!(!take_collect_now(true, 7)); // same epoch: consumed
        assert!(!take_collect_now(true, 3)); // older epoch: never
        assert!(take_collect_now(true, 9)); // newer epoch: once again
        assert!(!take_collect_now(true, 9));
    }

    #[test]
    fn truncation_never_splits_a_multibyte_character() {
        let mut text = "ab\u{1F980}cd".to_string(); // crab is 4 bytes at index 2
        truncate_at_boundary(&mut text, 4);
        assert_eq!(text, "ab");
        let mut short = "ab".to_string();
        truncate_at_boundary(&mut short, 10);
        assert_eq!(short, "ab");
    }

    #[test]
    fn run_capped_returns_output_and_bounds_runaway_commands() {
        assert!(run_capped("/bin/echo", &["hello"], Duration::from_secs(5)).contains("hello"));
        assert_eq!(
            run_capped("/bin/sleep", &["5"], Duration::from_millis(100)),
            "<command deadline exceeded>"
        );
        assert!(
            run_capped("/nonexistent-binary", &[], Duration::from_secs(1))
                .starts_with("<spawn failed")
        );
    }
}
