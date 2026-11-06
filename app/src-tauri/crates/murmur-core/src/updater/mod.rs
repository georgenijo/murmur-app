//! Host-neutral updater. The host supplies identity, executable, durable source
//! owner and presentation/relaunch adapters; it never supplies production trust.
mod feed;
mod install;
#[cfg(test)]
mod tests;
use crate::completion::{lifecycle::Operation, CompletionService};
pub use feed::{
    compare, forced, normalize_pending, parse_feed, pending_for, platform, translocated, verify,
    Offer, FEED, PUBLIC_KEY,
};
use feed::{version, MAX_DOWNLOAD, MAX_FEED};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
pub type Result<T> = std::result::Result<T, String>;
const INTERVAL: u64 = 6 * 60 * 60 * 1000;
const FRESHNESS: u64 = 5 * 60 * 1000;
const TRANSLOCATION: &str = "macOS opened Murmur from a read-only security location. Quit Murmur, then use Finder to move or reinstall it in Applications before reopening it and trying the update again.";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CompletedUpdate {
    pub version: String,
    pub notes: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "phase", rename_all = "kebab-case")]
pub enum Status {
    Idle,
    Checking,
    UpToDate,
    Available {
        version: String,
        notes: String,
        #[serde(rename = "isForced")]
        is_forced: bool,
    },
    Preparing {
        version: String,
    },
    Downloading {
        version: String,
        progress: Option<f64>,
    },
    Ready {
        version: String,
        #[serde(rename = "isForced")]
        is_forced: bool,
    },
    Restarting {
        version: String,
    },
    Error {
        stage: String,
        message: String,
        #[serde(rename = "isForced")]
        is_forced: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        recovery: Option<String>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub revision: u64,
    pub update_status: Status,
    pub completed_update: Option<CompletedUpdate>,
    pub is_update_dialog_open: bool,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            update_status: Status::Idle,
            completed_update: None,
            is_update_dialog_open: false,
        }
    }
}
#[derive(Clone, Deserialize, Debug)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Action {
    Configure { automatic: bool },
    Check { background: bool },
    Download,
    Install,
    Show,
    Dismiss,
    DismissCompleted,
}
struct Engine {
    offer: Option<Offer>,
    bytes: Option<Vec<u8>>,
    installed: bool,
    relaunch_requested: bool,
    startup: bool,
    notified: Option<String>,
}
pub struct Updater {
    engine: Mutex<Engine>,
    stopping: AtomicBool,
    automatic: AtomicBool,
    view: Mutex<Snapshot>,
    store: CompletionService,
    root: PathBuf,
    executable: PathBuf,
    identifier: String,
    current: String,
    feed: String,
    key: String,
    local_fixture: bool,
    changed: Arc<dyn Fn(Snapshot) + Send + Sync>,
    notify: Arc<dyn Fn(String) + Send + Sync>,
}
impl Updater {
    pub fn production(
        root: PathBuf,
        executable: PathBuf,
        identifier: String,
        current: String,
        store: CompletionService,
        changed: impl Fn(Snapshot) + Send + Sync + 'static,
        notify: impl Fn(String) + Send + Sync + 'static,
    ) -> Self {
        Self {
            engine: Mutex::new(Engine {
                offer: None,
                bytes: None,
                installed: false,
                relaunch_requested: false,
                startup: false,
                notified: None,
            }),
            stopping: AtomicBool::new(false),
            automatic: AtomicBool::new(false),
            view: Mutex::new(Snapshot::default()),
            root,
            executable,
            identifier,
            current,
            store,
            feed: FEED.into(),
            key: PUBLIC_KEY.into(),
            local_fixture: false,
            changed: Arc::new(changed),
            notify: Arc::new(notify),
        }
    }
    /// QA builds alone compile this seam; no environment trust override exists
    /// in production. The native adapter also checks the isolated identity.
    #[cfg(feature = "updater-test-support")]
    pub fn isolated_fixture(mut self, feed: String, key: String) -> Result<Self> {
        if self.identifier != "com.localdictation.shutdown-test" {
            return Err("Isolated updater identity required".into());
        }
        validate_url(&feed, true)?;
        self.feed = feed;
        self.key = key;
        self.local_fixture = true;
        Ok(self)
    }
    pub fn snapshot(&self) -> Snapshot {
        self.view.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
    fn update(&self, change: impl FnOnce(&mut Snapshot)) {
        let view = {
            let mut view = self.view.lock().unwrap_or_else(|p| p.into_inner());
            change(&mut view);
            view.revision += 1;
            view.clone()
        };
        (self.changed)(view);
    }
    fn state(&self, status: Status, open: bool) {
        self.update(|s| {
            s.update_status = status;
            if open {
                s.is_update_dialog_open = true;
            }
        });
    }
    fn source(&self) -> Result<crate::completion::lifecycle::Receipt> {
        let source = self
            .store
            .lifecycle(Operation::Status)
            .map_err(|e| e.to_string())?;
        if !source.v3 {
            return Err(
                "Updater storage migration is pending; reopen the main window and retry".into(),
            );
        }
        Ok(source)
    }
    fn save(&self, key: &str, value: Value) -> Result<()> {
        self.require_active()?;
        self.source()?;
        self.store
            .lifecycle(Operation::Save {
                key: key.into(),
                value,
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    fn client(&self) -> Result<reqwest::blocking::Client> {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(300))
            .connect_timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::custom({
                let local = self.local_fixture;
                move |attempt| {
                    if attempt.previous().len() >= 10
                        || validate_url(attempt.url().as_str(), local).is_err()
                    {
                        attempt.stop()
                    } else {
                        attempt.follow()
                    }
                }
            }))
            .user_agent("Murmur-Updater")
            .build()
            .map_err(|_| "Update HTTP client unavailable".into())
    }
    fn fetch(&self) -> Result<Option<Offer>> {
        validate_url(&self.feed, self.local_fixture)?;
        let client = self.client()?;
        let mut last = "Update check failed".to_string();
        for _ in 0..2 {
            let result = (|| {
                let response = client
                    .get(&self.feed)
                    .timeout(Duration::from_secs(30))
                    .send()
                    .map_err(|_| "Update feed unavailable")?;
                if response.status() == reqwest::StatusCode::NO_CONTENT {
                    return Ok(None);
                }
                let response = response
                    .error_for_status()
                    .map_err(|_| "Update feed unavailable")?;
                let mut bytes = Vec::new();
                response
                    .take(MAX_FEED + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| "Update feed interrupted")?;
                if bytes.len() as u64 > MAX_FEED {
                    return Err("Update feed too large".into());
                }
                parse_feed(&bytes, &self.current, &platform(), now())
            })();
            match result {
                Ok(offer) => return Ok(offer),
                Err(e) => last = e,
            }
        }
        Err(last)
    }
    fn refresh_completed(&self) -> Result<()> {
        let source = self.source()?;
        if self.root.join("updater-install.json").exists() {
            let bundle = install::bundle(&self.executable)?;
            install::recover(&self.root, &bundle, &self.current, &self.identifier)?;
        }
        let pending = source
            .values
            .get("pending-update-release-notes")
            .unwrap_or(&Value::Null);
        let completed = pending_for(pending, &self.current);
        if !pending.is_null() && completed.is_none() {
            self.save("pending-update-release-notes", Value::Null)?;
        }
        self.update(|s| s.completed_update = completed);
        Ok(())
    }
    fn check(&self, engine: &mut Engine, background: bool) -> Result<()> {
        if engine.bytes.is_some() || engine.installed || engine.relaunch_requested {
            if !background {
                self.update(|s| s.is_update_dialog_open = true);
            }
            return Ok(());
        }
        let source = self.source()?;
        let last = source
            .values
            .get("updater-last-check")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if background
            && (!self.automatic.load(Ordering::Acquire) || now().saturating_sub(last) < INTERVAL)
        {
            return Ok(());
        }
        if !background {
            self.state(Status::Checking, false);
        }
        let offer = self.fetch()?;
        self.require_active()?;
        self.save("updater-last-check", json!(now()))?;
        engine.offer = offer;
        if let Some(offer) = &engine.offer {
            self.state(
                Status::Available {
                    version: offer.version.clone(),
                    notes: offer.notes.clone(),
                    is_forced: offer.forced,
                },
                !background || offer.forced && self.automatic.load(Ordering::Acquire),
            );
            if background
                && self.automatic.load(Ordering::Acquire)
                && engine.notified.as_ref() != Some(&offer.version)
            {
                (self.notify)(offer.version.clone());
                engine.notified = Some(offer.version.clone());
            }
        } else {
            self.state(Status::UpToDate, false);
        }
        Ok(())
    }
    fn download(&self, engine: &mut Engine) -> Result<()> {
        if engine.bytes.is_some() || engine.installed {
            return Ok(());
        }
        let mut offer = engine.offer.clone().ok_or("Check for an update first")?;
        self.state(
            Status::Preparing {
                version: offer.version.clone(),
            },
            true,
        );
        // Verify environment before any large transfer, and recover old receipts.
        let bundle = install::bundle(&self.executable)?;
        install::recover(&self.root, &bundle, &self.current, &self.identifier)?;
        if now().saturating_sub(offer.discovered) > FRESHNESS {
            if let Ok(Some(fresh)) = self.fetch() {
                if compare(&fresh.version, &offer.version) == Some(std::cmp::Ordering::Greater) {
                    offer = fresh;
                }
            }
        }
        validate_url(&offer.url, self.local_fixture)?;
        self.save(
            "pending-update-release-notes",
            json!({"version":offer.version,"notes":offer.notes}),
        )?;
        engine.offer = Some(offer.clone());
        self.state(
            Status::Downloading {
                version: offer.version.clone(),
                progress: None,
            },
            true,
        );
        let bytes = install::download(&self.client()?, &offer.url, |progress| {
            self.state(
                Status::Downloading {
                    version: offer.version.clone(),
                    progress,
                },
                false,
            );
        })?;
        self.require_active()?;
        verify(&bytes, &offer.signature, &self.key)?;
        engine.bytes = Some(bytes);
        self.state(
            Status::Ready {
                version: offer.version,
                is_forced: offer.forced,
            },
            true,
        );
        Ok(())
    }
    fn install(&self, engine: &mut Engine) -> Result<bool> {
        if engine.relaunch_requested {
            return Ok(false);
        }
        let offer = engine.offer.as_ref().ok_or("No downloaded update")?;
        if !engine.installed {
            let bytes = engine.bytes.as_ref().ok_or("Download the update first")?;
            let bundle = install::bundle(&self.executable)?;
            self.state(
                Status::Restarting {
                    version: offer.version.clone(),
                },
                true,
            );
            // Do not recover here after an uncertain install: startup verifies the
            // receipt. Retrying cannot exchange the bundles a second time.
            install::install(
                &self.root,
                &bundle,
                offer,
                bytes,
                &self.key,
                &self.identifier,
                || !self.stopping.load(Ordering::Acquire),
            )?;
            engine.installed = true;
            engine.bytes = None;
        }
        self.canary_installed()?;
        engine.relaunch_requested = true;
        Ok(true)
    }
    /// True requests the host's compatible coordinated relaunch. The core has
    /// no process-control API. Commands serialize independently of view reads.
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
    }
    pub fn drained(&self) -> bool {
        self.engine.try_lock().is_ok()
    }
    fn require_active(&self) -> Result<()> {
        if self.stopping.load(Ordering::Acquire) {
            Err("Updater is shutting down".into())
        } else {
            Ok(())
        }
    }
    pub fn act(&self, action: Action) -> Result<bool> {
        self.require_active()?;
        if let Action::Configure { automatic } = action {
            self.automatic.store(automatic, Ordering::Release);
        }

        if matches!(
            action,
            Action::Show | Action::Dismiss | Action::DismissCompleted
        ) {
            match action {
                Action::Show => self.update(|s| s.is_update_dialog_open = true),
                Action::Dismiss => self.update(|s| {
                    if matches!(
                        s.update_status,
                        Status::Available {
                            is_forced: false,
                            ..
                        } | Status::Ready {
                            is_forced: false,
                            ..
                        } | Status::Error {
                            is_forced: false,
                            ..
                        }
                    ) {
                        s.is_update_dialog_open = false;
                    }
                }),
                Action::DismissCompleted => {
                    self.save("pending-update-release-notes", Value::Null)?;
                    self.update(|s| s.completed_update = None);
                }
                _ => {}
            }
            return Ok(false);
        }
        // Repeated background requests collapse while an owner is active.
        let mut engine = if matches!(action, Action::Check { background: true }) {
            match self.engine.try_lock() {
                Ok(v) => v,
                Err(std::sync::TryLockError::WouldBlock) => return Ok(false),
                Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            }
        } else {
            self.engine.lock().unwrap_or_else(|p| p.into_inner())
        };
        let stage = match action {
            Action::Download => "install",
            Action::Install => "restart",
            _ => "check",
        };
        let result = (|| {
            match action {
                Action::Configure { automatic } => {
                    self.automatic.store(automatic, Ordering::Release);
                    self.refresh_completed()?;
                    if automatic && !engine.startup {
                        engine.startup = true;
                        if canary_path().is_some() {
                            return self.run_canary(&mut engine);
                        }
                        self.check(&mut engine, true)?;
                    }
                }
                Action::Check { background } => self.check(&mut engine, background)?,
                Action::Download => self.download(&mut engine)?,
                Action::Install => return self.install(&mut engine),
                _ => {}
            }
            Ok(false)
        })();
        if let Err(error) = &result {
            self.state(
                Status::Error {
                    stage: stage.into(),
                    message: error.clone(),
                    is_forced: engine.offer.as_ref().is_some_and(|o| o.forced),
                    recovery: translocated(&self.executable).then(|| "reinstall".into()),
                },
                true,
            );
            self.canary_failed(error);
        }
        result
    }
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn validate_url(url: &str, local: bool) -> Result<()> {
    let url = url::Url::parse(url).map_err(|_| "Invalid update URL")?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"));
    if !url.username().is_empty()
        || url.password().is_some()
        || !(url.scheme() == "https" || local && loopback && url.scheme() == "http")
    {
        return Err("Update URL requires HTTPS".into());
    }
    if local && !loopback {
        return Err("Updater fixture requires a loopback URL".into());
    }
    Ok(())
}
pub fn canary_path() -> Option<PathBuf> {
    std::env::var_os("MURMUR_UPDATER_CANARY")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
}
pub fn canary_read() -> Option<Value> {
    canary_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|v| serde_json::from_slice(&v).ok())
}
pub fn canary_write(result: &Value) -> Result<()> {
    install::atomic_json(
        &canary_path().ok_or("Updater canary is not enabled")?,
        result,
    )
}
pub fn canary_dry_run() -> bool {
    std::env::var("MURMUR_UPDATER_CANARY_DRY_RUN")
        .is_ok_and(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "True"))
}
impl Updater {
    fn run_canary(&self, engine: &mut Engine) -> Result<bool> {
        let previous = canary_read();
        if !canary_dry_run()
            && previous.as_ref().is_some_and(|p| {
                p["status"] == "pending"
                    && p["offeredVersion"] == self.current
                    && [
                        "discover",
                        "policy",
                        "download",
                        "signatureVerify",
                        "install",
                    ]
                    .iter()
                    .all(|s| p["stages"][s] == "passed")
            })
        {
            let mut result = previous.unwrap();
            result["status"] = json!("passed");
            result["stages"]["relaunch"] = json!("passed");
            result["error"] = Value::Null;
            if let Ok(bundle) = install::bundle(&self.executable) {
                install::recover(&self.root, &bundle, &self.current, &self.identifier)?;
            }
            canary_write(&result)?;
            return Ok(false);
        }
        canary_write(
            &json!({"schemaVersion":1,"status":"pending","checkedVersion":self.current,"offeredVersion":null,"forced":false,"dryRun":canary_dry_run(),"stages":{"discover":"pending","policy":"pending","download":"pending","signatureVerify":"pending","install":"pending","relaunch":"pending"},"error":null}),
        )?;
        self.check(engine, false)?;
        let offer = engine
            .offer
            .as_ref()
            .ok_or("No update available for OTA canary")?;
        let mut result = canary_read().ok_or("Canary result unavailable")?;
        result["offeredVersion"] = json!(offer.version);
        result["forced"] = json!(offer.forced);
        result["stages"]["discover"] = json!("passed");
        result["stages"]["policy"] = json!(if offer.policy_verified {
            "passed"
        } else {
            "failed"
        });
        canary_write(&result)?;
        if !offer.policy_verified {
            return Err("Update policy could not be parsed from native manifest".into());
        }
        if canary_dry_run() {
            result["status"] = json!("dry-run");
            canary_write(&result)?;
            return Ok(false);
        }
        self.download(engine)?;
        result["stages"]["download"] = json!("passed");
        result["stages"]["signatureVerify"] = json!("passed");
        canary_write(&result)?;
        self.install(engine)
    }
    fn canary_installed(&self) -> Result<()> {
        if let Some(mut result) = canary_read().filter(|r| r["status"] == "pending") {
            result["stages"]["install"] = json!("passed");
            canary_write(&result)?;
        }
        Ok(())
    }
    fn canary_failed(&self, error: &str) {
        if let Some(mut result) = canary_read().filter(|r| r["status"] == "pending") {
            let stage = [
                "discover",
                "policy",
                "download",
                "signatureVerify",
                "install",
                "relaunch",
            ]
            .into_iter()
            .find(|s| result["stages"][s] == "pending");
            if let Some(stage) = stage {
                result["stages"][stage] = json!("failed");
            }
            result["status"] = json!("failed");
            result["error"] = json!(error);
            let _ = canary_write(&result);
        }
    }
}
