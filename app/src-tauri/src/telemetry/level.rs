//! Log level control. The default is `info`; `debug` is opt-in and local.
//!
//! Two switches raise the level, neither needs a rebuild:
//!
//! - `MURMUR_LOG=debug` in the environment pins debug on for that launch.
//! - A persisted flag file, `debug-logging.on`, in the log directory. It is read
//!   at launch and watched while the app runs, so `touch`ing or removing it
//!   switches debug on or off within a few seconds. [`set_debug_logging`] writes
//!   it for a future command or settings control.
//!
//! Debug events are admitted only from Murmur's own targets, never from
//! dependencies, and they reach only the local `app.log`. They are never written
//! to `events.jsonl`, the in-memory ring buffer, the Events tab, or the capture
//! health observer, so the log shipper cannot see them.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use tracing_subscriber::{reload, EnvFilter, Registry};

pub(super) const ENV_VAR: &str = "MURMUR_LOG";
pub(super) const DEBUG_FLAG_FILE: &str = "debug-logging.on";
const FLAG_POLL_INTERVAL: Duration = Duration::from_secs(3);

/// Targets Murmur itself logs under: the stream names, the crate's module paths
/// and the `hunt!` target. Dependency crates log under their own module paths
/// and stay at `info` even when debug is on.
const OWN_TARGETS: &[&str] = &[
    "ui_lib",
    "pipeline",
    "system",
    "audio",
    "keyboard",
    "meeting",
    "transform",
    "overlay",
    "recording",
    "hunt",
];

static DEBUG_ON: AtomicBool = AtomicBool::new(false);
static RELOAD: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();

/// Whether debug logging is currently on.
pub(super) fn debug_enabled() -> bool {
    DEBUG_ON.load(Ordering::Relaxed)
}

/// The `EnvFilter` directives for a level.
pub(super) fn filter_directives(debug: bool) -> String {
    let mut directives = String::from("info");
    if debug {
        for target in OWN_TARGETS {
            directives.push(',');
            directives.push_str(target);
            directives.push_str("=debug");
        }
    }
    directives
}

fn build_filter(debug: bool) -> EnvFilter {
    EnvFilter::new(filter_directives(debug))
}

/// `Some(true)` / `Some(false)` when the environment value names a level.
pub(super) fn parse_env_level(value: Option<&str>) -> Option<bool> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "debug" | "trace" => Some(true),
        "info" | "warn" | "error" => Some(false),
        _ => None,
    }
}

/// The level a launch starts at: the environment wins over the flag file.
pub(super) fn initial_debug(env: Option<&str>, flag_present: bool) -> bool {
    parse_env_level(env).unwrap_or(flag_present)
}

pub(super) fn flag_path(log_dir: &Path) -> PathBuf {
    log_dir.join(DEBUG_FLAG_FILE)
}

/// The filter layer for the subscriber, and the level the launch starts at.
pub(super) fn initial_layer(log_dir: &Path) -> (reload::Layer<EnvFilter, Registry>, bool, bool) {
    let env = std::env::var(ENV_VAR).ok();
    let pinned = parse_env_level(env.as_deref()).is_some();
    let debug = initial_debug(env.as_deref(), flag_path(log_dir).exists());
    DEBUG_ON.store(debug, Ordering::Relaxed);
    let (layer, handle) = reload::Layer::new(build_filter(debug));
    let _ = RELOAD.set(handle);
    (layer, debug, pinned)
}

fn apply(debug: bool) {
    DEBUG_ON.store(debug, Ordering::Relaxed);
    if let Some(handle) = RELOAD.get() {
        let _ = handle.reload(build_filter(debug));
    }
}

/// Persist the choice and apply it immediately.
#[allow(dead_code)] // The hook for a future command or settings control.
pub(crate) fn set_debug_logging(log_dir: &Path, enabled: bool) -> std::io::Result<()> {
    let path = flag_path(log_dir);
    if enabled {
        std::fs::write(&path, b"")?;
    } else if let Err(error) = std::fs::remove_file(&path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(error);
        }
    }
    apply(enabled);
    Ok(())
}

/// Follow the flag file while the app runs. One `stat` every few seconds on a
/// background thread; skipped when the environment pins the level.
pub(super) fn spawn_flag_watcher(log_dir: PathBuf) {
    let _ = std::thread::Builder::new()
        .name("log-level-watch".into())
        .spawn(move || {
            let path = flag_path(&log_dir);
            loop {
                std::thread::sleep(FLAG_POLL_INTERVAL);
                let present = path.exists();
                if present != debug_enabled() {
                    apply(present);
                    tracing::info!(
                        target: "system",
                        event_code = "system.debug_logging_changed",
                        enabled = present,
                        "debug logging changed"
                    );
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_level_is_info_and_debug_is_opt_in() {
        assert_eq!(filter_directives(false), "info");
        let debug = filter_directives(true);
        assert!(debug.starts_with("info,"));
        for target in OWN_TARGETS {
            assert!(debug.contains(&format!("{target}=debug")), "{debug}");
        }
        // Dependencies keep the info default: a bare `debug` would admit them.
        assert!(!debug.split(',').any(|part| part == "debug"));
    }

    #[test]
    fn environment_wins_over_the_flag_file() {
        assert!(!initial_debug(None, false));
        assert!(initial_debug(None, true));
        assert!(initial_debug(Some("debug"), false));
        assert!(initial_debug(Some(" DEBUG "), false));
        assert!(!initial_debug(Some("info"), true));
        // An unrecognised value falls back to the flag file.
        assert!(initial_debug(Some("verbose"), true));
        assert!(!initial_debug(Some(""), false));
    }

    #[test]
    fn the_filter_admits_debug_only_from_own_targets_and_only_when_on() {
        use tracing_subscriber::prelude::*;
        use tracing_subscriber::Layer;

        #[derive(Clone, Default)]
        struct Seen(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
        impl<S: tracing::Subscriber> Layer<S> for Seen {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _: tracing_subscriber::layer::Context<'_, S>,
            ) {
                self.0.lock().unwrap().push(format!(
                    "{}:{}",
                    event.metadata().level(),
                    event.metadata().target()
                ));
            }
        }

        let run = |debug: bool| {
            let seen = Seen::default();
            let subscriber = tracing_subscriber::registry()
                .with(build_filter(debug))
                .with(seen.clone());
            tracing::subscriber::with_default(subscriber, || {
                tracing::debug!(target: "audio", "own debug");
                tracing::debug!(target: "hyper::client", "dependency debug");
                tracing::info!(target: "audio", "own info");
            });
            let events = seen.0.lock().unwrap().clone();
            events
        };
        assert_eq!(run(false), ["INFO:audio"]);
        assert_eq!(run(true), ["DEBUG:audio", "INFO:audio"]);
    }

    #[test]
    fn the_flag_file_round_trips() {
        let dir = std::env::temp_dir().join(format!("murmur-level-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!flag_path(&dir).exists());
        set_debug_logging(&dir, true).unwrap();
        assert!(flag_path(&dir).exists());
        assert!(debug_enabled());
        set_debug_logging(&dir, false).unwrap();
        assert!(!flag_path(&dir).exists());
        assert!(!debug_enabled());
        // Removing an absent flag is not an error.
        set_debug_logging(&dir, false).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
