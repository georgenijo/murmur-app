//! Zero-config diagnostic log shipper.
//!
//! Tails the telemetry JSONL file (`events.jsonl`, already privacy-stripped by
//! `telemetry.rs`) and POSTs new lines as NDJSON batches to the central ingest
//! endpoint. Fire-and-forget: on any failure the persisted byte offset simply
//! does not advance, so the JSONL file itself is the retry queue. Installs are
//! identified by a random UUID generated on first run — never by hostname.
//!
//! Kill switch: set `MURMUR_LOG_SHIPPER=off` in the environment.
//! Endpoint override for testing: `MURMUR_LOG_ENDPOINT`.

use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const ENDPOINT: &str = "https://georgenijo.com/murmur/ingest";
const TOKEN: &str = "a1b4068693a1f3868bcf03c01ebcf1e9f000080b3e8bfcb0";
const TICK_SECS: u64 = 60;
const STARTUP_DELAY_SECS: u64 = 15;
/// Max bytes shipped per POST; a batch is always cut at a line boundary.
const MAX_BATCH_BYTES: usize = 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct ShipperState {
    install_id: String,
    offset: u64,
}

fn logs_dir() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("local-dictation").join("logs"))
}

fn state_path() -> Option<PathBuf> {
    let name = if cfg!(debug_assertions) {
        "shipper_state.dev.json"
    } else {
        "shipper_state.json"
    };
    Some(logs_dir()?.join(name))
}

fn jsonl_path() -> Option<PathBuf> {
    let name = if cfg!(debug_assertions) {
        "events.dev.jsonl"
    } else {
        "events.jsonl"
    };
    Some(logs_dir()?.join(name))
}

fn load_state(path: &Path) -> ShipperState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| ShipperState {
            install_id: uuid::Uuid::new_v4().to_string(),
            offset: 0,
        })
}

fn save_state(path: &Path, state: &ShipperState) {
    if let Ok(json) = serde_json::to_string(state) {
        let _ = std::fs::write(path, json);
    }
}

/// One batch of log bytes to ship, cut at a line boundary.
struct Batch {
    data: Vec<u8>,
    /// Offset into the *current* file after this batch is acknowledged.
    next_offset: u64,
}

/// Read the next batch from `log` at `offset`, falling back to the rotated
/// file when the current file is shorter than the offset (telemetry.rs renames
/// `events.jsonl` → `events.jsonl.1` at 5 MB and starts fresh).
fn next_batch(log: &Path, rotated: &Path, offset: u64) -> Option<Batch> {
    let log_len = std::fs::metadata(log).map(|m| m.len()).unwrap_or(0);

    if offset > log_len {
        // Rotation happened since our last ack: our offset points into the
        // renamed file. Drain its tail in one go, then restart at 0.
        let tail = read_range(rotated, offset, u64::MAX).unwrap_or_default();
        if tail.is_empty() {
            // Rotated file gone or shorter than the offset (e.g. double
            // rotation); nothing recoverable — restart at the new file.
            return Some(Batch { data: Vec::new(), next_offset: 0 });
        }
        return Some(Batch { data: tail, next_offset: 0 });
    }

    let chunk = read_range(log, offset, MAX_BATCH_BYTES as u64)?;
    if chunk.is_empty() {
        return None;
    }
    match chunk.iter().rposition(|&b| b == b'\n') {
        Some(pos) => Some(Batch {
            next_offset: offset + pos as u64 + 1,
            data: chunk[..=pos].to_vec(),
        }),
        // No newline in a full-size chunk means a pathological >1 MB line:
        // skip it rather than stall forever. A partial short chunk is just a
        // line still being written — wait for the next tick.
        None if chunk.len() >= MAX_BATCH_BYTES => Some(Batch {
            next_offset: offset + chunk.len() as u64,
            data: Vec::new(),
        }),
        None => None,
    }
}

fn read_range(path: &Path, offset: u64, max: u64) -> Option<Vec<u8>> {
    let mut file = std::fs::File::open(path).ok()?;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = Vec::new();
    file.take(max).read_to_end(&mut buf).ok()?;
    Some(buf)
}

async fn ship(client: &reqwest::Client, endpoint: &str, install_id: &str, data: Vec<u8>) -> bool {
    let result = client
        .post(endpoint)
        .header("Authorization", format!("Bearer {}", TOKEN))
        .header("X-Install-Id", install_id)
        .header("X-App-Version", env!("CARGO_PKG_VERSION"))
        .header("X-Dev", if cfg!(debug_assertions) { "1" } else { "0" })
        .header("Content-Type", "application/x-ndjson")
        .body(data)
        .send()
        .await;
    matches!(result, Ok(resp) if resp.status().is_success())
}

