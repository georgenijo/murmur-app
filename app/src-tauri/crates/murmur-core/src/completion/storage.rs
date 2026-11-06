use super::{Error, Result};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

/// Test failures occur at real publication boundaries, never by editing data.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Boundary {
    BeforeRename,
    AfterRename,
    DirectorySync,
}
#[derive(Default)]
pub struct Storage {
    pub root: PathBuf,
    pub(super) capture_failures: bool,
    pub(super) intents: std::collections::BTreeMap<String, serde_json::Value>,
    // Certainty of the most recent write, including failures before rename.
    pub renamed: bool,
    #[cfg(test)]
    pub fail: Option<(String, Boundary)>,
    #[cfg(test)]
    pub abort: Option<(String, Boundary)>,
    // Ordered faults; skip matching boundaries to target later writes to STATE.
    #[cfg(test)]
    pub faults: std::collections::VecDeque<(String, Boundary, usize)>,
    #[cfg(test)]
    pub pause: Option<(
        String,
        Boundary,
        std::sync::mpsc::Sender<()>,
        std::sync::mpsc::Receiver<()>,
    )>,
    #[cfg(test)]
    pub fail_read: Option<String>,
    #[cfg(test)]
    pub fail_sync: std::cell::Cell<bool>,
    #[cfg(test)]
    pub fail_prune_after_guard: std::cell::Cell<bool>,
    #[cfg(test)]
    pub trace: std::cell::RefCell<Vec<String>>,
}
impl Storage {
    pub fn pending_recovery_resolved(&self) -> Result<bool> {
        Ok(self
            .read::<serde_json::Value>("completion-recovery.json", |v| {
                v["schema"] == 1 && v["resolved"] == true
            })?
            .is_some())
    }
    pub fn remember_intent(&mut self, file: &str, value: &serde_json::Value) -> Result<()> {
        if self.intents.contains_key(file) {
            return Err(Error::TooManyFiles);
        }
        self.intents.insert(file.into(), value.clone());
        Ok(())
    }
    pub fn resolve_intent(&mut self, file: &str) {
        self.intents.remove(file);
    }
    pub fn has_intents(&self) -> bool {
        !self.intents.is_empty()
    }
    pub fn retry_intents(&mut self) -> Result<()> {
        for (file, value) in self.intents.clone() {
            // Never overwrite damaged/conflicting durable evidence with an
            // in-memory copy, even when the original admission was uncertain.
            if file == super::BATCH {
                let existed = self.root.join(super::BATCH).exists();
                let durable = self.read::<serde_json::Value>(super::BATCH, valid_batch)?;
                if let Some(durable) = durable {
                    if durable != value {
                        return Err(Error::Invalid);
                    }
                    self.sync_directory()?;
                } else if existed || self.quarantined(super::BATCH)? {
                    return Err(Error::Invalid);
                } else {
                    self.write(&file, &value)?;
                }
                self.recover_batch()?;
            } else {
                self.write(&file, &value)?;
            }
            self.resolve_intent(&file);
        }
        self.recover_batch()
    }
    /// The complete batch must be fully synced before either target can change.
    /// Retain one complete copy through uncertain admission and retirement.
    pub fn publish_batch(&mut self, snapshots: Vec<(String, serde_json::Value)>) -> Result<()> {
        if self.has_intents() || self.root.join(super::BATCH).exists() {
            return Err(Error::TooManyFiles);
        }
        let snapshots = serde_json::json!(snapshots);
        let payload = serde_json::json!({"schema":1,"digest":super::source_digest(&snapshots),"snapshots":snapshots});
        if !valid_batch(&payload) {
            return Err(Error::Invalid);
        }
        let bytes = serde_json::to_vec(&payload).map_err(|_| Error::Invalid)?;
        if bytes.len() > size_limit(super::BATCH) {
            return Err(Error::Invalid);
        }
        self.remember_intent(super::BATCH, &payload)?;
        // Do not capture failed target writes as independently retryable intents.
        // They must always be preceded by the durable complete batch.
        self.write_bytes(super::BATCH, &bytes)?;
        self.recover_batch()?;
        self.resolve_intent(super::BATCH);
        Ok(())
    }
    pub fn recover_batch(&mut self) -> Result<()> {
        let existed = self.root.join(super::BATCH).exists();
        let batch = self.read::<serde_json::Value>(super::BATCH, valid_batch)?;
        if let Some(batch) = batch {
            // Also resolves an unknown intent-rename outcome before publishing.
            self.sync_directory()?;
            let snapshots = batch["snapshots"].as_array().ok_or(Error::Invalid)?;
            for snapshot in snapshots {
                self.reject_future(snapshot[0].as_str().ok_or(Error::Invalid)?, "schema", 1)?;
            }
            for snapshot in snapshots {
                let bytes = serde_json::to_vec(&snapshot[1]).map_err(|_| Error::Invalid)?;
                self.write_bytes(snapshot[0].as_str().ok_or(Error::Invalid)?, &bytes)?;
            }
            // Every target is durable. Replaying after an uncertain unlink is
            // harmless: these are exact snapshots, never relative mutations.
            fs::remove_file(self.root.join(super::BATCH)).map_err(|_| Error::Io)?;
            self.boundary(super::BATCH, Boundary::DirectorySync)?;
            self.sync_directory()?;
        } else if existed || self.quarantined(super::BATCH)? {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn remove_temps(&self, name: &str) -> Result<()> {
        for path in self.artifact_paths(name, true)? {
            fs::remove_file(path).map_err(|_| Error::Io)?;
        }
        self.sync_directory()
    }
    pub fn remove_evidence(&self, name: &str) -> Result<()> {
        for path in self.artifact_paths(name, false)? {
            fs::remove_file(path).map_err(|_| Error::Io)?;
        }
        self.sync_directory()
    }

    pub fn publish_snapshot<T: Serialize>(&mut self, file: &str, value: &T) -> Result<()> {
        let pending = format!("{file}.pending");
        let payload = serde_json::json!({"schema":1,"value":value});
        self.remember_intent(&pending, &payload)?;
        self.write(&pending, &payload)?;
        self.resolve_intent(&pending);
        self.recover_snapshot(file, |v| v == &payload["value"])
    }
    pub fn recover_snapshot(
        &mut self,
        file: &str,
        validate: impl FnOnce(&serde_json::Value) -> bool,
    ) -> Result<()> {
        let pending = format!("{file}.pending");
        self.sync_directory()?;
        let existed = self.root.join(&pending).exists();
        if let Some(receipt) =
            self.read::<serde_json::Value>(&pending, |v| v["schema"] == 1 && validate(&v["value"]))?
        {
            self.write(file, &receipt["value"])?;
            fs::remove_file(self.root.join(&pending)).map_err(|_| Error::Io)?;
            self.sync_directory()?;
        } else if existed || self.quarantined(&pending)? {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            capture_failures: false,
            intents: Default::default(),
            renamed: false,
            #[cfg(test)]
            fail: None,
            #[cfg(test)]
            abort: None,
            #[cfg(test)]
            faults: Default::default(),
            #[cfg(test)]
            pause: None,
            #[cfg(test)]
            fail_read: None,
            #[cfg(test)]
            fail_sync: Default::default(),
            #[cfg(test)]
            fail_prune_after_guard: Default::default(),
            #[cfg(test)]
            trace: Default::default(),
        }
    }
    /// Future source schemas stay in place and reject every older launch.
    /// They are not corruption and must not turn into an absent/default file.
    pub fn reject_future(&self, name: &str, field: &str, current: u64) -> Result<()> {
        let path = self.root.join(name);
        let file = match File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(Error::Io),
        };
        let mut bytes = Vec::new();
        file.take(32 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Io)?;
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            if value[field].as_u64().is_some_and(|v| v > current) {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
    pub fn read<T: DeserializeOwned>(
        &self,
        name: &str,
        validate: impl FnOnce(&T) -> bool,
    ) -> Result<Option<T>> {
        #[cfg(test)]
        if self.fail_read.as_deref() == Some(name) {
            return Err(Error::Io);
        }
        let path = self.root.join(name);
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(Error::Io),
        };
        let mut bytes = Vec::new();
        let limit = size_limit(name);
        file.take(limit.saturating_add(1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Io)?;
        if bytes.len() <= limit {
            if let Ok(value) = serde_json::from_slice::<T>(&bytes) {
                if validate(&value) {
                    return Ok(Some(value));
                }
            }
        }
        self.quarantine(name)?;
        tracing::warn!(target:"completion", store=name, "unusable completion file quarantined");
        Ok(None)
    }
    pub fn quarantine(&self, name: &str) -> Result<()> {
        // Preserve the missing-source guard before any rename or deletion. This
        // guard is independent of the generation marker used to validate replay.
        self.preserve_source_history(name)?;
        let sequence_file = format!("{name}.quarantine_sequence");
        let previous: u64 = match fs::read(self.root.join(&sequence_file)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
            Err(_) => return Err(Error::Io),
        };
        let sequence = self
            .artifact_paths(name, false)?
            .iter()
            .map(|p| quarantine_sequence(p))
            .max()
            .unwrap_or(0)
            .max(previous)
            .checked_add(1)
            .ok_or(Error::Invalid)?;
        // Reserve durably first. Gaps after a crash are harmless; sequence reuse
        // and wall-clock/content timestamps must never order new quarantines.
        self.write_metadata(&sequence_file, sequence.to_string().as_bytes())?;
        fs::rename(
            self.root.join(name),
            self.root.join(format!(
                "{name}.quarantine-seq-{sequence:020}-{}",
                uuid::Uuid::new_v4()
            )),
        )
        .map_err(|_| Error::Io)?;
        self.sync_directory()?;
        self.prune_artifacts(name)
    }

    /// Cap each exact artifact family independently. Legacy destination aliases
    /// belong only to the destination, never its pending/identity/marker family.
    /// Callers hold the existing owner/write lock across pruning and recovery.
    pub fn prune_artifacts(&self, name: &str) -> Result<()> {
        self.preserve_source_history(name)?;
        #[cfg(test)]
        if self.fail_prune_after_guard.replace(false) {
            return Err(Error::Io);
        }
        let unresolved = name == super::BATCH
            || (name == super::PENDING && !self.root.join("completion-recovery.json").exists())
            || source_file(name).is_some_and(|file| {
                !self.root.join(file).exists() || self.root.join(format!("{file}.pending")).exists()
            });
        for temps in [false, true] {
            // All evidence for an unresolved source is pinned, including legacy
            // files whose generation cannot be proven. Temps carry no receipt.
            if !temps && unresolved {
                continue;
            }
            let mut files = self.artifact_paths(name, temps)?;
            files.sort_by_key(|p| (quarantine_sequence(p), p.clone()));
            let excess = files.len().saturating_sub(8);
            for path in files.into_iter().take(excess) {
                fs::remove_file(path).map_err(|_| Error::Io)?;
            }
            if excess > 0 {
                self.sync_directory()?;
            }
        }
        Ok(())
    }

    fn artifact_paths(&self, name: &str, temps: bool) -> Result<Vec<PathBuf>> {
        let kind = if temps { "tmp" } else { "quarantine" };
        let mut prefixes = vec![format!("{name}.{kind}-")];
        // The old publisher replaced .json with the artifact extension. Other
        // extensions are receipt families and must not alias the .json prefix.
        if let Some(stem) = name.strip_suffix(".json") {
            prefixes.push(format!("{stem}.{kind}-"));
        }
        if !temps {
            prefixes.push(format!("{name}.corrupt-"));
        }
        let mut paths = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(|_| Error::Io)? {
            let entry = entry.map_err(|_| Error::Io)?;
            let filename = entry.file_name();
            let filename = filename.to_string_lossy();
            if prefixes.iter().any(|p| filename.starts_with(p)) {
                paths.push(entry.path());
            }
        }
        Ok(paths)
    }

    pub fn quarantined(&self, name: &str) -> Result<bool> {
        Ok(!self.artifact_paths(name, false)?.is_empty())
    }

    /// A separate sticky guard cannot invalidate a pending receipt's base marker.
    /// Once observed, source history survives pruning and loss of other evidence.
    fn preserve_source_history(&self, name: &str) -> Result<()> {
        let Some(file) = source_file(name) else {
            return Ok(());
        };
        let guard = format!("{file}.required-history");
        if self.root.join(&guard).try_exists().map_err(|_| Error::Io)? {
            // A prior publication may have returned an unknown sync outcome.
            return self.sync_directory();
        }
        for owner in [
            file.to_string(),
            format!("{file}.pending"),
            format!("{file}.identity"),
            format!("{file}.required"),
        ] {
            if self.root.join(&owner).try_exists().map_err(|_| Error::Io)?
                || self.quarantined(&owner)?
            {
                return self.write_metadata(&guard, b"true");
            }
        }
        Ok(())
    }

    /// Nonrecursive durable metadata publication, with the same mode and full
    /// sync discipline as receipts. Never calls pruning before the guard exists.
    fn write_metadata(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let temp = self
            .root
            .join(format!("{name}.tmp-{}", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp).map_err(|_| Error::Io)?;
            file.write_all(bytes).map_err(|_| Error::Io)?;
            full_sync(&file)?;
            fs::rename(&temp, self.root.join(name)).map_err(|_| Error::Io)?;
            self.sync_directory()
        })();
        let _ = fs::remove_file(temp);
        result
    }
    pub fn sync_directory(&self) -> Result<()> {
        #[cfg(test)]
        {
            self.trace.borrow_mut().push("directory_sync".into());
            if self.fail_sync.replace(false) {
                return Err(Error::Io);
            }
        }
        File::open(&self.root)
            .and_then(|f| f.sync_all())
            .map_err(|_| Error::Io)
    }
    pub fn write<T: Serialize>(&mut self, name: &str, value: &T) -> Result<()> {
        let bytes = serde_json::to_vec(value).map_err(|_| Error::Invalid)?;
        let result = self.write_bytes(name, &bytes);
        if self.capture_failures && result.is_err() && name != super::STATE {
            self.intents
                .entry(name.to_owned())
                .or_insert(serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?);
        }
        result
    }
    pub fn write_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        self.renamed = false;
        if bytes.len() > size_limit(name) {
            return Err(Error::Invalid);
        }
        fs::create_dir_all(&self.root).map_err(|_| Error::Io)?;
        self.prune_artifacts(name)?;
        let path = self.root.join(name);
        let temp = self
            .root
            .join(format!("{name}.tmp-{}", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp).map_err(|_| Error::Io)?;
            file.write_all(bytes).map_err(|_| Error::Io)?;
            full_sync(&file)?;
            self.boundary(name, Boundary::BeforeRename)?;
            fs::rename(&temp, &path).map_err(|_| Error::Io)?;
            self.renamed = true;
            self.boundary(name, Boundary::AfterRename)?;
            self.boundary(name, Boundary::DirectorySync)?;
            self.sync_directory()
        })();
        let _ = fs::remove_file(temp);
        result
    }
    pub(super) fn boundary(&mut self, _name: &str, _at: Boundary) -> Result<()> {
        #[cfg(test)]
        self.trace.borrow_mut().push(format!("{_name}:{_at:?}"));
        #[cfg(test)]
        if self
            .abort
            .as_ref()
            .is_some_and(|(name, at)| name == _name && *at == _at)
        {
            self.abort = None;
            panic!("simulated process exit at {_name}:{_at:?}");
        }
        #[cfg(test)]
        if self
            .pause
            .as_ref()
            .is_some_and(|(name, at, _, _)| name == _name && *at == _at)
        {
            let (_, _, reached, resume) = self.pause.take().unwrap();
            reached.send(()).unwrap();
            resume.recv().unwrap();
        }
        #[cfg(test)]
        if self
            .fail
            .as_ref()
            .is_some_and(|(name, at)| name == _name && *at == _at)
        {
            self.fail = None;
            return Err(Error::Io);
        }
        #[cfg(test)]
        if let Some((name, at, skip)) = self.faults.front_mut() {
            if name == _name && *at == _at {
                if *skip == 0 {
                    self.faults.pop_front();
                    return Err(Error::Io);
                }
                *skip -= 1;
            }
        }
        Ok(())
    }
}
fn size_limit(name: &str) -> usize {
    match name {
        super::STATE => usize::MAX,
        // Two documents that each fit the existing destination limit plus envelope.
        super::BATCH => 64 * 1024 * 1024 + 4096,
        _ => 32 * 1024 * 1024,
    }
}

