//! SMAppService owns login launch after a durable, verified authority handoff.
//! No bootout/kill operation can terminate the app during migration.
use super::{
    autostart::{self, Checkpoint, Effect, Observation, Registration},
    os::OsServices,
};
use std::{
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::Command,
};
#[derive(Clone)]
pub struct Login {
    os: OsServices,
    home: PathBuf,
    data: PathBuf,
    identifier: String,
    intent: IntentFence,
    gate: std::sync::Arc<tokio::sync::Mutex<()>>,
}
#[derive(Clone, Default)]
struct IntentFence(std::sync::Arc<std::sync::atomic::AtomicU64>);
impl IntentFence {
    fn current(&self) -> u64 {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
    fn accept(&self, enabled: bool) -> Result<u64, String> {
        let next = |v: u64| (v & !1).checked_add(2).map(|v| v | u64::from(enabled));
        self.0
            .fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                next,
            )
            .ok()
            .and_then(next)
            .ok_or_else(|| "Login intent unavailable".into())
    }
    fn live(&self, ticket: u64) -> super::os::Live {
        let fence = self.clone();
        std::sync::Arc::new(move || fence.current() == ticket)
    }
    fn ensure(&self, ticket: u64) -> Result<(), String> {
        if self.current() == ticket {
            Ok(())
        } else {
            Err("Login choice changed".into())
        }
    }
}
struct Legacy {
    path: PathBuf,
    device: u64,
    inode: u64,
    bytes: Vec<u8>,
    enabled: bool,
}
fn label(identifier: &str) -> Option<&'static str> {
    match identifier {
        "com.localdictation" => Some("Murmur"),
        "com.localdictation.dev" => Some("Local Dictation Dev"),
        "com.localdictation.bench" => Some("Murmur Bench"),
        _ => None,
    }
}
unsafe extern "C" {
    fn getuid() -> u32;
}
impl Login {
    pub fn new(os: OsServices, home: PathBuf, data: PathBuf, identifier: String) -> Self {
        Self {
            os,
            home,
            data,
            identifier,
            intent: IntentFence::default(),
            gate: Default::default(),
        }
    }

