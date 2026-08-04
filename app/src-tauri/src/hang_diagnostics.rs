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

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::Duration;

use crate::MutexExt;

static ARMED: AtomicBool = AtomicBool::new(false);
static CONFIG: OnceLock<Mutex<Option<(String, String)>>> = OnceLock::new();

const SECTION_CAP_BYTES: usize = 1_500_000;
const SAMPLE_SECONDS: &str = "1";

fn config_cell() -> &'static Mutex<Option<(String, String)>> {
    CONFIG.get_or_init(|| Mutex::new(None))
}

/// Called by the log shipper once per successful ingest: records where a
/// bundle would go and whether the receiver armed this install.
pub(crate) fn configure(install_id: &str, bundle_endpoint: &str, armed: bool) {
    *config_cell().lock_or_recover() =
        Some((install_id.to_string(), bundle_endpoint.to_string()));
    let was = ARMED.swap(armed, Ordering::Relaxed);
    if armed && !was {
        tracing::warn!(
            target: "system",
            "remote hang diagnostics armed by the log receiver for this install"
        );
    } else if !armed && was {
        tracing::info!(target: "system", "remote hang diagnostics disarmed");
    }
}

pub(crate) fn armed() -> bool {
    ARMED.load(Ordering::Relaxed)
}

/// Run a command, returning combined stdout+stderr truncated to the section
/// cap. On deadline the child is left to finish on its own (diagnostic path
/// only; the commands used here all terminate on their own).
fn run_capped(program: &str, args: &[&str], deadline: Duration) -> String {
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
            text.truncate(SECTION_CAP_BYTES);
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
    pub(crate) fn start(
        capture_id: u64,
        backend: &'static str,
        worker_pid: u32,
    ) -> Option<Self> {
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
    let mut bundle = String::new();
    let mut section = |title: &str, body: &str| {
        bundle.push_str(&format!("\n===== {title} =====\n"));
        bundle.push_str(body);
        bundle.push('\n');
    };
    section(
        "hang context",
        &format!(
            "app_version: {}\ncapture_id: {capture_id}\nbackend: {backend}\nerror_kind: {error_kind}\nepoch_ms: {}",
            env!("CARGO_PKG_VERSION"),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or_default()
        ),
    );
    section("worker native stack sample", worker_sample);
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
        "system HAL plug-ins",
        &run_capped("/bin/ls", &["-la", "/Library/Audio/Plug-Ins/HAL"], Duration::from_secs(5)),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_is_inert_unless_the_receiver_arms_this_install() {
        configure("test-install", "http://127.0.0.1:9/bundle", false);
        assert!(!armed());
        assert!(HangProbe::start(1, "auhal", std::process::id()).is_none());

        configure("test-install", "http://127.0.0.1:9/bundle", true);
        assert!(armed());
        configure("test-install", "http://127.0.0.1:9/bundle", false);
        assert!(!armed());
    }

    #[test]
    fn run_capped_returns_output_and_bounds_runaway_commands() {
        assert!(run_capped("/bin/echo", &["hello"], Duration::from_secs(5)).contains("hello"));
        assert_eq!(
            run_capped("/bin/sleep", &["5"], Duration::from_millis(100)),
            "<command deadline exceeded>"
        );
        assert!(run_capped("/nonexistent-binary", &[], Duration::from_secs(1))
            .starts_with("<spawn failed"));
    }
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
