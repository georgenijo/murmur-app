//! Owned, resumable HTTP transfers. Partials are never installed artifacts.
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::Notify;

pub(crate) const CANCELLED: &str = "Model installation cancelled";
pub(crate) const RESERVE: u64 = 64 * 1024 * 1024;
pub(crate) const PARAKEET_STAGING: u64 = 2_000_000_000;
pub(crate) const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Default)]
pub(crate) struct Cancellation {
    cancelled: AtomicBool,
    notify: Notify,
}
impl Cancellation {
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.is_cancelled() {
            Err(CANCELLED.to_string())
        } else {
            Ok(())
        }
    }
    pub(crate) async fn cancelled(&self) {
        // Register before checking so cancellation cannot be lost at an await.
        let notified = self.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if !self.is_cancelled() {
            notified.await;
        }
    }
}

pub(crate) fn check_available(required: u64, available: u64) -> Result<(), String> {
    if available < required {
        return Err(format!(
            "Not enough disk space: need {:.0} MB, available {:.0} MB. Free space and retry.",
            (required as f64 / 1_000_000.0).ceil(),
            (available as f64 / 1_000_000.0).floor(),
        ));
    }
    Ok(())
}

/// Resolve the nearest existing ancestor: the install directory may not exist
/// yet, and FluidAudio's cache can live on a different volume from Whisper.
pub(crate) fn preflight(path: &Path, required: u64) -> Result<(), String> {
    let existing = path
        .ancestors()
        .find(|parent| parent.exists())
        .ok_or("Could not locate the model storage volume")?;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(existing.as_os_str().as_bytes())
            .map_err(|_| "Invalid model storage path")?;
        let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: a NUL-terminated path and valid output allocation are supplied.
        if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
            return Err(format!(
                "Could not check free disk space: {}",
                std::io::Error::last_os_error()
            ));
        }
        let stats = unsafe { stats.assume_init() };
        // statvfs field widths vary between macOS and other Unix targets.
        #[allow(clippy::unnecessary_cast)]
        let available = (stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64);
        check_available(required, available)
    }
    #[cfg(not(unix))]
    {
        let _ = (existing, required);
        Err("Disk space checks are unavailable on this platform".to_string())
    }
}

/// Conservative peak allocations, including archive + extracted/compiled data.
/// HTTP Content-Length refines the transfer requirement before opening a file.
pub(crate) fn install_bytes(model: &str) -> u64 {
    let bytes = match model {
        "tiny.en" => 80_000_000,
        "base.en" => 150_000_000,
        "small.en" => 500_000_000,
        "medium.en" => 1_550_000_000,
        "large-v3-turbo" => 1_630_000_000,
        "parakeet-tdt-0.6b-v2-fp16" => 1_200_000_000 + PARAKEET_STAGING,
        // FluidAudio owns download + compilation; budget temporary compiled copies.
        "parakeet-tdt-0.6b-v3-coreml" => 2_000_000_000,
        _ => 0,
    };
    bytes + RESERVE + 2_000_000 // VAD co-download
}

#[derive(Serialize, Deserialize)]
struct Resume {
    url: String,
    validator: String,
    total: u64,
}

pub(crate) fn resume_path(dest: &Path) -> std::path::PathBuf {
    let mut path = dest.as_os_str().to_os_string();
    path.push(".resume.json");
    path.into()
}

fn content_range(value: &str) -> Option<(u64, u64, u64)> {
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let (start, end, total) = (start.parse().ok()?, end.parse().ok()?, total.parse().ok()?);
    (start <= end && end < total).then_some((start, end, total))
}

/// Each body read owns a fresh idle interval; there is no cumulative deadline.
async fn next_chunk<S: futures_util::Stream + Unpin>(
    stream: &mut S,
    cancel: &Cancellation,
    idle: Duration,
) -> Result<Option<S::Item>, String> {
    use futures_util::StreamExt;
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(CANCELLED.to_string()),
        chunk = tokio::time::timeout(idle, stream.next()) => chunk
            .map_err(|_| "Download stalled: no data received. Retry to resume.".to_string()),
    }
}

pub(crate) async fn transfer(
    url: &str,
    dest: &Path,
    cancel: &Cancellation,
    idle: Duration,
    staging_bytes: u64,
    progress: impl FnMut(u64, u64),
) -> Result<u64, String> {
    transfer_with_space_check(url, dest, cancel, idle, staging_bytes, progress, preflight).await
}