async fn tick(client: &reqwest::Client, endpoint: &str) {
    let (Some(state_file), Some(log)) = (state_path(), jsonl_path()) else {
        return;
    };
    let rotated = log.with_extension("jsonl.1");
    let mut state = load_state(&state_file);

    // Bounded loop: drain at most a handful of batches per tick so a huge
    // backlog trickles out instead of hammering the endpoint.
    for _ in 0..8 {
        let Some(batch) = next_batch(&log, &rotated, state.offset) else {
            break;
        };
        if !batch.data.is_empty()
            && !ship(client, endpoint, &state.install_id, batch.data).await
        {
            break; // endpoint unreachable — offset stays put, retry next tick
        }
        state.offset = batch.next_offset;
        save_state(&state_file, &state);
    }
}

pub fn start() {
    if std::env::var("MURMUR_LOG_SHIPPER").is_ok_and(|v| v == "off") {
        tracing::info!(target: "system", "log shipper disabled via MURMUR_LOG_SHIPPER=off");
        return;
    }
    let endpoint =
        std::env::var("MURMUR_LOG_ENDPOINT").unwrap_or_else(|_| ENDPOINT.to_string());

    tauri::async_runtime::spawn(async move {
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
        {
            Ok(c) => c,
            Err(_) => return,
        };
        tokio::time::sleep(std::time::Duration::from_secs(STARTUP_DELAY_SECS)).await;
        loop {
            tick(&client, &endpoint).await;
            tokio::time::sleep(std::time::Duration::from_secs(TICK_SECS)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("murmur_shipper_test_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn batch_cuts_at_line_boundary() {
        let dir = tmp_dir("cut");
        let log = dir.join("events.jsonl");
        std::fs::write(&log, b"{\"a\":1}\n{\"b\":2}\n{\"partial").unwrap();
        let batch = next_batch(&log, &dir.join("events.jsonl.1"), 0).unwrap();
        assert_eq!(batch.data, b"{\"a\":1}\n{\"b\":2}\n");
        assert_eq!(batch.next_offset, 16);
        // The partial trailing line is not shippable yet.
        assert!(next_batch(&log, &dir.join("events.jsonl.1"), 16).is_none());
    }

    #[test]
    fn batch_resumes_from_offset() {
        let dir = tmp_dir("resume");
        let log = dir.join("events.jsonl");
        std::fs::write(&log, b"{\"a\":1}\n{\"b\":2}\n").unwrap();
        let batch = next_batch(&log, &dir.join("events.jsonl.1"), 8).unwrap();
        assert_eq!(batch.data, b"{\"b\":2}\n");
        assert_eq!(batch.next_offset, 16);
    }

    #[test]
    fn rotation_drains_old_file_then_restarts() {
        let dir = tmp_dir("rotate");
        let log = dir.join("events.jsonl");
        let rotated = dir.join("events.jsonl.1");
        // Rotated file holds the full old contents; new file is shorter than
        // our offset (12) into the old one.
        std::fs::write(&rotated, b"{\"old\":1}\n{\"old\":2}\n").unwrap();
        std::fs::write(&log, b"{\"new\":1}\n").unwrap();
        let batch = next_batch(&log, &rotated, 12).unwrap();
        assert_eq!(batch.data, b"old\":2}\n".to_vec());
        assert_eq!(batch.next_offset, 0);
        // Next call reads the new file from the top.
        let batch = next_batch(&log, &rotated, 0).unwrap();
        assert_eq!(batch.data, b"{\"new\":1}\n");
    }

    #[test]
    fn rotation_with_missing_rotated_file_restarts_cleanly() {
        let dir = tmp_dir("rotate_missing");
        let log = dir.join("events.jsonl");
        std::fs::write(&log, b"{\"new\":1}\n").unwrap();
        let batch = next_batch(&log, &dir.join("events.jsonl.1"), 999).unwrap();
        assert!(batch.data.is_empty());
        assert_eq!(batch.next_offset, 0);
    }

    #[test]
    fn state_roundtrip_and_fresh_install_id() {
        let dir = tmp_dir("state");
        let path = dir.join("shipper_state.json");
        let fresh = load_state(&path);
        assert_eq!(fresh.offset, 0);
        assert_eq!(fresh.install_id.len(), 36);
        save_state(&path, &fresh);
        let reloaded = load_state(&path);
        assert_eq!(reloaded.install_id, fresh.install_id);
        // A second fresh load (different path) gets a different UUID.
        let other = load_state(&dir.join("nope.json"));
        assert_ne!(other.install_id, fresh.install_id);
    }
}
