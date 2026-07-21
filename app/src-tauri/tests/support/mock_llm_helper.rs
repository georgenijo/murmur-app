//! Protocol-v1 mock of `murmur-llm-sidecar`, for supervisor integration tests.
//!
//! It never links llama.cpp and needs no real model. It performs the
//! hello/ready handshake, verifies the inherited model fd 3 is a regular file
//! (proving the supervisor's fd-passing), then behaves per the `MOCK_SCENARIO`
//! environment variable the test sets when spawning:
//!
//! - `happy`               — reply Result immediately (optional `MOCK_DELAY_MS`)
//! - `wrong_nonce`         — send Ready with a mismatched session nonce
//! - `crash_on_transform`  — exit(101) on the first Transform
//! - `malformed_on_transform` — emit an invalid-JSON frame
//! - `oversized_on_transform` — emit a length prefix over the 64 KiB frame cap
//! - `slow_ack_cancel`     — never Result; reply Cancelled to a Cancel
//! - `slow_ignore_cancel`  — never Result; ignore Cancel (forces a kill)
//!
//! The real supervisor spawns with `env_clear`, so these vars only exist for the
//! mock via the supervisor's test-only constructor.

use std::io::Write;
use std::sync::mpsc;
use std::time::Duration;

use murmur_local_llm_protocol::{
    read_frame, write_frame, FinishReason, HelperMessage, HostMessage, ModelIdentity, MODEL_FD,
    PROTOCOL_NAME, PROTOCOL_VERSION,
};

fn scenario() -> String {
    std::env::var("MOCK_SCENARIO").unwrap_or_else(|_| "happy".to_string())
}

/// Confirm fd 3 was inherited as a readable regular file.
fn verify_model_fd() -> bool {
    let dup = unsafe { libc::dup(MODEL_FD) };
    if dup < 0 {
        return false;
    }
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::fstat(dup, &mut stat) };
    unsafe { libc::close(dup) };
    rc == 0 && (stat.st_mode & libc::S_IFMT) == libc::S_IFREG
}

fn main() {
    if !verify_model_fd() {
        std::process::exit(3);
    }

    let scenario = scenario();
    let mut stdout = std::io::stdout();

    // --- Handshake ---
    let mut stdin = std::io::stdin();
    let hello = match read_frame::<HostMessage>(&mut stdin) {
        Ok(h) => h,
        Err(_) => std::process::exit(4),
    };
    let (session_nonce, model) = match hello {
        HostMessage::Hello {
            session_nonce,
            model,
            ..
        } => (session_nonce, model),
        _ => std::process::exit(5),
    };

    let ready_nonce = if scenario == "wrong_nonce" {
        "WRONG-NONCE".to_string()
    } else {
        session_nonce.clone()
    };
    let ready = HelperMessage::Ready {
        protocol: PROTOCOL_NAME.to_string(),
        version: PROTOCOL_VERSION,
        session_nonce: ready_nonce,
        runtime_version: "mock".to_string(),
        model: ModelIdentity {
            id: model.id,
            sha256: model.sha256,
            size_bytes: model.size_bytes,
        },
        backend: "mock".to_string(),
    };
    if write_frame(&mut stdout, &ready).is_err() {
        std::process::exit(6);
    }
    if scenario == "wrong_nonce" {
        // Let the supervisor reject the handshake and kill us.
        std::thread::sleep(Duration::from_secs(5));
        return;
    }

    // --- Reader thread: forward host frames even while "processing". ---
    let (tx, rx) = mpsc::channel::<Option<HostMessage>>();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        loop {
            match read_frame::<HostMessage>(&mut stdin) {
                Ok(frame) => {
                    if tx.send(Some(frame)).is_err() {
                        break;
                    }
                }
                Err(_) => {
                    let _ = tx.send(None);
                    break;
                }
            }
        }
    });

    while let Ok(Some(message)) = rx.recv() {
        match message {
            HostMessage::Transform { request_id, .. } => {
                match scenario.as_str() {
                    "crash_on_transform" => std::process::exit(101),
                    "malformed_on_transform" => {
                        // Valid length prefix, invalid JSON body → InvalidJson.
                        let body = b"{ this is not json";
                        let _ = stdout.write_all(&(body.len() as u32).to_be_bytes());
                        let _ = stdout.write_all(body);
                        let _ = stdout.flush();
                    }
                    "oversized_on_transform" => {
                        // Length header beyond the 64 KiB cap → rejected pre-alloc.
                        let too_large: u32 = 64 * 1024 + 1;
                        let _ = stdout.write_all(&too_large.to_be_bytes());
                        let _ = stdout.write_all(b"xx");
                        let _ = stdout.flush();
                    }
                    "slow_ack_cancel" | "slow_ignore_cancel" => {
                        // Never send a Result; wait for the Cancel below.
                    }
                    "wrong_nonce_on_result" => {
                        // Well-formed Result frame but with a mismatched session
                        // nonce — the supervisor must reject every frame's nonce.
                        let result = HelperMessage::Result {
                            protocol: PROTOCOL_NAME.to_string(),
                            version: PROTOCOL_VERSION,
                            session_nonce: "WRONG-NONCE".to_string(),
                            request_id,
                            output: "mock-output".to_string(),
                            finish_reason: FinishReason::Stop,
                            output_tokens: 3,
                        };
                        let _ = write_frame(&mut stdout, &result);
                    }
                    _ => {
                        if let Ok(ms) = std::env::var("MOCK_DELAY_MS") {
                            if let Ok(ms) = ms.parse::<u64>() {
                                std::thread::sleep(Duration::from_millis(ms));
                            }
                        }
                        let result = HelperMessage::Result {
                            protocol: PROTOCOL_NAME.to_string(),
                            version: PROTOCOL_VERSION,
                            session_nonce: session_nonce.clone(),
                            request_id,
                            output: "mock-output".to_string(),
                            finish_reason: FinishReason::Stop,
                            output_tokens: 3,
                        };
                        let _ = write_frame(&mut stdout, &result);
                    }
                }
            }
            HostMessage::Cancel { request_id, .. } => {
                if scenario == "slow_ack_cancel" {
                    let cancelled = HelperMessage::Cancelled {
                        protocol: PROTOCOL_NAME.to_string(),
                        version: PROTOCOL_VERSION,
                        session_nonce: session_nonce.clone(),
                        request_id,
                    };
                    let _ = write_frame(&mut stdout, &cancelled);
                }
                // slow_ignore_cancel: intentionally drop it.
            }
            HostMessage::Shutdown { .. } => {
                let stopped = HelperMessage::Stopped {
                    protocol: PROTOCOL_NAME.to_string(),
                    version: PROTOCOL_VERSION,
                    session_nonce: session_nonce.clone(),
                };
                let _ = write_frame(&mut stdout, &stopped);
                return;
            }
            HostMessage::Hello { .. } => std::process::exit(7),
        }
    }
}