async fn transfer_with_space_check(
    url: &str,
    dest: &Path,
    cancel: &Cancellation,
    idle: Duration,
    staging_bytes: u64,
    mut progress: impl FnMut(u64, u64),
    mut check_space: impl FnMut(&Path, u64) -> Result<(), String>,
) -> Result<u64, String> {
    use reqwest::header::{ACCEPT_ENCODING, CONTENT_RANGE, ETAG, IF_RANGE, RANGE};
    use tokio::io::AsyncWriteExt;
    cancel.check()?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(idle)
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;
    let metadata_path = resume_path(dest);
    let metadata = tokio::fs::read(&metadata_path)
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Resume>(&bytes).ok())
        .filter(|metadata| {
            metadata.url == url
                && metadata.validator.starts_with('"')
                && metadata.validator.ends_with('"')
        });
    let mut offset = tokio::fs::metadata(dest)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    if !metadata
        .as_ref()
        .is_some_and(|m| offset > 0 && offset < m.total)
    {
        offset = 0;
    }
    let mut request = client.get(url).header(ACCEPT_ENCODING, "identity");
    if offset > 0 {
        request = request
            .header(RANGE, format!("bytes={offset}-"))
            .header(IF_RANGE, &metadata.as_ref().unwrap().validator);
    }
    let send = |request: reqwest::RequestBuilder| async {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(CANCELLED.to_string()),
            response = request.send() => response.map_err(|e| format!("Download request failed (retry to resume): {e}")),
        }
    };
    let mut response = send(request).await?;
    let range = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(content_range);
    let validator_agrees = metadata.as_ref().is_some_and(|m| {
        response
            .headers()
            .get(ETAG)
            .is_none_or(|value| value.to_str().ok() == Some(m.validator.as_str()))
    });
    let valid_resume = offset > 0
        && validator_agrees
        && response.status() == reqwest::StatusCode::PARTIAL_CONTENT
        && range.is_some_and(|(start, end, total)| {
            start == offset
                && end + 1 == total
                && metadata.as_ref().is_some_and(|m| m.total == total)
                && response
                    .content_length()
                    .is_none_or(|len| len == end - start + 1)
        });
    // 200 means If-Range changed or Range isn't supported. Never append it.
    // Invalid ranges and 416 are retried with an ordinary full request.
    if offset > 0 && response.status() != reqwest::StatusCode::OK && !valid_resume {
        response = send(client.get(url).header(ACCEPT_ENCODING, "identity")).await?;
        offset = 0;
    } else if !valid_resume {
        offset = 0;
    }
    if !(response.status() == reqwest::StatusCode::OK || valid_resume && offset > 0) {
        return Err(format!(
            "Download failed with status: {}",
            response.status()
        ));
    }
    if response
        .headers()
        .get(reqwest::header::CONTENT_ENCODING)
        .is_some_and(|value| value != "identity")
    {
        return Err("Model server returned an unsupported content encoding".to_string());
    }
    let total = if offset > 0 {
        range.unwrap().2
    } else {
        response.content_length().unwrap_or(0)
    };
    cancel.check()?;
    if offset == 0 {
        // A full response cannot reuse the retained bytes. Release their disk
        // allocation before measuring free space, without creating a new file
        // before preflight. Keep old validator metadata until truncation has
        // succeeded; a crash now can only leave a zero-length partial.
        match tokio::fs::OpenOptions::new().write(true).open(dest).await {
            Ok(file) => file
                .set_len(0)
                .await
                .map_err(|e| format!("Failed to reset partial download: {e}"))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Failed to reset partial download: {error}")),
        }
        let _ = tokio::fs::remove_file(&metadata_path).await;
    }
    // Unknown lengths use the catalog preflight plus incremental write checks.
    check_space(
        dest,
        total
            .saturating_sub(offset)
            .saturating_add(staging_bytes)
            .saturating_add(RESERVE),
    )?;
    cancel.check()?;
    // Only a strong ETag proves that retained bytes belong to this artifact.
    // Last-Modified and weak validators restart cleanly instead of risking a
    // mixed file when the upstream artifact changes within the date precision.
    let validator = response
        .headers()
        .get(ETAG)
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.starts_with('"') && v.ends_with('"'))
        .unwrap_or("")
        .to_string();
    // A resumed response may omit its validator; If-Range already proved it.
    let validator = if offset > 0 {
        metadata.as_ref().unwrap().validator.clone()
    } else {
        validator
    };
    // Truncate BEFORE replacing metadata, so interruption cannot associate old
    // bytes with a new artifact validator.
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(offset == 0)
        .append(offset > 0)
        .open(dest)
        .await
        .map_err(|e| format!("Failed to open partial download: {e}"))?;
    let _ = tokio::fs::remove_file(&metadata_path).await;
    if total > 0 && !validator.is_empty() {
        tokio::fs::write(
            &metadata_path,
            serde_json::to_vec(&Resume {
                url: url.to_string(),
                validator,
                total,
            })
            .unwrap(),
        )
        .await
        .map_err(|e| format!("Failed to save download resume information: {e}"))?;
    }
    let mut received = offset;
    progress(received, total);
    let mut stream = response.bytes_stream();
    let result = async {
        loop {
            let chunk = next_chunk(&mut stream, cancel, idle).await?;
            let Some(chunk) = chunk else {
                break;
            };
            let chunk = chunk.map_err(|e| format!("Download interrupted. Retry to resume: {e}"))?;
            cancel.check()?;
            if total == 0 {
                check_space(dest, chunk.len() as u64 + staging_bytes + RESERVE)?;
            }
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("Failed to write model: {e}"))?;
            received += chunk.len() as u64;
            progress(received, total);
        }
        Ok::<(), String>(())
    }
    .await;
    // Drain tokio file writes even on cancellation before another owner resumes.
    file.flush()
        .await
        .map_err(|e| format!("Failed to flush model: {e}"))?;
    result?;
    cancel.check()?;
    if total > 0 && received != total {
        return Err(format!(
            "Download ended early: received {received} of {total} bytes. Retry to resume."
        ));
    }
    Ok(received)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{mpsc, Arc};
    use std::thread;

    // Exercise real reqwest streaming against a local HTTP peer, including
    // Range headers and truncated bodies, rather than mocking the transfer.
    type Reply = (String, Vec<(u64, Vec<u8>)>);

    fn server(replies: Vec<Reply>) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/model", listener.local_addr().unwrap());
        let (send, receive) = mpsc::channel();
        let worker = thread::spawn(move || {
            for (headers, chunks) in replies {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    if socket.read(&mut byte).unwrap_or(0) == 0 {
                        break;
                    }
                    request.push(byte[0]);
                }
                send.send(String::from_utf8(request).unwrap().to_ascii_lowercase())
                    .unwrap();
                if socket.write_all(headers.as_bytes()).is_err() {
                    continue;
                }
                for (delay, bytes) in chunks {
                    thread::sleep(Duration::from_millis(delay));
                    if socket.write_all(&bytes).is_err() {
                        break;
                    }
                }
            }
        });
        (url, receive, worker)
    }
    fn ok(length: u64) -> String {
        format!("HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n")
    }

    #[tokio::test(start_paused = true)]
    async fn slow_progress_can_outlive_many_idle_intervals() {
        let cancel = Cancellation::default();
        let mut stream = Box::pin(futures_util::stream::unfold(0, |n| async move {
            if n == 16 {
                return None;
            }
            tokio::time::sleep(Duration::from_secs(90)).await;
            Some((n, n + 1))
        }));
        let started = tokio::time::Instant::now();
        let mut received = Vec::new();
        while let Some(chunk) = next_chunk(&mut stream, &cancel, IDLE_TIMEOUT)
            .await
            .unwrap()
        {
            received.push(chunk);
        }
        // 24 virtual minutes of progress must survive the former 15-minute
        // total deadline. No sockets, OS scheduling, or real-time margins are
        // involved: this exercises the same idle read used by transfer().
        assert_eq!(received, (0..16).collect::<Vec<_>>());
        assert_eq!(started.elapsed(), Duration::from_secs(24 * 60));
    }

    #[tokio::test(start_paused = true)]
    async fn a_stalled_body_expires_after_one_idle_interval() {
        let mut stream = futures_util::stream::pending::<()>();
        let started = tokio::time::Instant::now();
        let error = next_chunk(&mut stream, &Cancellation::default(), IDLE_TIMEOUT)
            .await
            .unwrap_err();
        assert!(error.contains("Download stalled"));
        assert_eq!(started.elapsed(), IDLE_TIMEOUT);
    }

    #[tokio::test(start_paused = true)]
    async fn cancellation_interrupts_an_idle_read_before_its_timeout() {
        let cancel = Cancellation::default();
        let mut stream = futures_util::stream::pending::<()>();
        let started = tokio::time::Instant::now();
        let (result, ()) = tokio::join!(next_chunk(&mut stream, &cancel, IDLE_TIMEOUT), async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            cancel.cancel();
        },);
        assert_eq!(result, Err(CANCELLED.to_string()));
        assert_eq!(started.elapsed(), Duration::from_secs(10));
    }

    #[tokio::test]
    async fn http_body_is_written_and_reports_progress() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("model.tmp");
        let (url, requests, worker) = server(vec![(ok(8), vec![(0, b"xxxxxxxx".to_vec())])]);
        let mut updates = Vec::new();
        assert_eq!(
            transfer(
                &url,
                &dest,
                &Cancellation::default(),
                Duration::from_secs(30),
                0,
                |n, _| updates.push(n)
            )
            .await
            .unwrap(),
            8
        );
        assert_eq!(std::fs::read(dest).unwrap(), b"xxxxxxxx");
        assert_eq!(updates.last(), Some(&8));
        assert!(!requests.recv().unwrap().contains("range:"));
        worker.join().unwrap();
    }

    #[tokio::test]
    async fn stall_preserves_partial_and_retry_resumes_with_if_range() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("model.tmp");
        let (url, requests, worker) = server(vec![
            (ok(6), vec![(0, b"abc".to_vec()), (220, vec![])]),
            ("HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 3-5/6\r\nConnection: close\r\n\r\n".to_string(), vec![(0, b"def".to_vec())]),
        ]);
        assert!(transfer(
            &url,
            &dest,
            &Cancellation::default(),
            Duration::from_millis(100),
            0,
            |_, _| {}
        )
        .await
        .is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"abc");
        assert_eq!(
            transfer(
                &url,
                &dest,
                &Cancellation::default(),
                Duration::from_secs(1),
                0,
                |_, _| {}
            )
            .await
            .unwrap(),
            6
        );
        assert_eq!(std::fs::read(&dest).unwrap(), b"abcdef");
        assert!(!requests.recv().unwrap().contains("range:"));
        let request = requests.recv().unwrap();
        assert!(request.contains("range: bytes=3-"));
        assert!(request.contains("if-range: \"v1\""));
        worker.join().unwrap();
    }

    #[tokio::test]
    async fn cancel_interrupts_a_waiting_body_and_keeps_only_private_partial() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("model.tmp");
        let (url, _requests, worker) = server(vec![(
            ok(6),
            vec![(0, b"abc".to_vec()), (300, b"def".to_vec())],
        )]);
        let cancel = Arc::new(Cancellation::default());
        let trigger = cancel.clone();
        let mut cancellation = None;
        let result = transfer(&url, &dest, &cancel, Duration::from_secs(2), 0, |n, _| {
            if n == 3 {
                trigger.cancel();
                cancellation = Some(std::time::Instant::now());
            }
        })
        .await;
        assert_eq!(result, Err(CANCELLED.to_string()));
        assert!(cancellation.unwrap().elapsed() < Duration::from_millis(200));
        assert_eq!(std::fs::read(&dest).unwrap(), b"abc");
        assert!(resume_path(&dest).is_file());
        worker.join().unwrap();
    }

    #[tokio::test]
    async fn cancellation_before_start_creates_no_partial_or_request() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("model.tmp");
        let cancel = Cancellation::default();
        cancel.cancel();
        assert_eq!(
            transfer(
                "http://127.0.0.1:1",
                &dest,
                &cancel,
                Duration::from_secs(1),
                0,
                |_, _| {}
            )
            .await,
            Err(CANCELLED.to_string())
        );
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn ignored_changed_or_invalid_ranges_restart_without_appending() {
        for status in ["ignored", "invalid", "unsatisfiable"] {
            let root = tempfile::tempdir().unwrap();
            let dest = root.path().join("model.tmp");
            let full = (ok(6).replace("v1", "v2"), vec![(0, b"UVWXYZ".to_vec())]);
            let replies = match status {
                "ignored" => vec![full],
                "invalid" => vec![("HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 0-2/6\r\nConnection: close\r\n\r\n".to_string(), vec![(0, b"bad".to_vec())]), full],
                _ => vec![("HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(), vec![]), full],
            };
            let (url, requests, worker) = server(replies);
            std::fs::write(&dest, b"abc").unwrap();
            std::fs::write(
                resume_path(&dest),
                serde_json::to_vec(&Resume {
                    url: url.clone(),
                    validator: "\"v1\"".to_string(),
                    total: 6,
                })
                .unwrap(),
            )
            .unwrap();
            assert_eq!(
                transfer(
                    &url,
                    &dest,
                    &Cancellation::default(),
                    Duration::from_secs(1),
                    0,
                    |_, _| {}
                )
                .await
                .unwrap(),
                6
            );
            assert_eq!(std::fs::read(&dest).unwrap(), b"UVWXYZ", "{status}");
            assert!(requests.recv().unwrap().contains("range: bytes=3-"));
            if status != "ignored" {
                assert!(!requests.recv().unwrap().contains("range:"));
            }
            worker.join().unwrap();
        }
    }

    // Scale the review's 1 GB partial + 1 GB free example to bytes; retain the
    // real reserve and derive simulated free space from the real file length.
    // This proves truncation happens before preflight, not just that retries
    // eventually produce the right bytes on an unconstrained test volume.
    async fn constrained_full_restart(has_validator: bool) {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("model.tmp");
        std::fs::write(&dest, b"new").unwrap();
        let (url, requests, worker) = server(vec![(ok(8), vec![(0, b"newbytes".to_vec())])]);
        if has_validator {
            std::fs::write(
                resume_path(&dest),
                serde_json::to_vec(&Resume {
                    url: url.clone(),
                    validator: "\"v1\"".to_string(),
                    total: 8,
                })
                .unwrap(),
            )
            .unwrap();
        }
        let mut checked = false;
        transfer_with_space_check(
            &url,
            &dest,
            &Cancellation::default(),
            Duration::from_secs(30),
            0,
            |_, _| {},
            |path, required| {
                let retained = std::fs::metadata(path).unwrap().len();
                assert_eq!(
                    retained, 0,
                    "unusable partial must be truncated before checking space"
                );
                assert!(
                    !resume_path(path).exists(),
                    "old metadata must be cleared after truncation"
                );
                checked = true;
                check_available(required, RESERVE + 5 + (3 - retained))
            },
        )
        .await
        .unwrap();
        assert!(checked);
        assert_eq!(std::fs::read(&dest).unwrap(), b"newbytes");
        let metadata: Resume =
            serde_json::from_slice(&std::fs::read(resume_path(&dest)).unwrap()).unwrap();
        assert_eq!(metadata.validator, "\"v1\"");
        let request = requests.recv().unwrap();
        assert_eq!(request.contains("range: bytes=3-"), has_validator);
        worker.join().unwrap();
    }

    #[tokio::test]
    async fn ignored_range_reclaims_partial_space_before_full_restart_preflight() {
        constrained_full_restart(true).await;
    }

    #[tokio::test]
    async fn missing_validator_reclaims_partial_space_before_full_restart_preflight() {
        constrained_full_restart(false).await;
    }

    #[tokio::test]
    async fn partial_without_validator_is_restarted_safely() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("model.tmp");
        std::fs::write(&dest, b"unverified").unwrap();
        let (url, requests, worker) = server(vec![(ok(3), vec![(0, b"new".to_vec())])]);
        transfer(
            &url,
            &dest,
            &Cancellation::default(),
            Duration::from_secs(1),
            0,
            |_, _| {},
        )
        .await
        .unwrap();
        assert!(!requests.recv().unwrap().contains("range:"));
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        worker.join().unwrap();
    }

    #[tokio::test]
    async fn insufficient_peak_space_refuses_transfer_before_creating_partial() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("model.tmp");
        let (url, _requests, worker) = server(vec![(ok(3), vec![(0, b"abc".to_vec())])]);
        let error = transfer(
            &url,
            &dest,
            &Cancellation::default(),
            Duration::from_secs(1),
            u64::MAX,
            |_, _| panic!("preflight must precede progress"),
        )
        .await
        .unwrap_err();
        assert!(error.contains("Not enough disk space: need"));
        assert!(error.contains("available"));
        assert!(!dest.exists());
        assert!(!resume_path(&dest).exists());
        worker.join().unwrap();
    }

    #[test]
    fn disk_preflight_reports_needed_available_and_uses_existing_volume() {
        let error = check_available(1_200_000_000, 100_000_000).unwrap_err();
        assert!(error.contains("need 1200 MB, available 100 MB"));
        assert!(check_available(123, 123).is_ok());
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("FluidAudio/Models/new-cache");
        assert!(preflight(&missing, 0).is_ok());
        assert!(preflight(&missing, u64::MAX)
            .unwrap_err()
            .contains("Not enough disk space"));
        assert!(!missing.exists());
        assert!(
            install_bytes("parakeet-tdt-0.6b-v2-fp16")
                >= 1_200_000_000 + PARAKEET_STAGING + RESERVE
        );
        assert!(install_bytes("parakeet-tdt-0.6b-v3-coreml") > 3 * 470_000_000);
    }
}