// Only the ordered History mutation targets are admissible. Validate the
// entire envelope before touching any target, including pre-import documents.
fn valid_batch(v: &serde_json::Value) -> bool {
    v.as_object().is_some_and(|o| o.len() == 3)
        && v["schema"] == 1
        && v["digest"] == super::source_digest(&v["snapshots"])
        && v["snapshots"].as_array().is_some_and(|snapshots| {
            (1..=2).contains(&snapshots.len())
                && snapshots.iter().enumerate().all(|(i, snapshot)| {
                    snapshot.as_array().is_some_and(|s| s.len() == 2)
                        && serde_json::to_vec(&snapshot[1])
                            .is_ok_and(|bytes| bytes.len() <= 32 * 1024 * 1024)
                        && if i == 0 {
                            snapshot[0] == super::HOLD
                                && serde_json::from_value::<super::Holding>(snapshot[1].clone())
                                    .is_ok_and(|h| super::valid_holding(&h))
                        } else {
                            snapshot[0] == "history.json"
                                && snapshot[1]["schema"] == 1
                                && super::valid_document(super::Destination::History, &snapshot[1])
                        }
                })
        })
}

// Only the two separately stored source destinations have sticky history guards.
fn source_file(name: &str) -> Option<&'static str> {
    ["settings.json", "theme-library.json"]
        .into_iter()
        .find(|file| name == *file || name.strip_prefix(file).is_some_and(|s| s.starts_with('.')))
}

fn quarantine_sequence(path: &Path) -> u64 {
    path.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.split_once(".quarantine-seq-"))
        .and_then(|(_, n)| n.split_once('-'))
        .and_then(|(n, _)| n.parse().ok())
        .unwrap_or(0) // Legacy evidence has no provable ordering; never use mtime.
}

fn full_sync(file: &File) -> Result<()> {
    file.sync_all().map_err(|_| Error::Io)?;
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: the file descriptor is live for the duration of fcntl.
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_FULLFSYNC) } == -1 {
            return Err(Error::Io);
        }
    }
    Ok(())
}

/// One owner per data root, including accidental second service instances.
/// The host retains this lock until the serialized task drains and exits.
pub struct RootLock(File);
impl RootLock {
    pub fn acquire(root: &Path) -> Result<Self> {
        fs::create_dir_all(root).map_err(|_| Error::Io)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("completion.lock"))
            .map_err(|_| Error::Io)?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: live descriptor; nonblocking exclusive advisory lock.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(Error::InUse);
            }
        }
        Ok(Self(file))
    }
}
impl Drop for RootLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            unsafe {
                libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
            }
        }
    }
}
