//! Same-volume staging and atomic bundle exchange. The running bundle is never
//! unlinked. A durable receipt distinguishes preparation from an adopted update.
use super::*;
use std::{
    ffi::CString,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Component,
};

const JOURNAL: &str = "updater-install.json";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u32,
    id: uuid::Uuid,
    bundle: PathBuf,
    version: String,
    identifier: String,
}
pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().ok_or("Missing storage directory")?;
    fs::create_dir_all(parent).map_err(io)?;
    let temporary = parent.join(format!(".updater-{}.part", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(io)?;
        file.write_all(&serde_json::to_vec(value).map_err(|_| "Invalid updater state")?)
            .map_err(io)?;
        file.sync_all().map_err(io)?;
        fs::rename(&temporary, path).map_err(io)?;
        sync(parent)
    })();
    let _ = fs::remove_file(temporary);
    result
}
fn io(_: std::io::Error) -> String {
    "Update filesystem operation failed".into()
}
fn sync(path: &Path) -> Result<()> {
    File::open(path).and_then(|f| f.sync_all()).map_err(io)
}
fn real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
}
pub fn bundle(executable: &Path) -> Result<PathBuf> {
    if translocated(executable) {
        return Err(TRANSLOCATION.into());
    }
    let macos = executable.parent().ok_or("Missing executable directory")?;
    let contents = macos.parent().ok_or("Missing app contents")?;
    let bundle = contents.parent().ok_or("Missing app bundle")?;
    if macos.file_name().is_none_or(|s| s != "MacOS")
        || contents.file_name().is_none_or(|s| s != "Contents")
        || bundle.extension().is_none_or(|s| s != "app")
        || executable.file_name().is_none_or(|s| s != "ui")
    {
        return Err("Updates require an installed macOS app bundle".into());
    }
    let canonical = fs::canonicalize(bundle).map_err(io)?;
    // Never overwrite a symlinked bundle or a path with symlinked ancestors.
    if canonical != bundle {
        return Err("Move Murmur to a regular writable app location before updating".into());
    }
    Ok(canonical)
}
// Keep aligned with the native bundle manifest and finalize_macos_bundle.py. These
// helpers are required even if an accidentally incomplete old bundle omitted one.
pub(super) const EXECUTABLES: &[&str] = &[
    "ui",
    "murmur-capture-agent",
    "murmur-capture-helper",
    "murmur-capture-worker",
];
fn validate(app: &Path, expected: &str, identifier: &str) -> Result<()> {
    let value = plist::Value::from_file(app.join("Contents/Info.plist"))
        .map_err(|_| "Invalid update bundle metadata")?;
    let dict = value
        .as_dictionary()
        .ok_or("Invalid update bundle metadata")?;
    let string = |key| dict.get(key).and_then(plist::Value::as_string);
    if string("CFBundleIdentifier") != Some(identifier)
        || string("CFBundleExecutable") != Some("ui")
        || string("CFBundleShortVersionString").and_then(version) != version(expected)
    {
        return Err("Update bundle identity or version mismatch".into());
    }
    if !real_dir(&app.join("Contents")) || !real_dir(&app.join("Contents/MacOS")) {
        return Err("Invalid update executable directory".into());
    }
    for name in EXECUTABLES {
        let executable = app.join("Contents/MacOS").join(name);
        let metadata = fs::symlink_metadata(&executable).map_err(io)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("Update executable is missing or invalid".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            use std::os::unix::fs::PermissionsExt;
            let path = CString::new(executable.as_os_str().as_bytes())
                .map_err(|_| "Invalid executable path")?;
            // Check mode as well as effective access: privileged test processes
            // must also reject a signed archive carrying a nonexecutable file.
            if metadata.permissions().mode() & 0o111 == 0
                || unsafe { libc::access(path.as_ptr(), libc::X_OK) } != 0
            {
                return Err("Update executable lacks execute permission".into());
            }
        }
        #[cfg(not(unix))]
        return Err("Updates require a Unix executable permission check".into());
    }
    Ok(())
}
fn staging(bundle: &Path, id: uuid::Uuid) -> Result<PathBuf> {
    Ok(bundle
        .parent()
        .ok_or("Missing app parent")?
        .join(format!(".murmur-update-{id}")))
}
fn safe_path(path: &Path) -> Result<PathBuf> {
    let mut clean = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(s) => clean.push(s),
            Component::CurDir => {}
            _ => return Err("Unsafe update archive path".into()),
        }
    }
    Ok(clean)
}
fn extract(bytes: &[u8], root: &Path) -> Result<PathBuf> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    let mut app_name = None;
    let mut expanded = 0u64;
    for (count, entry) in archive.entries().map_err(io)?.enumerate() {
        if count > 100_000 {
            return Err("Update archive has too many entries".into());
        }
        let mut entry = entry.map_err(io)?;
        let path = safe_path(&entry.path().map_err(io)?)?;
        if path.as_os_str().is_empty() {
            continue;
        }
        let first = path
            .components()
            .next()
            .ok_or("Empty archive path")?
            .as_os_str()
            .to_owned();
        if Path::new(&first).extension().is_none_or(|s| s != "app")
            || app_name.as_ref().is_some_and(|s| s != &first)
        {
            return Err("Update archive must contain one app bundle".into());
        }
        app_name = Some(first);
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir() || kind.is_symlink()) {
            return Err("Unsupported update archive entry".into());
        }
        expanded = expanded
            .checked_add(entry.size())
            .ok_or("Update archive too large")?;
        if expanded > 4 * MAX_DOWNLOAD {
            return Err("Update archive too large".into());
        }
        // No archive member may write through an earlier symlink.
        let mut ancestor = path.parent();
        while let Some(p) = ancestor {
            if fs::symlink_metadata(root.join(p)).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err("Update archive traverses a symlink".into());
            }
            ancestor = p.parent();
        }
        if kind.is_symlink() {
            let link = entry
                .link_name()
                .map_err(io)?
                .ok_or("Missing update symlink target")?;
            let mut depth = path.components().count() as i64 - 1;
            for c in link.components() {
                match c {
                    Component::Normal(_) => depth += 1,
                    Component::ParentDir => depth -= 1,
                    Component::CurDir => {}
                    _ => return Err("Unsafe update symlink".into()),
                }
                if depth < 1 {
                    return Err("Update symlink escapes bundle".into());
                }
            }
        }
        if !entry.unpack_in(root).map_err(io)? {
            return Err("Unsafe update archive".into());
        }
    }
    Ok(root.join(app_name.ok_or("Empty update archive")?))
}
fn sync_tree(path: &Path) -> Result<()> {
    for e in fs::read_dir(path).map_err(io)? {
        let e = e.map_err(io)?;
        let kind = e.file_type().map_err(io)?;
        if kind.is_dir() {
            sync_tree(&e.path())?;
        } else if kind.is_file() {
            sync(&e.path())?;
        }
    }
    sync(path)
}
/// Both names remain present before/after the syscall, including a process
/// crash. Only macOS ships; Linux exchange runs synthetic recovery tests.
fn exchange(a: &Path, b: &Path) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let a = CString::new(a.as_os_str().as_bytes()).map_err(|_| "Invalid app path")?;
    let b = CString::new(b.as_os_str().as_bytes()).map_err(|_| "Invalid stage path")?;
    #[cfg(target_os = "macos")]
    let result = unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            a.as_ptr(),
            libc::AT_FDCWD,
            b.as_ptr(),
            libc::RENAME_SWAP,
        )
    };
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            a.as_ptr(),
            libc::AT_FDCWD,
            b.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let result = -1;
    if result != 0 {
        return Err("Atomic app replacement failed; current app is unchanged".into());
    }
    Ok(())
}
/// Retain a durable copy of the signed bytes. Interrupted downloads are never
/// promoted to Ready and are deleted before a new attempt.
pub fn download(
    client: &reqwest::blocking::Client,
    url: &str,
    progress: impl Fn(Option<f64>),
) -> Result<Vec<u8>> {
    let mut response = client
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|_| "Update download failed")?;
    let total = response.content_length().filter(|n| *n > 0);
    if total.is_some_and(|n| n > MAX_DOWNLOAD) {
        return Err("Update download too large".into());
    }
    let mut bytes = Vec::new();
    let mut chunk = [0; 65536];
    loop {
        let count = response
            .read(&mut chunk)
            .map_err(|_| "Update download interrupted")?;
        if count == 0 {
            break;
        }
        if bytes.len() as u64 + count as u64 > MAX_DOWNLOAD {
            return Err("Update download too large".into());
        }
        bytes.extend_from_slice(&chunk[..count]);
        progress(total.map(|n| (bytes.len() as f64 / n as f64 * 100.).min(100.)));
    }
    if total.is_some_and(|n| n != bytes.len() as u64) {
        return Err("Update download interrupted".into());
    }
    Ok(bytes)
}
#[derive(Debug)]
pub(super) struct InstallError {
    pub invalid_payload: bool,
    pub message: String,
}
impl InstallError {
    fn invalid(message: String) -> Self {
        Self {
            invalid_payload: true,
            message,
        }
    }
}
impl From<&str> for InstallError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
impl From<String> for InstallError {
    fn from(message: String) -> Self {
        Self {
            invalid_payload: false,
            message,
        }
    }
}
pub fn install(
    root: &Path,
    target: &Path,
    offer: &Offer,
    bytes: &[u8],
    key: &str,
    identifier: &str,
    allowed: impl Fn() -> bool,
) -> std::result::Result<(), InstallError> {
    verify(bytes, &offer.signature, key).map_err(InstallError::invalid)?;
    if root.join(JOURNAL).exists() {
        return Err("An update transaction needs recovery before retrying".into());
    }
    let id = uuid::Uuid::new_v4();
    let stage = staging(target, id)?;
    fs::create_dir(&stage).map_err(io)?;
    let prepare = (|| {
        let unpacked = extract(bytes, &stage).map_err(InstallError::invalid)?;
        validate(&unpacked, &offer.version, identifier).map_err(InstallError::invalid)?;
        fs::rename(unpacked, stage.join("payload.app")).map_err(io)?;
        sync_tree(&stage)?;
        atomic_json(
            &root.join(JOURNAL),
            &Journal {
                schema: 1,
                id,
                bundle: target.into(),
                version: offer.version.clone(),
                identifier: identifier.into(),
            },
        )
        .map_err(InstallError::from)
    })();
    if let Err(error) = prepare {
        // An uncertain journal write must retain its stage for startup recovery.
        if !root.join(JOURNAL).exists() {
            let _ = fs::remove_dir_all(&stage);
        }
        return Err(error);
    }
    if !allowed() {
        return Err("Updater is shutting down".into());
    }
    exchange(target, &stage.join("payload.app"))?;
    // After exchange, an fsync failure is uncertain. Keep the journal and both
    // complete bundles; no destructive rollback can remove the running path.
    sync(target.parent().ok_or("Missing app parent")?)?;
    sync(&stage)?;
    validate(target, &offer.version, identifier).map_err(InstallError::invalid)
}
/// Resolve an uncertain post-exchange sync without exchanging twice. Keep the
/// old bundle until a new process validates its own running version.
pub fn adopted(root: &Path, target: &Path, offered: &str, identifier: &str) -> Result<bool> {
    let path = root.join(JOURNAL);
    let raw = match fs::read(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(io(e)),
    };
    if raw.len() > 65536 {
        return Err("Invalid update recovery receipt".into());
    }
    let journal: Journal =
        serde_json::from_slice(&raw).map_err(|_| "Invalid update recovery receipt")?;
    if journal.schema != 1
        || journal.bundle != target
        || journal.identifier != identifier
        || journal.version != offered
    {
        return Err("Update recovery identity mismatch".into());
    }
    if validate(target, offered, identifier).is_err() {
        return Ok(false);
    }
    sync(target.parent().ok_or("Missing app parent")?)?;
    sync(&staging(target, journal.id)?)?;
    sync(root)?;
    Ok(true)
}
/// Recover before the first check. A pre-exchange receipt is discarded only
/// while the original bundle is present; a post-exchange receipt is consumed
/// only by a process running the offered version. Future/corrupt evidence stays.
pub fn recover(root: &Path, target: &Path, current: &str, identifier: &str) -> Result<()> {
    let path = root.join(JOURNAL);
    let raw = match fs::read(&path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(io(e)),
    };
    if raw.len() > 65536 {
        return Err("Invalid update recovery receipt".into());
    }
    let journal: Journal =
        serde_json::from_slice(&raw).map_err(|_| "Invalid update recovery receipt")?;
    if journal.schema != 1
        || journal.bundle != target
        || journal.identifier != identifier
        || version(&journal.version).is_none()
    {
        return Err("Update recovery identity mismatch".into());
    }
    let stage = staging(target, journal.id)?;
    if !real_dir(target) || stage.exists() && !real_dir(&stage) {
        return Err("Invalid update recovery location".into());
    }
    if validate(target, &journal.version, identifier).is_ok()
        && version(current) != version(&journal.version)
    {
        return Err("Update installed; relaunch Murmur to complete recovery".into());
    }
    // Target must validate as either the running original or the new version.
    validate(target, current, identifier)?;
    if stage.exists() {
        fs::remove_dir_all(stage).map_err(io)?;
    }
    sync(target.parent().ok_or("Missing app parent")?)?;
    fs::remove_file(path).map_err(io)?;
    sync(root)
}
#[cfg(test)]
pub(super) fn simulate_exchange(a: &Path, b: &Path) -> Result<()> {
    exchange(a, b)
}
