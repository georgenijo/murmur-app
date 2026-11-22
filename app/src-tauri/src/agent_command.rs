//! Agent commands: hand a spoken command to the local T3 Code server.
//!
//! STUB. The dispatcher is written on `feat/agent-command-core` and replaces
//! this file. Only the three items below are the agreed interface; the
//! recording side (`commands/recording.rs`) depends on nothing else.
//!
//! `spawn_dispatch` must return at once: the recording pipeline calls it while
//! it holds the dictation lock, so that a cancel either lands before the
//! dispatch or after it, never in between.

/// What one agent command is dispatched with. Resolved once at recording
/// start and carried in that recording's immutable context.
#[derive(Clone, Debug)]
pub struct AgentCommandConfig {
    pub t3_bin: Option<std::path::PathBuf>,
    pub dispatch_project: String,
    pub provider: String,
    pub model: String,
}

/// Where the dispatcher's replies are shown.
pub trait AgentNotifier: Send + Sync + 'static {
    fn notify(&self, title: &str, body: &str);
}

/// Start dispatching `transcript` in the background and return immediately.
pub fn spawn_dispatch(
    transcript: String,
    config: AgentCommandConfig,
    notifier: std::sync::Arc<dyn AgentNotifier>,
) {
    let _ = (transcript, config, notifier);
}
