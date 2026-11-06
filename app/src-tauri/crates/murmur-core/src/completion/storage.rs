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
    // Certainty of the most recent write, including failures before rename.
    pub renamed: bool,
    #[cfg(test)]
    pub fail: Option<(String, Boundary)>,
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
    pub trace: std::cell::RefCell<Vec<String>>,
}
impl Storage {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            renamed: false,
            #[cfg(test)]
            fail: None,
            #[cfg(test)]
            faults: Default::default(),
            #[cfg(test)]
            pause: None,
            #[cfg(test)]
            fail_read: None,
            #[cfg(test)]
            fail_sync: Default::default(),
            #[cfg(test)]
            trace: Default::default(),
        }
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
        file.take(if name == super::STATE {
            u64::MAX
        } else {
            32 * 1024 * 1024 + 1
        })
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Io)?;
        if name == super::STATE || bytes.len() <= 32 * 1024 * 1024 {
            if let Ok(value) = serde_json::from_slice::<T>(&bytes) {
                if validate(&value) {
                    return Ok(Some(value));
                }
            }
        }
        fs::rename(
            &path,
            path.with_extension(format!("quarantine-{}", uuid::Uuid::new_v4())),
        )
        .map_err(|_| Error::Io)?;
        self.sync_directory()?;
        tracing::warn!(target:"completion", store=name, "unusable completion file quarantined");
        Ok(None)
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
        self.write_bytes(name, &bytes)
    }
    pub fn write_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        self.renamed = false;
        if name != super::STATE && bytes.len() > 32 * 1024 * 1024 {
            return Err(Error::Invalid);
        }
        fs::create_dir_all(&self.root).map_err(|_| Error::Io)?;
        let path = self.root.join(name);
        let temp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
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
    fn boundary(&mut self, _name: &str, _at: Boundary) -> Result<()> {
        #[cfg(test)]
        self.trace.borrow_mut().push(format!("{_name}:{_at:?}"));
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
