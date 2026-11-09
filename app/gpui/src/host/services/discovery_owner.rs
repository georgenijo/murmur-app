//! The production owner of discovery state: the application sources, the local
//! statistics and settings, and the Tokio runtime.
use super::{
    backend::Backend,
    discovery::{Owner, State},
    projections as read,
};
use murmur_core::discovery as core;
use serde_json::Value;
use std::{future::Future, sync::atomic::Ordering};

impl Owner for Backend {
    fn shared(&self) -> &State {
        &self.discovery
    }
    fn booted(&self) -> bool {
        self.booted.load(Ordering::Acquire)
    }
    async fn state(&self) -> Result<Value, String> {
        Ok(self
            .sources()
            .await?
            .get(core::SOURCE)
            .cloned()
            .unwrap_or(Value::Null))
    }
    async fn write(&self, expected: Value, next: Value) -> Result<(), String> {
        self.source(core::SOURCE, expected, next).await
    }
    async fn evidence(&self) -> Result<core::Evidence, String> {
        let (_, completion) = self.services.history_snapshot().await?;
        let statistics =
            serde_json::to_value(&completion.statistics).map_err(|_| "Discovery unavailable")?;
        Ok(core::Evidence::read(
            &statistics,
            &read::settings(self).await?,
        ))
    }
    async fn newest(&self) -> Result<Value, String> {
        let (_, completion) = self.services.history_snapshot().await?;
        // History is oldest first.
        Ok(completion
            .history
            .last()
            .map_or(Value::Null, |entry| entry["id"].clone()))
    }
    fn launch(&self, work: impl Future<Output = ()> + Send + 'static) {
        self.spawn(work);
    }
}

/// Native acceptance of the discovery failure paths. Only a `native-host-qa`
/// build, the exact test bundle and an isolated `murmur-gpui-test-*` home honor
/// `--native-host-discovery-qa <scenario>`; any other launch ignores it.
/// It replaces that test profile's discovery state with a fresh one that has the
/// timing hint waiting, then arms the scenario's one-shot failure, which the
/// first Dismiss, Hint or checklist write meets. Scenarios: `clean`,
/// `read-fails-once`, `write-conflict-once`, `write-conflict-exhausts`.
#[cfg(feature = "native-host-qa")]
pub async fn qa(b: &Backend) {
    use super::{discovery, os};
    use std::path::PathBuf;
    let args: Vec<_> = std::env::args_os().collect();
    let Some(index) = args.iter().position(|a| a == "--native-host-discovery-qa") else {
        return;
    };
    let Some(scenario) = args.get(index + 1).and_then(|a| a.to_str()) else {
        return;
    };
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    if os::identity() != "com.localdictation.gpui-host-qa110"
        || std::env::var_os("MURMUR_NATIVE_TEST_HOME")
            .map(PathBuf::from)
            .as_ref()
            != Some(&home)
        || home
            .file_name()
            .is_none_or(|name| !name.to_string_lossy().starts_with("murmur-gpui-test-"))
    {
        return;
    }
    if discovery::seed(b).await.is_ok() {
        b.discovery.arm(scenario);
    }
}
