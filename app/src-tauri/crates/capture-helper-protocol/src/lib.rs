use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io::{Read, Write};

pub const PROTOCOL_NAME: &str = "murmur.capture_probe";
pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_FRAME_BYTES: usize = 4 * 1024;
pub const MAX_NONCE_BYTES: usize = 128;
pub const SYNTHETIC_FIXTURE: &str = "seq-v1";
pub const SYNTHETIC_FIXTURE_CHUNKS: u64 = 64;
pub const SYNTHETIC_FIXTURE_DIGEST: &str =
    "9fda676f94adbf56e31e91462c702dcda9fcf989eece435876a28778782abfd3";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CapturePhase {
    Enumeration,
    StreamOpen,
    AwaitingFirstCallback,
    Active,
    Stopping,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FailureCode {
    PermissionDenied,
    NoInputDevice,
    EnumerationFailed,
    ConfigurationFailed,
    StreamOpenFailed,
    StreamStartFailed,
    StreamError,
    CallbackStalled,
    InvalidMessage,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum HostMessage {
    Hello {
        protocol: String,
        version: u16,
        session_nonce: String,
    },
    Cancel {
        protocol: String,
        version: u16,
        session_nonce: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum HelperMessage {
    Phase {
        protocol: String,
        version: u16,
        session_nonce: String,
        phase: CapturePhase,
    },
    Ready {
        protocol: String,
        version: u16,
        session_nonce: String,
    },
    FirstCallback {
        protocol: String,
        version: u16,
        session_nonce: String,
        callback_latency_ms: u64,
    },
    CallbackHealth {
        protocol: String,
        version: u16,
        session_nonce: String,
        callback_count_bucket: String,
    },
    SyntheticChunk {
        protocol: String,
        version: u16,
        session_nonce: String,
        fixture: String,
        fixture_digest: String,
        sequence: u64,
    },
    Failure {
        protocol: String,
        version: u16,
        session_nonce: String,
        code: FailureCode,
    },
    Stopped {
        protocol: String,
        version: u16,
        session_nonce: String,
    },
}

impl HostMessage {
    pub fn nonce(&self) -> &str {
        match self {
            Self::Hello { session_nonce, .. } | Self::Cancel { session_nonce, .. } => session_nonce,
        }
    }
}

impl HelperMessage {
    pub fn nonce(&self) -> &str {
        match self {
            Self::Phase { session_nonce, .. }
            | Self::Ready { session_nonce, .. }
            | Self::FirstCallback { session_nonce, .. }
            | Self::CallbackHealth { session_nonce, .. }
            | Self::SyntheticChunk { session_nonce, .. }
            | Self::Failure { session_nonce, .. }
            | Self::Stopped { session_nonce, .. } => session_nonce,
        }
    }
}

pub fn valid_host_message(message: &HostMessage) -> bool {
    match message {
        HostMessage::Hello {
            protocol,
            version,
            session_nonce,
        }
        | HostMessage::Cancel {
            protocol,
            version,
            session_nonce,
        } => {
            protocol == PROTOCOL_NAME
                && *version == PROTOCOL_VERSION
                && !session_nonce.is_empty()
                && session_nonce.len() <= MAX_NONCE_BYTES
        }
    }
}

pub fn valid_helper_message(message: &HelperMessage, expected_nonce: &str) -> bool {
    let (protocol, version) = match message {
        HelperMessage::Phase {
            protocol, version, ..
        }
        | HelperMessage::Ready {
            protocol, version, ..
        }
        | HelperMessage::FirstCallback {
            protocol, version, ..
        }
        | HelperMessage::CallbackHealth {
            protocol, version, ..
        }
        | HelperMessage::SyntheticChunk {
            protocol, version, ..
        }
        | HelperMessage::Failure {
            protocol, version, ..
        }
        | HelperMessage::Stopped {
            protocol, version, ..
        } => (protocol, version),
    };
    protocol == PROTOCOL_NAME && *version == PROTOCOL_VERSION && message.nonce() == expected_nonce
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame header is incomplete")]
    IncompleteHeader,
    #[error("frame length {0} exceeds the protocol limit")]
    TooLarge(usize),
    #[error("frame body is incomplete")]
    IncompleteBody,
    #[error("frame JSON is invalid")]
    InvalidJson,
    #[error("frame write failed")]
    WriteFailed,
}

pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> Result<T, FrameError> {
    let mut header = [0_u8; 4];
    reader
        .read_exact(&mut header)
        .map_err(|_| FrameError::IncompleteHeader)?;
    let length = u32::from_be_bytes(header) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(length));
    }
    let mut body = vec![0_u8; length];
    reader
        .read_exact(&mut body)
        .map_err(|_| FrameError::IncompleteBody)?;
    serde_json::from_slice(&body).map_err(|_| FrameError::InvalidJson)
}

pub fn write_frame<T: Serialize>(writer: &mut impl Write, value: &T) -> Result<(), FrameError> {
    let body = serde_json::to_vec(value).map_err(|_| FrameError::InvalidJson)?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(body.len()));
    }
    writer
        .write_all(&(body.len() as u32).to_be_bytes())
        .and_then(|_| writer.write_all(&body))
        .and_then(|_| writer.flush())
        .map_err(|_| FrameError::WriteFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_round_trip() {
        let message = HostMessage::Hello {
            protocol: PROTOCOL_NAME.to_string(),
            version: PROTOCOL_VERSION,
            session_nonce: "nonce-1".to_string(),
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &message).unwrap();
        assert_eq!(
            read_frame::<HostMessage>(&mut bytes.as_slice()).unwrap(),
            message
        );
        assert!(valid_host_message(&message));
    }

    #[test]
    fn oversized_header_fails_before_allocation() {
        let bytes = ((MAX_FRAME_BYTES + 1) as u32).to_be_bytes();
        assert!(matches!(
            read_frame::<HostMessage>(&mut bytes.as_slice()),
            Err(FrameError::TooLarge(_))
        ));
    }

    #[test]
    fn synthetic_fixture_contract_is_fixed_and_content_free() {
        assert_eq!(SYNTHETIC_FIXTURE, "seq-v1");
        assert_eq!(SYNTHETIC_FIXTURE_CHUNKS, 64);
        assert_eq!(SYNTHETIC_FIXTURE_DIGEST.len(), 64);
        assert!(SYNTHETIC_FIXTURE_DIGEST
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()));
    }
}
