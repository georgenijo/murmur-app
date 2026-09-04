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
pub const PRODUCTION_PROTOCOL_VERSION: u16 = 8;
pub const PRODUCTION_MAGIC: [u8; 4] = *b"MRMR";
pub const PRODUCTION_HEADER_BYTES: usize = 36;
pub const MAX_CONTROL_BYTES: usize = 16 * 1024;
pub const MAX_PCM_SAMPLES: usize = 16 * 1024;
/// Maximum exact input-device count carried by content-free resolution
/// evidence. A helper must continue resolving across the full candidate list
/// and set `input_device_count_capped` when the observed count exceeds this
/// telemetry-only bound.
pub const MAX_INPUT_DEVICE_COUNT: usize = 256;
pub type SessionNonce = [u8; 16];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureBackend {
    Cpal,
    Auhal,
}

/// Physical source carried by a production PCM frame. Dictation emits only
/// `Microphone`; meeting capture keeps microphone and system output separate
/// all the way to durable storage.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum CaptureChannel {
    Microphone,
    System,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EchoCancellationMode {
    #[default]
    Disabled,
    Enabled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EchoCancellationBypassReason {
    InitializationFailed,
    UnsupportedFormat,
    RenderDiscontinuity,
    ProcessorFailed,
    ProcessingBacklog,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum EchoCancellationStatus {
    Disabled,
    Active,
    Bypassed {
        reason: EchoCancellationBypassReason,
    },
}

impl CaptureChannel {
    fn wire_value(self) -> u8 {
        match self {
            Self::Microphone => 1,
            Self::System => 2,
        }
    }

    fn from_wire(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Microphone),
            2 => Some(Self::System),
            _ => None,
        }
    }
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
    /// Installs passive Core Audio topology/default-input listeners in the
    /// isolated worker. Listener callbacks never enumerate or open devices.
    WatchInputTopology,
    Start {
        device_id: Option<String>,
        backend: CaptureBackend,
    },
    StartMeeting {
        device_id: Option<String>,
        backend: CaptureBackend,
        echo_cancellation: EchoCancellationMode,
    },
    /// Explicit, user-initiated CATap probe. The host never sends this from a
    /// focus listener or permission polling loop because creating a tap is the
    /// permission request on macOS.
    ProbeSystemAudio,
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
        default_input_id: Option<String>,
    },
    InputTopologyWatchReady,
    /// Content-free invalidation signal. The host performs any enumeration
    /// later, under its shared idle-HAL boundary.
    InputTopologyChanged,
    Phase {
        phase: CapturePhase,
        backend: CaptureBackend,
    },
    SetupStep {
        backend: CaptureBackend,
        step: CaptureSetupStep,
        transition: SetupTransition,
    },
    /// Content-free evidence from the live device-resolution pass used by one
    /// capture backend attempt. The requested selector, device labels, stable
    /// IDs, default ID, and raw backend errors never cross this boundary.
    InputResolution {
        backend: CaptureBackend,
        input_enumeration_ok: bool,
        /// `Some` only when a pinned selector was checked against a successful
        /// backend enumeration. System-default mode and failed enumeration are
        /// represented by `None`; the host owns the mode and disambiguates the
        /// two without serializing the selector.
        requested_present: Option<bool>,
        input_device_count: u16,
        input_device_count_capped: bool,
        default_input_available: bool,
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
    MeetingPhase {
        phase: CapturePhase,
        channel: CaptureChannel,
    },
    MeetingSetupStep {
        channel: CaptureChannel,
        step: CaptureSetupStep,
        transition: SetupTransition,
    },
    MeetingEchoCancellation {
        status: EchoCancellationStatus,
    },
    SystemAudioPermission {
        status: SystemAudioPermissionStatus,
        /// Whether the tap delivered at least one callback inside the probe's
        /// observation window. Capture readiness is independent of
        /// authorization: a granted tap on a silent Mac reports
        /// `Granted` with `audio_flowing: false`, which is a healthy state and
        /// never a permission failure.
        audio_flowing: bool,
    },
    MeetingFailure {
        code: FailureCode,
        channel: Option<CaptureChannel>,
        microphone_samples: u64,
        system_samples: u64,
    },
    MeetingStopped {
        microphone_samples: u64,
        system_samples: u64,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SystemAudioPermissionStatus {
    Granted,
    Denied,
    Unsupported,
}

/// Capture setup steps bracket the native calls the worker makes, so a step
/// that reports `Entered` without `Completed` identifies the exact operation
/// that hung. Step names are content-free: no device identity ever rides on
/// this message. AUHAL steps map to native Core Audio calls as follows:
///
/// - `AudioUnitNew`: `AudioComponentFindNext`, `AudioComponentInstanceNew`,
///   and `AudioUnitInitialize` (one bracket; the safe wrapper creates and
///   initializes in a single call)
/// - `EnableInputIo` / `DisableOutputIo`:
///   `AudioUnitSetProperty(kAudioOutputUnitProperty_EnableIO)` on the input /
///   output element
/// - `SetCurrentDevice`:
///   `AudioUnitSetProperty(kAudioOutputUnitProperty_CurrentDevice)`
/// - `FormatConfiguration`:
///   `AudioUnitSetProperty(kAudioUnitProperty_StreamFormat)`
/// - `CallbackInstallation`: buffer-size query plus
///   `AudioUnitSetProperty(kAudioOutputUnitProperty_SetInputCallback)`
/// - `StreamStart`: `AudioOutputUnitStart` (AUHAL) or cpal `Stream::play`
///   (CPAL)
///
/// `AudioUnitCreation` is the legacy coarse bracket that covered
/// `AudioUnitNew` through `SetCurrentDevice`; it is retained for protocol
/// stability but no longer emitted.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CaptureSetupStep {
    DeviceResolution,
    AudioUnitCreation,
    AudioUnitNew,
    EnableInputIo,
    DisableOutputIo,
    SetCurrentDevice,
    FormatConfiguration,
    CallbackInstallation,
    DefaultConfig,
    StreamBuild,
    StreamStart,
    AwaitingFirstCallback,
    SystemTapCreate,
    AggregateDeviceCreate,
    IoProcCreate,
    IoProcStart,
}

impl CaptureSetupStep {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DeviceResolution => "device_resolution",
            Self::AudioUnitCreation => "audio_unit_creation",
            Self::AudioUnitNew => "audio_unit_new",
            Self::EnableInputIo => "enable_input_io",
            Self::DisableOutputIo => "disable_output_io",
            Self::SetCurrentDevice => "set_current_device",
            Self::FormatConfiguration => "format_configuration",
            Self::CallbackInstallation => "callback_installation",
            Self::DefaultConfig => "default_config",
            Self::StreamBuild => "stream_build",
            Self::StreamStart => "stream_start",
            Self::AwaitingFirstCallback => "awaiting_first_callback",
            Self::SystemTapCreate => "system_tap_create",
            Self::AggregateDeviceCreate => "aggregate_device_create",
            Self::IoProcCreate => "io_proc_create",
            Self::IoProcStart => "io_proc_start",
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
    pub channel: CaptureChannel,
    pub sequence: u64,
    pub sample_rate: u32,
    /// Nanoseconds elapsed on the worker's monotonic clock when the callback
    /// batch was drained. This orders channels best-effort; their hardware
    /// clocks are intentionally not presented as sample-accurate peers.
    pub captured_at_ns: u64,
    pub sample_offset: u64,
    pub samples: Vec<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProductionPcmMetadata {
    pub channel: CaptureChannel,
    pub sequence: u64,
    pub sample_rate: u32,
    pub captured_at_ns: u64,
    pub sample_offset: u64,
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
    UnsupportedOs,
    SystemAudioUnavailable,
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

/// Validate the bounded, cross-field invariants for production input
/// resolution evidence. `requested_device` is host-owned request metadata and
/// is never placed on the wire or in telemetry.
pub fn valid_input_resolution_evidence(
    requested_device: bool,
    input_enumeration_ok: bool,
    requested_present: Option<bool>,
    input_device_count: u16,
    input_device_count_capped: bool,
) -> bool {
    if usize::from(input_device_count) > MAX_INPUT_DEVICE_COUNT {
        return false;
    }
    if !input_enumeration_ok {
        return requested_present.is_none()
            && input_device_count == 0
            && !input_device_count_capped;
    }
    if input_device_count_capped && usize::from(input_device_count) != MAX_INPUT_DEVICE_COUNT {
        return false;
    }
    requested_present.is_some() == requested_device
        && !matches!(requested_present, Some(true) if input_device_count == 0)
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
    channel: u8,
    payload_len: usize,
    capture_id: u64,
    nonce: SessionNonce,
) -> Result<(), FrameError> {
    let limit = if kind == 1 {
        32 + MAX_PCM_SAMPLES * std::mem::size_of::<f32>()
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
    header[7] = channel;
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
    write_production_header(writer, 0, 0, body.len(), capture_id, nonce)?;
    writer
        .write_all(&body)
        .and_then(|_| writer.flush())
        .map_err(|_| FrameError::WriteFailed)
}

pub fn write_production_pcm(
    writer: &mut impl Write,
    capture_id: u64,
    nonce: SessionNonce,
    metadata: ProductionPcmMetadata,
    samples: &[f32],
) -> Result<(), FrameError> {
    if samples.is_empty() || samples.len() > MAX_PCM_SAMPLES {
        return Err(FrameError::InvalidPcm);
    }
    let payload_len = 32 + std::mem::size_of_val(samples);
    write_production_header(
        writer,
        1,
        metadata.channel.wire_value(),
        payload_len,
        capture_id,
        nonce,
    )?;
    writer
        .write_all(&metadata.sequence.to_le_bytes())
        .and_then(|_| writer.write_all(&metadata.sample_rate.to_le_bytes()))
        .and_then(|_| writer.write_all(&(samples.len() as u32).to_le_bytes()))
        .and_then(|_| writer.write_all(&metadata.captured_at_ns.to_le_bytes()))
        .and_then(|_| writer.write_all(&metadata.sample_offset.to_le_bytes()))
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
    {
        return Err(FrameError::InvalidHeader);
    }
    let kind = header[6];
    let channel = header[7];
    let length = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
    let capture_id = u64::from_le_bytes(header[12..20].try_into().unwrap());
    let nonce: SessionNonce = header[20..36].try_into().unwrap();
    if capture_id != expected_capture_id || nonce != expected_nonce {
        return Err(FrameError::StaleCapture);
    }
    match kind {
        0 if channel == 0 && length <= MAX_CONTROL_BYTES => {
            let mut body = vec![0_u8; length];
            reader
                .read_exact(&mut body)
                .map_err(|_| FrameError::IncompleteBody)?;
            let value = serde_json::from_slice(&body).map_err(|_| FrameError::InvalidJson)?;
            Ok(ProductionFrame::Control(value))
        }
        1 if (32..=32 + MAX_PCM_SAMPLES * 4).contains(&length) => {
            let channel = CaptureChannel::from_wire(channel).ok_or(FrameError::InvalidPcm)?;
            let mut body = vec![0_u8; length];
            reader
                .read_exact(&mut body)
                .map_err(|_| FrameError::IncompleteBody)?;
            let sequence = u64::from_le_bytes(body[0..8].try_into().unwrap());
            let sample_rate = u32::from_le_bytes(body[8..12].try_into().unwrap());
            let count = u32::from_le_bytes(body[12..16].try_into().unwrap()) as usize;
            let captured_at_ns = u64::from_le_bytes(body[16..24].try_into().unwrap());
            let sample_offset = u64::from_le_bytes(body[24..32].try_into().unwrap());
            if count == 0 || count > MAX_PCM_SAMPLES || length != 32 + count * 4 {
                return Err(FrameError::InvalidPcm);
            }
            let samples = body[32..]
                .chunks_exact(4)
                .map(|bytes| f32::from_bits(u32::from_le_bytes(bytes.try_into().unwrap())))
                .collect();
            Ok(ProductionFrame::Pcm(ProductionPcm {
                channel,
                sequence,
                sample_rate,
                captured_at_ns,
                sample_offset,
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
        write_production_pcm(
            &mut bytes,
            42,
            nonce,
            ProductionPcmMetadata {
                channel: CaptureChannel::Microphone,
                sequence: 3,
                sample_rate: 48_000,
                captured_at_ns: 123,
                sample_offset: 456,
            },
            &[0.25, -0.5],
        )
        .unwrap();
        let frame =
            read_production_frame::<ProductionHelperMessage>(&mut bytes.as_slice(), 42, nonce)
                .unwrap();
        assert_eq!(
            frame,
            ProductionFrame::Pcm(ProductionPcm {
                channel: CaptureChannel::Microphone,
                sequence: 3,
                sample_rate: 48_000,
                captured_at_ns: 123,
                sample_offset: 456,
                samples: vec![0.25, -0.5],
            })
        );
        assert!(matches!(
            read_production_frame::<ProductionHelperMessage>(&mut bytes.as_slice(), 43, nonce),
            Err(FrameError::StaleCapture)
        ));
    }

    #[test]
    fn meeting_echo_cancellation_contract_is_typed_and_default_off() {
        assert_eq!(
            EchoCancellationMode::default(),
            EchoCancellationMode::Disabled
        );
        let request = ProductionHostMessage::StartMeeting {
            device_id: None,
            backend: CaptureBackend::Auhal,
            echo_cancellation: EchoCancellationMode::Enabled,
        };
        let encoded = serde_json::to_vec(&request).unwrap();
        assert_eq!(
            serde_json::from_slice::<ProductionHostMessage>(&encoded).unwrap(),
            request
        );
        let status = ProductionHelperMessage::MeetingEchoCancellation {
            status: EchoCancellationStatus::Bypassed {
                reason: EchoCancellationBypassReason::ProcessorFailed,
            },
        };
        let encoded = serde_json::to_vec(&status).unwrap();
        assert_eq!(
            serde_json::from_slice::<ProductionHelperMessage>(&encoded).unwrap(),
            status
        );
    }

    #[test]
    fn production_v3_and_unknown_pcm_channels_are_rejected() {
        let nonce = [4_u8; 16];
        let mut version_mismatch = Vec::new();
        write_production_control(
            &mut version_mismatch,
            9,
            nonce,
            &ProductionHostMessage::Hello,
        )
        .unwrap();
        version_mismatch[4..6].copy_from_slice(&3_u16.to_le_bytes());
        assert!(matches!(
            read_production_frame::<ProductionHostMessage>(
                &mut version_mismatch.as_slice(),
                9,
                nonce,
            ),
            Err(FrameError::InvalidHeader)
        ));

        let mut invalid_channel = Vec::new();
        write_production_pcm(
            &mut invalid_channel,
            9,
            nonce,
            ProductionPcmMetadata {
                channel: CaptureChannel::System,
                sequence: 0,
                sample_rate: 48_000,
                captured_at_ns: 0,
                sample_offset: 0,
            },
            &[0.0],
        )
        .unwrap();
        invalid_channel[7] = 99;
        assert!(matches!(
            read_production_frame::<ProductionHelperMessage>(
                &mut invalid_channel.as_slice(),
                9,
                nonce,
            ),
            Err(FrameError::InvalidPcm)
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

    #[test]
    fn system_audio_permission_carries_capture_flow_separately() {
        let nonce = [9_u8; 16];
        for (status, audio_flowing, expected_json) in [
            (
                SystemAudioPermissionStatus::Granted,
                false,
                r#"{"type":"systemAudioPermission","status":"granted","audioFlowing":false}"#,
            ),
            (
                SystemAudioPermissionStatus::Granted,
                true,
                r#"{"type":"systemAudioPermission","status":"granted","audioFlowing":true}"#,
            ),
            (
                SystemAudioPermissionStatus::Denied,
                false,
                r#"{"type":"systemAudioPermission","status":"denied","audioFlowing":false}"#,
            ),
        ] {
            let message = ProductionHelperMessage::SystemAudioPermission {
                status,
                audio_flowing,
            };
            let mut bytes = Vec::new();
            write_production_control(&mut bytes, 21, nonce, &message).unwrap();
            assert_eq!(
                read_production_frame::<ProductionHelperMessage>(&mut bytes.as_slice(), 21, nonce)
                    .unwrap(),
                ProductionFrame::Control(message.clone())
            );
            assert_eq!(serde_json::to_string(&message).unwrap(), expected_json);
        }
    }

    #[test]
    fn production_v6_topology_watch_and_resolution_are_content_free() {
        let nonce = [5_u8; 16];
        for (message, expected_json) in [
            (
                ProductionHelperMessage::InputTopologyWatchReady,
                r#"{"type":"inputTopologyWatchReady"}"#,
            ),
            (
                ProductionHelperMessage::InputTopologyChanged,
                r#"{"type":"inputTopologyChanged"}"#,
            ),
        ] {
            let mut bytes = Vec::new();
            write_production_control(&mut bytes, 17, nonce, &message).unwrap();
            assert_eq!(
                read_production_frame::<ProductionHelperMessage>(&mut bytes.as_slice(), 17, nonce)
                    .unwrap(),
                ProductionFrame::Control(message.clone())
            );
            let serialized = serde_json::to_string(&message).unwrap();
            assert_eq!(serialized, expected_json);
            assert!(!serialized.contains("device"));
            assert!(!serialized.contains("name"));
            assert!(!serialized.contains("uid"));
        }

        let serialized = serde_json::to_string(&ProductionHostMessage::WatchInputTopology).unwrap();
        assert_eq!(serialized, r#"{"type":"watchInputTopology"}"#);

        let devices = ProductionHelperMessage::Devices {
            devices: vec![ProductionDevice {
                id: "stable-uid".to_string(),
                name: "Built-in Microphone".to_string(),
            }],
            default_input_id: Some("stable-uid".to_string()),
        };
        assert_eq!(
            serde_json::to_string(&devices).unwrap(),
            r#"{"type":"devices","devices":[{"id":"stable-uid","name":"Built-in Microphone"}],"defaultInputId":"stable-uid"}"#
        );

        let resolution = ProductionHelperMessage::InputResolution {
            backend: CaptureBackend::Auhal,
            input_enumeration_ok: true,
            requested_present: Some(false),
            input_device_count: 2,
            input_device_count_capped: false,
            default_input_available: true,
        };
        let expected = r#"{"type":"inputResolution","backend":"auhal","inputEnumerationOk":true,"requestedPresent":false,"inputDeviceCount":2,"inputDeviceCountCapped":false,"defaultInputAvailable":true}"#;
        assert_eq!(serde_json::to_string(&resolution).unwrap(), expected);
        assert!(serde_json::from_str::<ProductionHelperMessage>(
            r#"{"type":"inputResolution","backend":"auhal","inputEnumerationOk":true,"requestedPresent":false,"inputDeviceCount":2,"inputDeviceCountCapped":false,"defaultInputAvailable":true,"deviceId":"PRIVATE"}"#
        )
        .is_err());
        let mut bytes = Vec::new();
        write_production_control(&mut bytes, 17, nonce, &resolution).unwrap();
        assert_eq!(
            read_production_frame::<ProductionHelperMessage>(&mut bytes.as_slice(), 17, nonce)
                .unwrap(),
            ProductionFrame::Control(resolution)
        );
        for forbidden in [
            "stable-uid",
            "Built-in Microphone",
            "deviceId",
            "deviceName",
            "defaultInputId",
            "rawError",
        ] {
            assert!(!expected.contains(forbidden));
        }
    }

    #[test]
    fn production_v6_input_resolution_invariants_are_bounded() {
        assert!(valid_input_resolution_evidence(
            true,
            true,
            Some(false),
            2,
            false
        ));
        assert!(valid_input_resolution_evidence(false, true, None, 2, false));
        assert!(valid_input_resolution_evidence(true, false, None, 0, false));
        assert!(valid_input_resolution_evidence(
            true,
            true,
            Some(true),
            MAX_INPUT_DEVICE_COUNT as u16,
            true
        ));

        assert!(!valid_input_resolution_evidence(true, true, None, 2, false));
        assert!(!valid_input_resolution_evidence(
            false,
            true,
            Some(false),
            2,
            false
        ));
        assert!(!valid_input_resolution_evidence(
            true,
            false,
            Some(false),
            0,
            false
        ));
        assert!(!valid_input_resolution_evidence(
            true, false, None, 1, false
        ));
        assert!(!valid_input_resolution_evidence(
            true,
            true,
            Some(true),
            0,
            false
        ));
        assert!(!valid_input_resolution_evidence(
            true,
            true,
            Some(true),
            1,
            true
        ));
        assert!(!valid_input_resolution_evidence(
            true,
            true,
            Some(true),
            (MAX_INPUT_DEVICE_COUNT + 1) as u16,
            false
        ));

        let system_default = ProductionHelperMessage::InputResolution {
            backend: CaptureBackend::Cpal,
            input_enumeration_ok: true,
            requested_present: None,
            input_device_count: 0,
            input_device_count_capped: false,
            default_input_available: false,
        };
        assert_eq!(
            serde_json::to_string(&system_default).unwrap(),
            r#"{"type":"inputResolution","backend":"cpal","inputEnumerationOk":true,"requestedPresent":null,"inputDeviceCount":0,"inputDeviceCountCapped":false,"defaultInputAvailable":false}"#
        );
    }
}
