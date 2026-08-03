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

// Production capture uses a separate, binary-framed protocol. Probe v1 above
// remains stable so shipped attribution/recovery evidence stays readable.
pub const PRODUCTION_PROTOCOL_NAME: &str = "murmur.capture";
pub const PRODUCTION_PROTOCOL_VERSION: u16 = 3;
pub const PRODUCTION_MAGIC: [u8; 4] = *b"MRMR";
pub const PRODUCTION_HEADER_BYTES: usize = 36;
pub const MAX_CONTROL_BYTES: usize = 16 * 1024;
pub const MAX_PCM_SAMPLES: usize = 16 * 1024;
pub type SessionNonce = [u8; 16];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureBackend {
    Cpal,
    Auhal,
}

impl CaptureBackend {
    pub fn fallback(self) -> Self {
        match self {
            Self::Cpal => Self::Auhal,
            Self::Auhal => Self::Cpal,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductionDevice {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ProductionHostMessage {
    Hello,
    Enumerate,
    Start {
        device_id: Option<String>,
        backend: CaptureBackend,
    },
    Stop,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ProductionHelperMessage {
    HelloAck,
    Devices {
        devices: Vec<ProductionDevice>,
    },
    Phase {
        phase: CapturePhase,
        backend: CaptureBackend,
    },
    SetupStep {
        backend: CaptureBackend,
        step: CaptureSetupStep,
        transition: SetupTransition,
    },
    BackendFallback {
        from: CaptureBackend,
        to: CaptureBackend,
        reason: FailureCode,
    },
    Failure {
        code: FailureCode,
        backend: CaptureBackend,
        retained_samples: u64,
    },
    Stopped {
        retained_samples: u64,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CaptureSetupStep {
    DeviceResolution,
    AudioUnitCreation,
    FormatConfiguration,
    CallbackInstallation,
    DefaultConfig,
    StreamBuild,
    StreamStart,
    AwaitingFirstCallback,
}

impl CaptureSetupStep {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DeviceResolution => "device_resolution",
            Self::AudioUnitCreation => "audio_unit_creation",
            Self::FormatConfiguration => "format_configuration",
            Self::CallbackInstallation => "callback_installation",
            Self::DefaultConfig => "default_config",
            Self::StreamBuild => "stream_build",
            Self::StreamStart => "stream_start",
            Self::AwaitingFirstCallback => "awaiting_first_callback",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SetupTransition {
    Entered,
    Completed,
}

impl SetupTransition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Entered => "entered",
            Self::Completed => "completed",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProductionPcm {
    pub sequence: u64,
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProductionFrame<T> {
    Control(T),
    Pcm(ProductionPcm),
}

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
    #[error("production frame header is invalid")]
    InvalidHeader,
    #[error("production frame capture identity does not match")]
    StaleCapture,
    #[error("production PCM frame is malformed")]
    InvalidPcm,
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

fn write_production_header(
    writer: &mut impl Write,
    kind: u8,
    payload_len: usize,
    capture_id: u64,
    nonce: SessionNonce,
) -> Result<(), FrameError> {
    let limit = if kind == 1 {
        16 + MAX_PCM_SAMPLES * std::mem::size_of::<f32>()
    } else {
        MAX_CONTROL_BYTES
    };
    if payload_len > limit {
        return Err(FrameError::TooLarge(payload_len));
    }
    let mut header = [0_u8; PRODUCTION_HEADER_BYTES];
    header[0..4].copy_from_slice(&PRODUCTION_MAGIC);
    header[4..6].copy_from_slice(&PRODUCTION_PROTOCOL_VERSION.to_le_bytes());
    header[6] = kind;
    header[8..12].copy_from_slice(&(payload_len as u32).to_le_bytes());
    header[12..20].copy_from_slice(&capture_id.to_le_bytes());
    header[20..36].copy_from_slice(&nonce);
    writer
        .write_all(&header)
        .map_err(|_| FrameError::WriteFailed)
}

pub fn write_production_control<T: Serialize>(
    writer: &mut impl Write,
    capture_id: u64,
    nonce: SessionNonce,
    value: &T,
) -> Result<(), FrameError> {
    let body = serde_json::to_vec(value).map_err(|_| FrameError::InvalidJson)?;
    write_production_header(writer, 0, body.len(), capture_id, nonce)?;
    writer
        .write_all(&body)
        .and_then(|_| writer.flush())
        .map_err(|_| FrameError::WriteFailed)
}

pub fn write_production_pcm(
    writer: &mut impl Write,
    capture_id: u64,
    nonce: SessionNonce,
    sequence: u64,
    sample_rate: u32,
    samples: &[f32],
) -> Result<(), FrameError> {
    if samples.is_empty() || samples.len() > MAX_PCM_SAMPLES {
        return Err(FrameError::InvalidPcm);
    }
    let payload_len = 16 + samples.len() * std::mem::size_of::<f32>();
    write_production_header(writer, 1, payload_len, capture_id, nonce)?;
    writer
        .write_all(&sequence.to_le_bytes())
        .and_then(|_| writer.write_all(&sample_rate.to_le_bytes()))
        .and_then(|_| writer.write_all(&(samples.len() as u32).to_le_bytes()))
        .map_err(|_| FrameError::WriteFailed)?;
    for sample in samples {
        writer
            .write_all(&sample.to_bits().to_le_bytes())
            .map_err(|_| FrameError::WriteFailed)?;
    }
    writer.flush().map_err(|_| FrameError::WriteFailed)
}

pub fn read_production_frame<T: DeserializeOwned>(
    reader: &mut impl Read,
    expected_capture_id: u64,
    expected_nonce: SessionNonce,
) -> Result<ProductionFrame<T>, FrameError> {
    let mut header = [0_u8; PRODUCTION_HEADER_BYTES];
    reader
        .read_exact(&mut header)
        .map_err(|_| FrameError::IncompleteHeader)?;
    if header[0..4] != PRODUCTION_MAGIC
        || u16::from_le_bytes([header[4], header[5]]) != PRODUCTION_PROTOCOL_VERSION
        || header[7] != 0
    {
        return Err(FrameError::InvalidHeader);
    }
    let kind = header[6];
    let length = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
    let capture_id = u64::from_le_bytes(header[12..20].try_into().unwrap());
    let nonce: SessionNonce = header[20..36].try_into().unwrap();
    if capture_id != expected_capture_id || nonce != expected_nonce {
        return Err(FrameError::StaleCapture);
    }
    match kind {
        0 if length <= MAX_CONTROL_BYTES => {
            let mut body = vec![0_u8; length];
            reader
                .read_exact(&mut body)
                .map_err(|_| FrameError::IncompleteBody)?;
            let value = serde_json::from_slice(&body).map_err(|_| FrameError::InvalidJson)?;
            Ok(ProductionFrame::Control(value))
        }
        1 if (16..=16 + MAX_PCM_SAMPLES * 4).contains(&length) => {
            let mut body = vec![0_u8; length];
            reader
                .read_exact(&mut body)
                .map_err(|_| FrameError::IncompleteBody)?;
            let sequence = u64::from_le_bytes(body[0..8].try_into().unwrap());
            let sample_rate = u32::from_le_bytes(body[8..12].try_into().unwrap());
            let count = u32::from_le_bytes(body[12..16].try_into().unwrap()) as usize;
            if count == 0 || count > MAX_PCM_SAMPLES || length != 16 + count * 4 {
                return Err(FrameError::InvalidPcm);
            }
            let samples = body[16..]
                .chunks_exact(4)
                .map(|bytes| f32::from_bits(u32::from_le_bytes(bytes.try_into().unwrap())))
                .collect();
            Ok(ProductionFrame::Pcm(ProductionPcm {
                sequence,
                sample_rate,
                samples,
            }))
        }
        0 | 1 => Err(FrameError::TooLarge(length)),
        _ => Err(FrameError::InvalidHeader),
    }
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

    #[test]
    fn production_pcm_round_trip_is_capture_scoped_and_bounded() {
        let nonce = [7_u8; 16];
        let mut bytes = Vec::new();
        write_production_pcm(&mut bytes, 42, nonce, 3, 48_000, &[0.25, -0.5]).unwrap();
        let frame =
            read_production_frame::<ProductionHelperMessage>(&mut bytes.as_slice(), 42, nonce)
                .unwrap();
        assert_eq!(
            frame,
            ProductionFrame::Pcm(ProductionPcm {
                sequence: 3,
                sample_rate: 48_000,
                samples: vec![0.25, -0.5],
            })
        );
        assert!(matches!(
            read_production_frame::<ProductionHelperMessage>(&mut bytes.as_slice(), 43, nonce),
            Err(FrameError::StaleCapture)
        ));
    }

    #[test]
    fn production_setup_telemetry_is_typed_and_content_free() {
        let nonce = [9_u8; 16];
        let message = ProductionHelperMessage::SetupStep {
            backend: CaptureBackend::Auhal,
            step: CaptureSetupStep::AudioUnitCreation,
            transition: SetupTransition::Entered,
        };
        let mut bytes = Vec::new();
        write_production_control(&mut bytes, 7, nonce, &message).unwrap();
        assert_eq!(
            read_production_frame::<ProductionHelperMessage>(&mut bytes.as_slice(), 7, nonce)
                .unwrap(),
            ProductionFrame::Control(message.clone())
        );

        let serialized = serde_json::to_string(&message).unwrap();
        assert!(!serialized.contains("deviceId"));
        assert!(!serialized.contains("deviceName"));
        assert!(!serialized.contains("uid"));
        assert!(!serialized.contains("error"));
    }
}
