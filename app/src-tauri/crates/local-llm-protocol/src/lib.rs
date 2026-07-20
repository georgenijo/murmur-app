use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io::{Read, Write};

pub const PROTOCOL_NAME: &str = "murmur.local_llm";
pub const PROTOCOL_VERSION: u16 = 1;
pub const MODEL_FD: i32 = 3;
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
pub const MAX_INSTRUCTION_BYTES: usize = 4 * 1024;
pub const MAX_INPUT_BYTES: usize = 16 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 16 * 1024;
pub const MAX_OUTPUT_TOKENS: u32 = 2_048;
pub const MAX_CONTEXT_TOKENS: u32 = 8_192;
pub const DEFAULT_DEADLINE_MS: u64 = 15_000;
pub const MAX_DEADLINE_MS: u64 = 30_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolLimits {
    pub max_frame_bytes: u32,
    pub max_instruction_bytes: u32,
    pub max_input_bytes: u32,
    pub max_output_bytes: u32,
    pub max_output_tokens: u32,
    pub max_context_tokens: u32,
    pub max_deadline_ms: u64,
}

impl Default for ProtocolLimits {
    fn default() -> Self {
        Self {
            max_frame_bytes: MAX_FRAME_BYTES as u32,
            max_instruction_bytes: MAX_INSTRUCTION_BYTES as u32,
            max_input_bytes: MAX_INPUT_BYTES as u32,
            max_output_bytes: MAX_OUTPUT_BYTES as u32,
            max_output_tokens: MAX_OUTPUT_TOKENS,
            max_context_tokens: MAX_CONTEXT_TOKENS,
            max_deadline_ms: MAX_DEADLINE_MS,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelIdentity {
    pub id: String,
    pub sha256: String,
    pub size_bytes: u64,
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
        model: ModelIdentity,
        limits: ProtocolLimits,
    },
    Transform {
        protocol: String,
        version: u16,
        session_nonce: String,
        request_id: String,
        instruction: String,
        input: String,
        max_output_tokens: u32,
        deadline_ms: u64,
    },
    Cancel {
        protocol: String,
        version: u16,
        session_nonce: String,
        request_id: String,
    },
    Shutdown {
        protocol: String,
        version: u16,
        session_nonce: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FinishReason {
    Stop,
    Length,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    InvalidFrame,
    InvalidMessage,
    ProtocolMismatch,
    ModelMismatch,
    ModelLoadFailed,
    RuntimeUnavailable,
    Busy,
    DeadlineExceeded,
    Cancelled,
    OutputInvalid,
    ResourceLimit,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum HelperMessage {
    Ready {
        protocol: String,
        version: u16,
        session_nonce: String,
        runtime_version: String,
        model: ModelIdentity,
        backend: String,
    },
    Result {
        protocol: String,
        version: u16,
        session_nonce: String,
        request_id: String,
        output: String,
        finish_reason: FinishReason,
        output_tokens: u32,
    },
    Cancelled {
        protocol: String,
        version: u16,
        session_nonce: String,
        request_id: String,
    },
    Error {
        protocol: String,
        version: u16,
        session_nonce: String,
        request_id: Option<String>,
        code: ErrorCode,
    },
    Stopped {
        protocol: String,
        version: u16,
        session_nonce: String,
    },
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
    let body = serde_json::to_vec(value).map_err(|_| FrameError::WriteFailed)?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(body.len()));
    }
    writer
        .write_all(&(body.len() as u32).to_be_bytes())
        .and_then(|_| writer.write_all(&body))
        .and_then(|_| writer.flush())
        .map_err(|_| FrameError::WriteFailed)
}

pub fn validate_host_message(message: &HostMessage) -> Result<(), ErrorCode> {
    let (protocol, version, nonce) = match message {
        HostMessage::Hello {
            protocol,
            version,
            session_nonce,
            ..
        }
        | HostMessage::Transform {
            protocol,
            version,
            session_nonce,
            ..
        }
        | HostMessage::Cancel {
            protocol,
            version,
            session_nonce,
            ..
        }
        | HostMessage::Shutdown {
            protocol,
            version,
            session_nonce,
        } => (protocol, version, session_nonce),
    };
    if protocol != PROTOCOL_NAME
        || *version != PROTOCOL_VERSION
        || nonce.len() > 64
        || nonce.is_empty()
    {
        return Err(ErrorCode::ProtocolMismatch);
    }

    if let HostMessage::Transform {
        request_id,
        instruction,
        input,
        max_output_tokens,
        deadline_ms,
        ..
    } = message
    {
        if request_id.is_empty()
            || request_id.len() > 64
            || instruction.len() > MAX_INSTRUCTION_BYTES
            || input.len() > MAX_INPUT_BYTES
            || *max_output_tokens == 0
            || *max_output_tokens > MAX_OUTPUT_TOKENS
            || *deadline_ms == 0
            || *deadline_ms > MAX_DEADLINE_MS
            || instruction.contains('\0')
            || input.contains('\0')
        {
            return Err(ErrorCode::InvalidMessage);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn frame_round_trip() {
        let message = HostMessage::Shutdown {
            protocol: PROTOCOL_NAME.to_string(),
            version: PROTOCOL_VERSION,
            session_nonce: "nonce".to_string(),
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &message).unwrap();
        assert_eq!(
            read_frame::<HostMessage>(&mut Cursor::new(bytes)).unwrap(),
            message
        );
    }

    #[test]
    fn oversized_frame_is_rejected_before_allocation() {
        let mut bytes = ((MAX_FRAME_BYTES + 1) as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(b"{}");
        assert!(matches!(
            read_frame::<HostMessage>(&mut Cursor::new(bytes)),
            Err(FrameError::TooLarge(_))
        ));
    }

    #[test]
    fn transform_bounds_are_enforced() {
        let message = HostMessage::Transform {
            protocol: PROTOCOL_NAME.to_string(),
            version: PROTOCOL_VERSION,
            session_nonce: "nonce".to_string(),
            request_id: "request".to_string(),
            instruction: "x".repeat(MAX_INSTRUCTION_BYTES + 1),
            input: "text".to_string(),
            max_output_tokens: 10,
            deadline_ms: DEFAULT_DEADLINE_MS,
        };
        assert_eq!(
            validate_host_message(&message),
            Err(ErrorCode::InvalidMessage)
        );
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let payload = br#"{
            "type":"shutdown",
            "protocol":"murmur.local_llm",
            "version":1,
            "sessionNonce":"nonce",
            "command":"whoami"
        }"#;
        assert!(serde_json::from_slice::<HostMessage>(payload).is_err());
    }

    #[test]
    fn wire_fields_are_camel_case() {
        let message = HostMessage::Cancel {
            protocol: PROTOCOL_NAME.to_string(),
            version: PROTOCOL_VERSION,
            session_nonce: "nonce".to_string(),
            request_id: "request".to_string(),
        };
        let json = serde_json::to_value(message).unwrap();
        assert_eq!(json["sessionNonce"], "nonce");
        assert_eq!(json["requestId"], "request");
        assert!(json.get("session_nonce").is_none());
    }
}