    fn checkpoint(&self) -> Result<Checkpoint, String> {
        let path = self.data.join("native-login-migration.json");
        match std::fs::read(path) {
            Ok(bytes) if bytes.len() <= 1024 => {
                serde_json::from_slice(&bytes).map_err(|_| "Login migration unavailable".into())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Checkpoint::Unseen),
            _ => Err("Login migration unavailable".into()),
        }
    }
    fn persist(&self, value: Checkpoint) -> Result<(), String> {
        std::fs::create_dir_all(&self.data).map_err(|_| "Login migration unavailable")?;
        let destination = self.data.join("native-login-migration.json");
        let pending = self.data.join(format!(
            ".native-login-{}-{}.pending",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let result = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&pending)?;
            file.write_all(&serde_json::to_vec(&value).map_err(std::io::Error::other)?)?;
            file.sync_all()?;
            std::fs::rename(&pending, &destination)?;
            std::fs::File::open(&self.data)?.sync_all()
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&pending);
        }
        result.map_err(|_| "Login migration unavailable".into())
    }
    fn legacy(&self) -> Result<Option<Legacy>, String> {
        let Some(label) = label(&self.identifier) else {
            return Ok(None);
        };
        let path = self
            .home
            .join("Library/LaunchAgents")
            .join(format!("{label}.plist"));
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            _ => return Err("Legacy login agent unavailable".into()),
        };
        if !metadata.is_file()
            || metadata.uid() != unsafe { getuid() }
            || metadata.nlink() != 1
            || metadata.len() > 65536
        {
            return Err("Legacy login agent identity unavailable".into());
        }
        let bytes = std::fs::read(&path).map_err(|_| "Legacy login agent unavailable")?;
        let value: plist::Value =
            plist::from_bytes(&bytes).map_err(|_| "Legacy login agent unavailable")?;
        let dict = value
            .as_dictionary()
            .ok_or("Legacy login agent unavailable")?;
        let args = dict
            .get("ProgramArguments")
            .and_then(plist::Value::as_array)
            .ok_or("Legacy login agent unavailable")?;
        if dict.get("Label").and_then(plist::Value::as_string) != Some(label)
            || dict.get("RunAtLoad").and_then(plist::Value::as_boolean) != Some(true)
            || args.len() != 1
        {
            return Err("Legacy login agent identity unavailable".into());
        }
        let exe = args[0]
            .as_string()
            .map(Path::new)
            .filter(|p| p.is_absolute())
            .ok_or("Legacy login agent identity unavailable")?;
        let macos = exe
            .parent()
            .filter(|p| p.file_name().is_some_and(|n| n == "MacOS"))
            .ok_or("Legacy login agent identity unavailable")?;
        let contents = macos
            .parent()
            .filter(|p| p.file_name().is_some_and(|n| n == "Contents"))
            .ok_or("Legacy login agent identity unavailable")?;
        let info = contents.join("Info.plist");
        if std::fs::metadata(&info)
            .map_err(|_| "Legacy app identity unavailable")?
            .len()
            > 1024 * 1024
        {
            return Err("Legacy app identity unavailable".into());
        }
        let bundle: plist::Value =
            plist::from_file(info).map_err(|_| "Legacy app identity unavailable")?;
        let bundle = bundle
            .as_dictionary()
            .ok_or("Legacy app identity unavailable")?;
        if bundle
            .get("CFBundleIdentifier")
            .and_then(plist::Value::as_string)
            != Some(&self.identifier)
            || bundle
                .get("CFBundleExecutable")
                .and_then(plist::Value::as_string)
                != exe.file_name().and_then(|n| n.to_str())
        {
            return Err("Legacy app identity mismatch".into());
        }
        let enabled = !self.disabled(label)?;
        Ok(Some(Legacy {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
            bytes,
            enabled,
        }))
    }
    fn disabled(&self, label: &str) -> Result<bool, String> {
        let output = Command::new("/bin/launchctl")
            .args(["print-disabled", &format!("gui/{}", unsafe { getuid() })])
            .output()
            .map_err(|_| "Login authority unavailable")?;
        if !output.status.success() {
            return Err("Login authority unavailable".into());
        }
        let text = String::from_utf8(output.stdout).map_err(|_| "Login authority unavailable")?;
        Ok(text
            .lines()
            .any(|line| line.trim().trim_end_matches(';') == format!("\"{label}\" => true")))
    }
    fn retire(&self) -> Result<(), String> {
        let Some(legacy) = self.legacy()? else {
            return Ok(());
        };
        let label = label(&self.identifier).ok_or("Legacy login identity unavailable")?;
        let status = Command::new("/bin/launchctl")
            .args(["disable", &format!("gui/{}/{label}", unsafe { getuid() })])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|_| "Legacy login retirement unavailable")?;
        if !status.success() || !self.disabled(label)? {
            return Err("Legacy login retirement unavailable".into());
        }
        let now =
            std::fs::symlink_metadata(&legacy.path).map_err(|_| "Legacy login agent changed")?;
        if now.dev() != legacy.device
            || now.ino() != legacy.inode
            || now.nlink() != 1
            || !now.is_file()
            || std::fs::read(&legacy.path).map_err(|_| "Legacy login agent changed")?
                != legacy.bytes
        {
            return Err("Legacy login agent changed".into());
        }
        std::fs::remove_file(&legacy.path).map_err(|_| "Legacy login retirement unavailable")?;
        std::fs::File::open(
            legacy
                .path
                .parent()
                .ok_or("Legacy login retirement unavailable")?,
        )
        .and_then(|d| d.sync_all())
        .map_err(|_| "Legacy login retirement unavailable".into())
    }
    pub async fn migrate(&self, preference: Option<bool>) -> Result<Registration, String> {
        let ticket = self.intent.current();
        if ticket != 0 {
            return Err("Login migration superseded".into());
        }
        let _guard = self.gate.lock().await;
        self.intent.ensure(ticket)?;
        let mut checkpoint = self.checkpoint()?;
        for _ in 0..12 {
            let service = self.os.login_status().await?;
            self.intent.ensure(ticket)?;
            let legacy = self.legacy()?;
            let observation = Observation {
                preference,
                legacy_verified: legacy.is_some(),
                legacy_enabled: legacy.as_ref().is_some_and(|a| a.enabled),
                service,
            };
            match autostart::next(checkpoint, observation) {
                Effect::Persist(value) => {
                    self.persist(value)?;
                    checkpoint = value;
                }
                Effect::RetireLegacy => self.retire()?,
                Effect::Register => {
                    self.persist(Checkpoint::RegistrationAttempted)?;
                    checkpoint = Checkpoint::RegistrationAttempted;
                    self.os.login_set(true, self.intent.live(ticket)).await?;
                }
                Effect::Unregister => self.os.login_set(false, self.intent.live(ticket)).await?,
                Effect::Done => return Ok(service),
                Effect::Unavailable => return Ok(Registration::Unavailable),
            }
        }
        Err("Login migration unavailable".into())
    }
    /// Explicit user action; finish changing the OS authority before publishing
    /// its preference. This never treats a stale saved true as fresh consent.
    pub fn set(
        &self,
        enabled: bool,
    ) -> impl std::future::Future<Output = Result<Registration, String>> + Send + 'static {
        // Allocate synchronously, before scheduling. An explicit off revokes an
        // already queued main-thread registration even before this future runs.
        let ticket = self.intent.accept(enabled);
        let login = self.clone();
        async move {
            let ticket = ticket?;
            let _guard = login.gate.lock().await;
            login.intent.ensure(ticket)?;
            let before = login.os.login_status().await?;
            login.intent.ensure(ticket)?;
            if before == Registration::Unavailable {
                return Err("Login registration unavailable".into());
            }
            if enabled {
                login.persist(Checkpoint::MigrateEnabled)?;
                login.retire()?;
                login.persist(Checkpoint::RegistrationAttempted)?;
                if before == Registration::Disabled {
                    login.os.login_set(true, login.intent.live(ticket)).await?;
                }
            } else {
                login.retire()?;
                if before != Registration::Disabled {
                    login.os.login_set(false, login.intent.live(ticket)).await?;
                }
            }
            login.intent.ensure(ticket)?;
            let status = login.os.login_status().await?;
            login.intent.ensure(ticket)?;
            if status == Registration::Unavailable {
                return Err("Login registration unavailable".into());
            }
            login.persist(Checkpoint::Complete)?;
            Ok(status)
        }
    }
}
#[cfg(test)]
mod intent_tests {
    use super::*;
    #[test]
    fn explicit_off_revokes_a_registration_already_queued_on_main() {
        let fence = IntentFence::default();
        let automatic = fence.live(0);
        assert!(automatic());
        let off = fence.accept(false).unwrap();
        assert!(!automatic(), "queued automatic registration must refuse");
        let pending_off = fence.live(off);
        assert!(pending_off());
        let later_on = fence.accept(true).unwrap();
        assert!(
            !pending_off(),
            "stale off must not disable a newer explicit on"
        );
        assert!(fence.live(later_on)());
    }
    #[test]
    fn creating_off_future_revokes_migration_before_the_future_is_polled() {
        let login = Login::new(
            OsServices::default(),
            "/synthetic".into(),
            "/synthetic/app".into(),
            "synthetic.id".into(),
        );
        let migration = login.intent.live(0);
        let future = login.clone().set(false);
        assert!(!migration());
        drop(future); // No OS call, filesystem access or runtime is started.
    }
}
