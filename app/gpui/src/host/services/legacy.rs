//! A bounded, exact-child WebKit transport. The GPUI process owns every durable
//! migration operation; only its disposable worker loads WebKit. No profile
//! path or store identifier is accepted from CLI or a remote message.
use super::{backend::Backend, os};
use murmur_core::completion::lifecycle::{TRANSCRIPT_KEYS, UPDATER_V3_KEYS, V2_KEYS};
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString, c_char},
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
const MAX: usize = 32 * 1024 * 1024;
unsafe extern "C" {
    fn murmur_legacy_stage() -> i32;
    fn murmur_legacy_start(page: *const c_char, synthetic: bool) -> bool;
    fn murmur_legacy_pump();
    fn murmur_legacy_failed() -> bool;
    fn murmur_legacy_message() -> *mut c_char;
    fn murmur_legacy_remove(script: *const c_char);
    fn murmur_legacy_stop();
    fn murmur_legacy_retire() -> bool;
    fn murmur_services_free(value: *mut c_char);
}
fn approved(key: &str) -> bool {
    TRANSCRIPT_KEYS.contains(&key) || UPDATER_V3_KEYS.contains(&key)
}
fn line(reader: &mut impl BufRead) -> Result<Value, ()> {
    let mut bytes = Vec::new();
    reader
        .take(MAX as u64 + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| ())?;
    if bytes.len() > MAX || bytes.last() != Some(&b'\n') {
        return Err(());
    }
    serde_json::from_slice(&bytes).map_err(|_| ())
}
fn write(writer: &mut impl Write, value: &Value) -> Result<(), ()> {
    serde_json::to_writer(&mut *writer, value).map_err(|_| ())?;
    writer
        .write_all(b"\n")
        .and_then(|_| writer.flush())
        .map_err(|_| ())
}
fn page(seed: Option<Value>) -> CString {
    let keys: Vec<_> = TRANSCRIPT_KEYS
        .iter()
        .chain(V2_KEYS)
        .chain(UPDATER_V3_KEYS)
        .copied()
        .collect();
    let seed = seed
        .map(|seed| {
            format!(
                "for(const [k,v] of Object.entries({}))localStorage.setItem(k,v);",
                seed.to_string()
                    .replace('<', "\\u003c")
                    .replace('>', "\\u003e")
                    .replace('&', "\\u0026")
            )
        })
        .unwrap_or_default();
    CString::new(format!(r#"<!doctype html><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; connect-src 'none'"><script>
const post=x=>{{const s=JSON.stringify(x);if(s.length<={MAX})window.webkit.messageHandlers.migration.postMessage(s);else window.webkit.messageHandlers.migration.postMessage('{{"error":"oversized"}}');}};
try{{{seed}post(Object.fromEntries({keys}.map(k=>[k,localStorage.getItem(k)])));}}catch(_){{post({{error:'storage_unreadable'}});}}
function removeImported(keys){{try{{for(const k of keys)localStorage.removeItem(k);if(keys.some(k=>localStorage.getItem(k)!==null))throw 0;post({{removed:keys}});}}catch(_){{post({{error:'removal_failed'}});}}}}
</script>"#,keys=json!(keys))).expect("bounded static migration page")
}
/// Called by worker dispatch, before the GPUI platform is constructed.
pub fn worker() -> Option<i32> {
    let args: Vec<_> = std::env::args_os().collect();
    let synthetic =
        cfg!(feature = "native-host-qa") && args.iter().any(|a| a == "--native-legacy-fixture");
    if !synthetic && !args.iter().any(|a| a == "--native-legacy-import-worker") {
        return None;
    }
    if synthetic && os::identity() != "com.localdictation.gpui-host-qa110" {
        return Some(2);
    }
    Some(run_worker(synthetic).unwrap_or_else(|_| {
        if synthetic {
            unsafe { murmur_legacy_stage() }
        } else {
            2
        }
    }))
}
fn run_worker(synthetic: bool) -> Result<i32, ()> {
    // No WebKit object is created before the owning service authorizes this
    // exact child over its private pipe after checking the durable receipt.
    let authorization = line(&mut std::io::stdin().lock())?;
    if authorization["import"] != true {
        return Err(());
    }
    let seed = if synthetic {
        Some(authorization["seed"].clone())
    } else {
        None
    };
    let page = page(seed);
    if !unsafe { murmur_legacy_start(page.as_ptr(), synthetic) } {
        return Err(());
    }
    struct Release;
    impl Drop for Release {
        fn drop(&mut self) {
            unsafe { murmur_legacy_stop() };
        }
    }
    let _release = Release;
    let (send, receive) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        for _ in 0..2 {
            if send.send(line(&mut std::io::stdin().lock())).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut removed = false;
    let mut snapshot = false;
    while Instant::now() < deadline && !unsafe { murmur_legacy_failed() } {
        unsafe { murmur_legacy_pump() };
        let raw = unsafe { murmur_legacy_message() };
        if !raw.is_null() {
            let bytes = unsafe { CStr::from_ptr(raw) }.to_bytes();
            let message = if bytes.len() <= MAX {
                serde_json::from_slice::<Value>(bytes).ok()
            } else {
                None
            };
            unsafe { murmur_services_free(raw) };
            let value = message.ok_or(())?;
            if !snapshot {
                write(&mut std::io::stdout().lock(), &value)?;
                snapshot = true;
            } else if removed {
                write(&mut std::io::stdout().lock(), &value)?;
                // Wait for the durable prepare receipt before any broad purge.
                loop {
                    unsafe { murmur_legacy_pump() };
                    if let Ok(request) = receive.try_recv() {
                        if request?["retire"] != true || !unsafe { murmur_legacy_retire() } {
                            return Err(());
                        }
                        write(&mut std::io::stdout().lock(), &json!({"retired":true}))?;
                        return Ok(0);
                    }
                    if Instant::now() >= deadline {
                        return Err(());
                    }
                }
            } else {
                return Err(());
            }
        }
        if !removed && let Ok(message) = receive.try_recv() {
            let value = message?;
            let keys = value.as_array().ok_or(())?;
            if !snapshot || !keys.iter().all(|k| k.as_str().is_some_and(approved)) {
                return Err(());
            }
            let script = CString::new(format!("removeImported({value})")).map_err(|_| ())?;
            unsafe { murmur_legacy_remove(script.as_ptr()) };
            removed = true;
        }
    }
    Err(())
}
struct ExactChild(Child);
impl Drop for ExactChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
/// Every failure preserves unacknowledged keys and retries on an explicit Retry
/// or later launch. The worker's stderr/content never enter application logs.
pub async fn migrate(b: &Backend, synthetic: bool) -> Result<(), String> {
    let status = b.call("webkit_state_snapshot", json!({})).await?;
    // This check precedes store existence, process spawn, and framework loading.
    // A leftover directory after retirement must never resurrect the importer.
    if status["retired"] == true {
        return Ok(());
    }
    if !synthetic && !b.services.legacy_store_present() {
        b.services.prepare_legacy_retirement().await?;
        b.services.finish_legacy_retirement().await?;
        return Ok(());
    }
    let work = b.services.accept_work()?;
    let services = b.services.clone();
    let runtime = b.runtime.clone();
    let result = b
        .runtime
        .spawn_blocking(move || {
            let _work = work;
            let mut command = Command::new(std::env::current_exe().map_err(|_| ())?);
            command.arg(if synthetic {
                "--native-legacy-fixture"
            } else {
                "--native-legacy-import-worker"
            });
            let mut child = ExactChild(
                command
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|_| ())?,
            );
            let mut input = child.0.stdin.take().ok_or(())?;
            let output = child.0.stdout.take().ok_or(())?;
            write(
                &mut input,
                &json!({"import":true,"seed":if synthetic { fixture_seed() } else { Value::Null }}),
            )?;
            let (send, receive) = mpsc::sync_channel(2);
            let reader = std::thread::spawn(move || {
                let mut output = BufReader::new(output);
                for _ in 0..3 {
                    if send.send(line(&mut output)).is_err() {
                        break;
                    }
                }
            });
            let result = (|| {
                let source = receive
                    .recv_timeout(Duration::from_secs(35))
                    .map_err(|_| ())??;
                let (keys, committed) = runtime
                    .block_on(services.import_legacy(source))
                    .map_err(|_| ())?;
                write(&mut input, &json!(keys))?;
                let acknowledgement = receive
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|_| ())??;
                if acknowledgement["removed"] != json!(keys) {
                    return Err(());
                }
                runtime
                    .block_on(services.acknowledge_legacy(acknowledgement))
                    .map_err(|_| ())?;
                if !committed {
                    return Err(());
                }
                runtime
                    .block_on(services.prepare_legacy_retirement())
                    .map_err(|_| ())?;
                write(&mut input, &json!({"retire":true}))?;
                let receipt = receive
                    .recv_timeout(Duration::from_secs(25))
                    .map_err(|_| ())??;
                if receipt != json!({"retired":true}) {
                    return Err(());
                }
                // Worker exit is the no-live-WebKit-object barrier. A bounded
                // wait avoids hanging quit if the framework refuses to exit.
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    if let Some(status) = child.0.try_wait().map_err(|_| ())? {
                        if !status.success() {
                            return Err(());
                        }
                        break;
                    }
                    if Instant::now() >= deadline {
                        return Err(());
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                remove_retired_directory(&services.legacy_directory().map_err(|_| ())?)?;
                runtime
                    .block_on(services.finish_legacy_retirement())
                    .map_err(|_| ())?;
                Ok(())
            })();
            drop(input);
            drop(child);
            let _ = reader.join();
            result
        })
        .await
        .map_err(|_| "Legacy importer unavailable".to_owned())?;
    result.map_err(|_| "Legacy import preserved for retry".into())
}
/// Never follow a symlink supplied in place of the exclusive bundle directory.
fn remove_retired_directory(directory: &std::path::Path) -> Result<(), ()> {
    match std::fs::symlink_metadata(directory) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
            std::fs::remove_dir_all(directory).map_err(|_| ())?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Err(()),
    }
    std::fs::File::open(directory.parent().ok_or(())?)
        .and_then(|parent| parent.sync_all())
        .map_err(|_| ())
}
fn fixture_seed() -> Value {
    json!({"dictation-history":json!([{"schemaVersion":2,"id":"synthetic-legacy-110","text":"Synthetic migration fixture.","duration":1.0,"source":"recording","timestamp":1,"pinned":false}]).to_string(),"dictation-settings":json!({"settingsVersion":7,"model":"parakeet-tdt-0.6b-v3-coreml","microphone":"system_default","launchAtLogin":false,"language":"es","launchOnboardingComplete":false}).to_string()})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retirement_directory_cleanup_retries_without_following_links() {
        let root =
            std::env::temp_dir().join(format!("murmur-gpui-test-retire-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let target = root.join("exclusive");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("synthetic-cache"), b"synthetic").unwrap();
        remove_retired_directory(&target).unwrap();
        assert!(!target.exists());
        remove_retired_directory(&target).unwrap();
        let preserved = root.join("preserved");
        std::fs::create_dir(&preserved).unwrap();
        std::fs::write(preserved.join("synthetic"), b"keep").unwrap();
        std::os::unix::fs::symlink(&preserved, &target).unwrap();
        assert!(remove_retired_directory(&target).is_err());
        assert!(preserved.join("synthetic").exists());
        std::fs::remove_file(target).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn worker_fixture_commits_through_the_actual_core_protocol() {
        let root = std::env::temp_dir().join(format!(
            "murmur-gpui-test-worker-seed-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&root).unwrap();
        let service =
            murmur_core::completion::CompletionService::start(root.clone(), true).unwrap();
        let mut source = serde_json::Map::new();
        for key in TRANSCRIPT_KEYS.iter().chain(V2_KEYS).chain(UPDATER_V3_KEYS) {
            source.insert((*key).into(), Value::Null);
        }
        source.extend(fixture_seed().as_object().unwrap().clone());
        let removal =
            murmur_core::completion::transport::import(&service, Value::Object(source)).unwrap();
        assert!(removal.committed);
        murmur_core::completion::transport::acknowledge(&service, &json!({"removed":removal.keys}))
            .unwrap();
        assert_eq!(
            service
                .execute(murmur_core::completion::Command::Snapshot)
                .unwrap()
                .history
                .len(),
            1
        );
        assert!(
            service
                .lifecycle(murmur_core::completion::lifecycle::Operation::Status)
                .unwrap()
                .transcript_keys_removed
        );
        let _ = futures::executor::block_on(service.shutdown_receipt());
        drop(service);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn bounded_worker_protocol_never_accepts_an_arbitrary_removal_key() {
        assert!(approved("dictation-history"));
        assert!(!approved("dictation-settings"));
        assert!(!approved("unrelated"));
        assert!(line(&mut &b"{}\n"[..]).is_ok());
        assert!(line(&mut &b"{}"[..]).is_err());
        let page = page(Some(
            json!({"dictation-settings":"</script><script>synthetic</script>"}),
        ))
        .into_string()
        .unwrap();
        assert!(!page.contains("</script><script>"));
        assert!(page.contains("connect-src 'none'"));
    }
}
