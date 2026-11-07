//! Relaunch is a committed coordinator effect. A kernel pipe, kept open for the
//! parent's lifetime, prevents launching a replacement before parent exit and
//! avoids PID-reuse and new-version worker-protocol races.
use super::os;
use murmur_services::native_services::Services;
use std::{
    io::Read,
    os::{fd::OwnedFd, unix::net::UnixStream},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
const READY: &[u8] = b"murmur-ready-v1\n";
// The script is constant. The bundle is one positional argument, never shell
// source. EOF, rather than polling/signalling any PID, proves parent death.
const LAUNCHER: &str = "printf 'murmur-ready-v1\\n'\nexec 1>/dev/null\nIFS= read -r unexpected && exit 64\nexec /usr/bin/open -n -a \"$1\"\n";
static LAUNCHED: AtomicBool = AtomicBool::new(false);
static PARENT_LIFETIME: OnceLock<ChildStdin> = OnceLock::new();
fn arm(mut command: Command) -> Result<(Child, ChildStdin), &'static str> {
    let (mut ready, writer) = UnixStream::pair().map_err(|_| "Relaunch unavailable")?;
    ready
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| "Relaunch unavailable")?;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::from(OwnedFd::from(writer)))
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Relaunch unavailable")?;
    let parent = child.stdin.take();
    let mut response = vec![0; READY.len()];
    if ready.read_exact(&mut response).is_err() || response != READY || parent.is_none() {
        // This exact Child is ours; never signal a discovered/reused PID.
        let _ = child.kill();
        let _ = child.wait();
        return Err("Relaunch unavailable");
    }
    Ok((child, parent.expect("checked parent pipe")))
}
pub async fn relaunch(
    services: Services,
    runtime: tokio::runtime::Handle,
) -> Result<(), &'static str> {
    runtime
        .spawn_blocking(move || {
            if !services.restart_permitted() {
                return Err("Relaunch requires a drained restart permit");
            }
            let bundle = os::bundle_path();
            if bundle.extension().and_then(|e| e.to_str()) != Some("app") {
                return Err("Relaunch needs an installed app");
            }
            if LAUNCHED.swap(true, Ordering::SeqCst) {
                return Err("Relaunch already issued");
            }
            let mut command = Command::new("/bin/sh");
            command
                .args(["-c", LAUNCHER, "murmur-relaunch"])
                .arg(bundle);
            match arm(command) {
                Ok((mut child, parent)) => {
                    if PARENT_LIFETIME.set(parent).is_err() {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err("Relaunch already issued");
                    }
                    // Statics are never dropped. Only OS process exit closes this
                    // pipe; normal view/runtime cleanup cannot launch early.
                    Ok(())
                }
                Err(error) => {
                    LAUNCHED.store(false, Ordering::SeqCst);
                    Err(error)
                }
            }
        })
        .await
        .map_err(|_| "Relaunch unavailable")?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_waits_for_parent_pipe_eof_after_ready_handshake() {
        let root = std::env::temp_dir().join(format!("murmur-relaunch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let marker = root.join("synthetic-ready");
        let mut command = Command::new("/bin/sh");
        command.args(["-c","printf 'murmur-ready-v1\\n'\nIFS= read -r value && exit 64\nprintf synthetic > \"$1\"","synthetic-relaunch"]).arg(&marker);
        let (mut child, parent) = arm(command).unwrap();
        assert!(!marker.exists());
        assert!(child.try_wait().unwrap().is_none());
        drop(parent);
        assert!(child.wait().unwrap().success());
        assert_eq!(std::fs::read(&marker).unwrap(), b"synthetic");
        std::fs::remove_file(marker).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
