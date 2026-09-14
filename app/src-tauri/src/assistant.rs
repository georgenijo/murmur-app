//! Private, bounded local mirror of explicitly connected Pi conversations.
//! Content is emitted only to the main webview; IDs never become filesystem paths.
use crate::MutexExt;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::{collections::HashSet, fs, io::Write, path::PathBuf, sync::Mutex};
use tauri::Emitter;

const MAX_STORE: usize = 16 * 1024 * 1024;
const MAX_CONVERSATIONS: usize = 100;
const MAX_MESSAGES: usize = 200;
const MAX_CONNECTIONS: usize = 100;
const MAX_ACTIONS_PER_MESSAGE: usize = 16;
const MAX_STREAM_BYTES: usize = 512 * 1024;
const STORAGE_ERROR: &str = "assistant_storage_unavailable";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ActionTarget {
    pub entity_id: String,
    pub name: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ActionParameters {
    pub power: String,
    pub brightness_pct: Option<u8>,
    pub rgb_color: Option<[u8; 3]>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ActionReceipt {
    pub execution_id: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub write_attempted: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LightState {
    On,
    Off,
    Unknown,
    Unavailable,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LightAvailability {
    Available,
    Unknown,
    Unavailable,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FreshnessStatus {
    Fresh,
    Stale,
    Unknown,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FreshnessBasis {
    LastReported,
    LastUpdated,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct LightFreshness {
    pub status: FreshnessStatus,
    pub basis: Option<FreshnessBasis>,
    pub age_seconds: Option<f64>,
    pub stale_after_seconds: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct LightCapabilities {
    pub on_off: bool,
    pub brightness: Option<bool>,
    pub supported_color_modes: Option<Vec<String>>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct LightAttributes {
    pub brightness: Option<u8>,
    pub color_mode: Option<String>,
    pub rgb_color: Option<[u8; 3]>,
    pub rgbw_color: Option<[u8; 4]>,
    pub rgbww_color: Option<[u8; 5]>,
    pub hs_color: Option<[f64; 2]>,
    pub xy_color: Option<[f64; 2]>,
    pub color_temp_kelvin: Option<u32>,
    pub min_color_temp_kelvin: Option<u32>,
    pub max_color_temp_kelvin: Option<u32>,
    pub color_temp: Option<u32>,
    pub min_mireds: Option<u32>,
    pub max_mireds: Option<u32>,
    pub supported_features: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct Light {
    pub entity_id: String,
    pub aliases: Vec<String>,
    pub name: Option<String>,
    pub state: LightState,
    pub availability: LightAvailability,
    pub observed_at: String,
    pub last_changed: Option<String>,
    pub last_updated: Option<String>,
    pub last_reported: Option<String>,
    pub freshness: LightFreshness,
    pub capabilities: LightCapabilities,
    pub attributes: LightAttributes,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ActionVerification {
    pub source: String,
    pub verified_at: String,
    pub matched: Option<bool>,
    pub states: Vec<Light>,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActionStatus {
    Proposed,
    Cancelled,
    Expired,
    Executing,
    Completed,
    Failed,
    Uncertain,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct AssistantAction {
    pub schema_version: u32,
    pub action_id: String,
    pub idempotency_key: String,
    pub kind: String,
    pub actor_id: String,
    pub connection_id: String,
    pub conversation_id: String,
    pub request_id: String,
    pub targets: Vec<ActionTarget>,
    pub parameters: ActionParameters,
    pub created_at: String,
    pub expires_at: String,
    pub parameter_digest: String,
    pub status: ActionStatus,
    pub receipt: Option<ActionReceipt>,
    pub verification: Option<ActionVerification>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredConnection {
    binding: String,
    connection_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Message {
    pub id: String,
    pub request_id: String,
    pub role: String,
    pub content: String,
    pub created_at_ms: i64,
    pub status: String,
    pub error_code: Option<String>,
    #[serde(default)]
    pub actions: Vec<AssistantAction>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Conversation {
    pub id: String,
    pub title: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub messages: Vec<Message>,
    pub active_pass_id: Option<u64>,
    /// Hydrated at the command boundary only, never trusted from the disk.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub live_state: Option<String>,
    #[serde(default)]
    connection_id: Option<String>,
    #[serde(default)]
    active_action_id: Option<String>,
    #[serde(default)]
    seed_pending: bool,
    #[serde(default)]
    connection_binding: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConversationSummary {
    id: String,
    title: String,
    created_at_ms: i64,
    updated_at_ms: i64,
    message_count: usize,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Snapshot {
    version: u32,
    binding: Option<String>,
    #[serde(default)]
    connections: Vec<StoredConnection>,
    conversations: Vec<Conversation>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            version: 1,
            binding: None,
            connections: vec![],
            conversations: vec![],
        }
    }
}

#[derive(Default)]
struct Inner {
    root: Option<PathBuf>,
    snapshot: Snapshot,
    app: Option<tauri::AppHandle>,
}
#[derive(Default)]
pub(crate) struct AssistantStore(Mutex<Inner>);

#[derive(Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum WireRecord {
    #[serde(rename = "text_delta")]
    TextDelta { schema_version: u32, text: String },
    #[serde(rename = "action")]
    Action {
        schema_version: u32,
        action: AssistantAction,
    },
    #[serde(rename = "done")]
    Done { schema_version: u32 },
}

pub(crate) enum AssistantStreamUpdate {
    Text(String),
    Action(AssistantAction),
}

pub(crate) struct AssistantStreamParser {
    buffer: Vec<u8>,
    total_bytes: usize,
    done: bool,
    action_count: usize,
}

impl AssistantStreamParser {
    pub(crate) fn new() -> Self {
        Self {
            buffer: Vec::new(),
            total_bytes: 0,
            done: false,
            action_count: 0,
        }
    }

    pub(crate) fn push(
        &mut self,
        bytes: &[u8],
    ) -> Result<Vec<AssistantStreamUpdate>, &'static str> {
        self.total_bytes = self.total_bytes.saturating_add(bytes.len());
        if self.total_bytes > MAX_STREAM_BYTES {
            return Err("output_too_large");
        }
        self.buffer.extend_from_slice(bytes);
        let mut updates = Vec::new();
        while let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<_> = self.buffer.drain(..=newline).collect();
            self.parse_line(&line[..line.len() - 1], &mut updates)?;
        }
        Ok(updates)
    }

    pub(crate) fn finish(mut self) -> Result<(Vec<AssistantStreamUpdate>, usize), &'static str> {
        let mut updates = Vec::new();
        if !self.buffer.is_empty() {
            let line = std::mem::take(&mut self.buffer);
            self.parse_line(&line, &mut updates)?;
        }
        if !self.done {
            return Err("invalid_assistant_response");
        }
        Ok((updates, self.action_count))
    }

    fn parse_line(
        &mut self,
        line: &[u8],
        updates: &mut Vec<AssistantStreamUpdate>,
    ) -> Result<(), &'static str> {
        if line.is_empty() || self.done {
            return Err("invalid_assistant_response");
        }
        let record: WireRecord =
            serde_json::from_slice(line).map_err(|_| "invalid_assistant_response")?;
        match record {
            WireRecord::TextDelta {
                schema_version: 2,
                text,
            } if !text.is_empty() => updates.push(AssistantStreamUpdate::Text(text)),
            WireRecord::Action {
                schema_version: 2,
                action,
            } => {
                validate_action(&action).map_err(|_| "invalid_assistant_response")?;
                self.action_count += 1;
                if self.action_count > MAX_ACTIONS_PER_MESSAGE {
                    return Err("output_too_large");
                }
                updates.push(AssistantStreamUpdate::Action(action));
            }
            WireRecord::Done { schema_version: 2 } => self.done = true,
            _ => return Err("invalid_assistant_response"),
        }
        Ok(())
    }
}

pub(crate) struct ActionDispatch {
    pub conversation_id: String,
    pub request_id: String,
    pub connection_id: String,
    pub parameter_digest: String,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|value| value.to_string() == id)
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
fn valid_entity_id(value: &str) -> bool {
    let Some(id) = value.strip_prefix("light.") else {
        return false;
    };
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}
fn valid_timestamp(value: &str) -> bool {
    use chrono::Timelike;

    if value.len() > 64 || !value.ends_with('Z') {
        return false;
    }
    chrono::DateTime::parse_from_rfc3339(value).is_ok_and(|time| {
        if time.offset().local_minus_utc() != 0 {
            return false;
        }
        let utc = time.with_timezone(&chrono::Utc);
        let canonical = if utc.nanosecond() == 0 {
            utc.format("%Y-%m-%dT%H:%M:%SZ").to_string()
        } else {
            utc.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
        };
        canonical == value
    })
}
fn timestamp(value: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    if !valid_timestamp(value) {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(value).ok()
}
fn valid_alias(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && value.len() <= 64
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}
fn valid_color_mode(value: &str) -> bool {
    matches!(
        value,
        "onoff"
            | "brightness"
            | "color_temp"
            | "hs"
            | "xy"
            | "rgb"
            | "rgbw"
            | "rgbww"
            | "white"
            | "unknown"
    )
}
fn valid_optional_color(value: Option<&String>) -> bool {
    value.is_none_or(|mode| valid_color_mode(mode))
}
fn valid_point(point: Option<&[f64; 2]>, first_max: f64, second_max: f64) -> bool {
    point.is_none_or(|point| {
        point[0].is_finite()
            && point[1].is_finite()
            && (0.0..=first_max).contains(&point[0])
            && (0.0..=second_max).contains(&point[1])
    })
}
fn validate_light(light: &Light) -> bool {
    valid_entity_id(&light.entity_id)
        && light.aliases.len() <= 512
        && light.aliases.iter().all(|alias| valid_alias(alias))
        && light.name.as_ref().is_none_or(|name| name.len() <= 256)
        && valid_timestamp(&light.observed_at)
        && [
            light.last_changed.as_ref(),
            light.last_updated.as_ref(),
            light.last_reported.as_ref(),
        ]
        .into_iter()
        .all(|time| time.is_none_or(|time| valid_timestamp(time)))
        && light
            .freshness
            .age_seconds
            .is_none_or(|age| age.is_finite())
        && light.freshness.stale_after_seconds == 300
        && light.capabilities.on_off
        && light
            .capabilities
            .supported_color_modes
            .as_ref()
            .is_none_or(|modes| {
                modes.len() <= 10 && modes.iter().all(|mode| valid_color_mode(mode))
            })
        && valid_optional_color(light.attributes.color_mode.as_ref())
        && valid_point(light.attributes.hs_color.as_ref(), 360.0, 100.0)
        && valid_point(light.attributes.xy_color.as_ref(), 1.0, 1.0)
        && [
            light.attributes.color_temp_kelvin,
            light.attributes.min_color_temp_kelvin,
            light.attributes.max_color_temp_kelvin,
        ]
        .into_iter()
        .all(|value| value.is_none_or(|value| (1..=100_000).contains(&value)))
        && [
            light.attributes.color_temp,
            light.attributes.min_mireds,
            light.attributes.max_mireds,
        ]
        .into_iter()
        .all(|value| value.is_none_or(|value| (1..=10_000).contains(&value)))
        && light
            .attributes
            .supported_features
            .is_none_or(|value| value <= 2_147_483_647)
}
fn canonical_json(value: &serde_json::Value, output: &mut String) -> Result<(), String> {
    match value {
        serde_json::Value::Null => output.push_str("null"),
        serde_json::Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        serde_json::Value::Number(value) => output.push_str(&value.to_string()),
        serde_json::Value::String(value) => output
            .push_str(&serde_json::to_string(value).map_err(|_| "invalid_assistant_response")?),
        serde_json::Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                canonical_json(value, output)?;
            }
            output.push(']');
        }
        serde_json::Value::Object(values) => {
            output.push('{');
            let mut keys: Vec<_> = values.keys().collect();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(
                    &serde_json::to_string(key).map_err(|_| "invalid_assistant_response")?,
                );
                output.push(':');
                canonical_json(&values[key], output)?;
            }
            output.push('}');
        }
    }
    Ok(())
}
fn action_digest(action: &AssistantAction) -> Result<String, String> {
    let immutable = serde_json::json!({
        "schema_version": action.schema_version,
        "action_id": action.action_id,
        "idempotency_key": action.idempotency_key,
        "kind": action.kind,
        "actor_id": action.actor_id,
        "connection_id": action.connection_id,
        "conversation_id": action.conversation_id,
        "request_id": action.request_id,
        "targets": action.targets,
        "parameters": action.parameters,
        "created_at": action.created_at,
        "expires_at": action.expires_at,
    });
    let mut canonical = String::new();
    canonical_json(&immutable, &mut canonical)?;
    Ok(format!("{:x}", sha2::Sha256::digest(canonical.as_bytes())))
}
fn validate_action(action: &AssistantAction) -> Result<(), String> {
    let target_ids: Vec<_> = action
        .targets
        .iter()
        .map(|target| target.entity_id.as_str())
        .collect();
    let mut sorted_ids = target_ids.clone();
    sorted_ids.sort_unstable();
    let target_set: HashSet<_> = target_ids.iter().copied().collect();
    let valid_parameters = match action.parameters.power.as_str() {
        "off" => {
            action.parameters.brightness_pct.is_none() && action.parameters.rgb_color.is_none()
        }
        "on" => action
            .parameters
            .brightness_pct
            .is_none_or(|value| (1..=100).contains(&value)),
        _ => false,
    };
    let receipt = action.receipt.as_ref();
    let verification = action.verification.as_ref();
    let verification_ids: Vec<_> = verification
        .into_iter()
        .flat_map(|value| &value.states)
        .map(|light| light.entity_id.as_str())
        .collect();
    let mut sorted_verification_ids = verification_ids.clone();
    sorted_verification_ids.sort_unstable();
    let verification_set: HashSet<_> = verification_ids.iter().copied().collect();
    let terminal_shape_valid = match &action.status {
        ActionStatus::Proposed | ActionStatus::Cancelled => {
            receipt.is_none() && verification.is_none()
        }
        ActionStatus::Executing => receipt.is_some_and(|value| value.finished_at.is_none()),
        ActionStatus::Completed => {
            receipt.is_some_and(|value| value.finished_at.is_some() && value.write_attempted)
                && verification.is_some_and(|value| value.matched == Some(true))
                && sorted_verification_ids == target_ids
        }
        ActionStatus::Failed => {
            receipt.is_some_and(|value| value.finished_at.is_some() && !value.write_attempted)
        }
        ActionStatus::Uncertain => receipt.is_some_and(|value| value.finished_at.is_some()),
        ActionStatus::Expired => true,
    };
    let created = timestamp(&action.created_at);
    let expires = timestamp(&action.expires_at);
    if action.schema_version != 1
        || !valid_id(&action.action_id)
        || !valid_digest(&action.idempotency_key)
        || action.kind != "lights.set"
        || action.actor_id.is_empty()
        || action.actor_id.len() > 128
        || action
            .actor_id
            .bytes()
            .any(|byte| !(33..=126).contains(&byte))
        || !valid_id(&action.connection_id)
        || !valid_id(&action.conversation_id)
        || !valid_id(&action.request_id)
        || action.targets.is_empty()
        || action.targets.len() > 16
        || target_ids != sorted_ids
        || target_set.len() != target_ids.len()
        || action.targets.iter().any(|target| {
            !valid_entity_id(&target.entity_id)
                || target.name.as_ref().is_some_and(|name| {
                    name.len() > 256 || name.chars().any(|character| character < ' ')
                })
        })
        || !valid_parameters
        || created.is_none()
        || expires.is_none()
        || expires.zip(created).is_none_or(|(expires, created)| {
            expires.signed_duration_since(created) != chrono::Duration::seconds(300)
        })
        || !valid_digest(&action.parameter_digest)
        || action.receipt.as_ref().is_some_and(|receipt| {
            !valid_id(&receipt.execution_id)
                || !valid_timestamp(&receipt.started_at)
                || receipt
                    .finished_at
                    .as_ref()
                    .is_some_and(|time| !valid_timestamp(time))
        })
        || action.verification.as_ref().is_some_and(|verification| {
            verification.source != "home_assistant_reported"
                || !valid_timestamp(&verification.verified_at)
                || verification.states.len() > 16
                || verification_set.len() != verification_ids.len()
                || verification_ids
                    .iter()
                    .any(|entity_id| !target_set.contains(entity_id))
                || verification
                    .states
                    .iter()
                    .any(|light| !validate_light(light))
        })
        || !terminal_shape_valid
        || action_digest(action)? != action.parameter_digest
    {
        return Err("invalid_assistant_response".into());
    }
    Ok(())
}
fn validate(snapshot: &Snapshot) -> Result<(), String> {
    if snapshot.version != 1
        || snapshot.conversations.len() > MAX_CONVERSATIONS
        || snapshot.connections.len() > MAX_CONNECTIONS
        || snapshot
            .binding
            .as_ref()
            .is_some_and(|binding| !valid_digest(binding))
    {
        return Err(STORAGE_ERROR.into());
    }
    let mut connection_ids = HashSet::new();
    let mut connection_bindings = HashSet::new();
    for connection in &snapshot.connections {
        if !valid_digest(&connection.binding)
            || !valid_id(&connection.connection_id)
            || !connection_ids.insert(connection.connection_id.as_str())
            || !connection_bindings.insert(connection.binding.as_str())
        {
            return Err(STORAGE_ERROR.into());
        }
    }
    if snapshot
        .binding
        .as_ref()
        .is_some_and(|binding| !connection_bindings.contains(binding.as_str()))
    {
        return Err(STORAGE_ERROR.into());
    }
    for c in &snapshot.conversations {
        if !valid_id(&c.id)
            || c.title.len() > 256
            || c.messages.len() > MAX_MESSAGES
            || c.connection_binding
                .as_ref()
                .is_some_and(|binding| !valid_digest(binding))
            || c.connection_id.as_ref().is_some_and(|id| !valid_id(id))
            || c.active_action_id.as_ref().is_some_and(|id| !valid_id(id))
        {
            return Err(STORAGE_ERROR.into());
        }
        for m in &c.messages {
            if !valid_id(&m.id)
                || !valid_id(&m.request_id)
                || !matches!(m.role.as_str(), "user" | "assistant")
                || m.content.len() > 256 * 1024
                || !matches!(
                    m.status.as_str(),
                    "pending" | "running" | "ready" | "failed" | "cancelled" | "interrupted"
                )
                || m.error_code.as_ref().is_some_and(|e| {
                    e.len() > 64 || !e.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                })
                || m.actions.len() > MAX_ACTIONS_PER_MESSAGE
                || (m.role == "user" && !m.actions.is_empty())
                || m.actions.iter().any(|action| {
                    validate_action(action).is_err()
                        || action.conversation_id != c.id
                        || action.request_id != m.request_id
                        || c.connection_id.as_deref() != Some(action.connection_id.as_str())
                })
            {
                return Err(STORAGE_ERROR.into());
            }
        }
    }
    Ok(())
}

impl AssistantStore {
    pub(crate) fn initialize(
        &self,
        root: PathBuf,
        app: Option<tauri::AppHandle>,
    ) -> Result<(), String> {
        if fs::symlink_metadata(&root).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(STORAGE_ERROR.into());
        }
        fs::create_dir_all(&root).map_err(|_| STORAGE_ERROR)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
                .map_err(|_| STORAGE_ERROR)?;
        }
        let path = root.join("conversations-v1.json");
        let mut snapshot = match fs::symlink_metadata(&path) {
            Ok(m) => {
                if !m.is_file() || m.file_type().is_symlink() || m.len() > MAX_STORE as u64 {
                    return Err(STORAGE_ERROR.into());
                }
                let bytes = fs::read(&path).map_err(|_| STORAGE_ERROR)?;
                serde_json::from_slice::<Snapshot>(&bytes).map_err(|_| STORAGE_ERROR)?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Snapshot::default(),
            Err(_) => return Err(STORAGE_ERROR.into()),
        };
        let mut legacy_bindings = Vec::new();
        if let Some(binding) = snapshot.binding.as_ref() {
            legacy_bindings.push(binding.clone());
        }
        legacy_bindings.extend(
            snapshot
                .conversations
                .iter()
                .filter_map(|conversation| conversation.connection_binding.clone()),
        );
        legacy_bindings.sort();
        legacy_bindings.dedup();
        for binding in legacy_bindings {
            if !snapshot
                .connections
                .iter()
                .any(|connection| connection.binding == binding)
            {
                snapshot.connections.push(StoredConnection {
                    binding,
                    connection_id: id(),
                });
            }
        }
        for conversation in &mut snapshot.conversations {
            if conversation.connection_id.is_none() {
                conversation.connection_id =
                    conversation
                        .connection_binding
                        .as_ref()
                        .and_then(|binding| {
                            snapshot
                                .connections
                                .iter()
                                .find(|connection| &connection.binding == binding)
                                .map(|connection| connection.connection_id.clone())
                        });
            }
        }
        validate(&snapshot)?;
        for c in &mut snapshot.conversations {
            c.active_pass_id = None;
            c.active_action_id = None;
            for m in &mut c.messages {
                if matches!(m.status.as_str(), "pending" | "running") {
                    m.status = "interrupted".into();
                    m.error_code = Some("interrupted".into());
                }
            }
        }
        let mut inner = self.0.lock_or_recover();
        inner.root = Some(root);
        inner.app = app;
        Self::write(&inner, &snapshot)?;
        inner.snapshot = snapshot;
        Ok(())
    }
    fn write(inner: &Inner, snapshot: &Snapshot) -> Result<(), String> {
        validate(snapshot)?;
        let root = inner.root.as_ref().ok_or(STORAGE_ERROR)?;
        let bytes = serde_json::to_vec(snapshot).map_err(|_| STORAGE_ERROR)?;
        // Reserve one maximum answer before admitting a request, so streaming can finish.
        if bytes.len() > MAX_STORE {
            return Err("assistant_history_full".into());
        }
        let temp = root.join(format!(".{}.tmp", id()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let result = (|| {
            let mut file = options.open(&temp).map_err(|_| STORAGE_ERROR)?;
            file.write_all(&bytes).map_err(|_| STORAGE_ERROR)?;
            file.sync_all().map_err(|_| STORAGE_ERROR)?;
            fs::rename(&temp, root.join("conversations-v1.json")).map_err(|_| STORAGE_ERROR)?;
            fs::File::open(root)
                .and_then(|d| d.sync_all())
                .map_err(|_| STORAGE_ERROR)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
    fn mutate<T>(
        &self,
        change: impl FnOnce(&mut Snapshot) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut inner = self.0.lock_or_recover();
        let mut next = inner.snapshot.clone();
        let result = change(&mut next)?;
        Self::write(&inner, &next)?;
        inner.snapshot = next;
        Ok(result)
    }
    pub(crate) fn connect(&self, binding: Option<String>) -> Result<(), String> {
        self.mutate(|s| {
            if let Some(binding) = binding.as_ref() {
                if !valid_digest(binding) {
                    return Err("assistant_connection_changed".into());
                }
                if !s
                    .connections
                    .iter()
                    .any(|connection| &connection.binding == binding)
                {
                    if s.connections.len() >= MAX_CONNECTIONS {
                        return Err("assistant_history_full".into());
                    }
                    s.connections.push(StoredConnection {
                        binding: binding.clone(),
                        connection_id: id(),
                    });
                }
            }
            s.binding = binding;
            Ok(())
        })
    }
    pub(crate) fn connected(&self, binding: &str) -> bool {
        self.0.lock_or_recover().snapshot.binding.as_deref() == Some(binding)
    }
    pub(crate) fn current_binding(&self) -> Option<String> {
        self.0.lock_or_recover().snapshot.binding.clone()
    }
    pub(crate) fn list(&self) -> Result<Vec<ConversationSummary>, String> {
        let inner = self.0.lock_or_recover();
        inner.root.as_ref().ok_or(STORAGE_ERROR)?;
        let mut items: Vec<_> = inner
            .snapshot
            .conversations
            .iter()
            .map(|c| ConversationSummary {
                id: c.id.clone(),
                title: c.title.clone(),
                created_at_ms: c.created_at_ms,
                updated_at_ms: c.updated_at_ms,
                message_count: c.messages.len(),
            })
            .collect();
        items.sort_by_key(|c| std::cmp::Reverse(c.updated_at_ms));
        Ok(items)
    }
    pub(crate) fn get(&self, conversation_id: &str) -> Result<Conversation, String> {
        self.0
            .lock_or_recover()
            .snapshot
            .conversations
            .iter()
            .find(|c| c.id == conversation_id)
            .cloned()
            .ok_or_else(|| "assistant_conversation_unavailable".into())
    }
    pub(crate) fn create(&self, prior: Option<(String, String)>) -> Result<Conversation, String> {
        self.create_checked(prior, None)
    }

    pub(crate) fn import(
        &self,
        question: String,
        answer: String,
        expected_binding: &str,
    ) -> Result<Conversation, String> {
        self.create_checked(Some((question, answer)), Some(expected_binding))
    }

    fn create_checked(
        &self,
        prior: Option<(String, String)>,
        expected_binding: Option<&str>,
    ) -> Result<Conversation, String> {
        self.mutate(|s| {
            if s.binding.is_none() {
                return Err("assistant_not_connected".into());
            }
            if expected_binding.is_some_and(|expected| s.binding.as_deref() != Some(expected)) {
                return Err("assistant_connection_changed".into());
            }
            if s.conversations.len() >= MAX_CONVERSATIONS {
                return Err("assistant_history_full".into());
            }
            let timestamp = now();
            let connection_id = s
                .binding
                .as_ref()
                .and_then(|binding| {
                    s.connections
                        .iter()
                        .find(|connection| &connection.binding == binding)
                })
                .map(|connection| connection.connection_id.clone())
                .ok_or("assistant_not_connected")?;
            let mut c = Conversation {
                id: id(),
                title: "New conversation".into(),
                created_at_ms: timestamp,
                updated_at_ms: timestamp,
                messages: vec![],
                active_pass_id: None,
                live_state: None,
                connection_id: Some(connection_id),
                active_action_id: None,
                seed_pending: prior.is_some(),
                connection_binding: s.binding.clone(),
            };
            if let Some((question, answer)) = prior {
                c.title = question.chars().take(60).collect();
                let request_id = id();
                for (role, content) in [("user", question), ("assistant", answer)] {
                    c.messages.push(Message {
                        id: id(),
                        request_id: request_id.clone(),
                        role: role.into(),
                        content,
                        created_at_ms: timestamp,
                        status: "ready".into(),
                        error_code: None,
                        actions: vec![],
                    });
                }
                // Validate seed bound before saving an unusable import.
                envelope(&c, &id(), "")?;
            }
            s.conversations.push(c.clone());
            Ok(c)
        })
    }
    pub(crate) fn delete(&self, conversation_id: &str) -> Result<(), String> {
        self.mutate(|s| {
            let c = s
                .conversations
                .iter()
                .find(|c| c.id == conversation_id)
                .ok_or("assistant_conversation_unavailable")?;
            if c.active_pass_id.is_some() {
                return Err("busy".into());
            }
            s.conversations.retain(|c| c.id != conversation_id);
            Ok(())
        })
    }
    /// Read-only admission for local composer capture. No message, request,
    /// activity marker, timestamp, or durable write is created until Send.
    pub(crate) fn validate_draft(
        &self,
        conversation_id: &str,
        binding: &str,
    ) -> Result<(), String> {
        let inner = self.0.lock_or_recover();
        if inner.root.is_none() {
            return Err(STORAGE_ERROR.into());
        }
        if inner.snapshot.binding.as_deref() != Some(binding) {
            return Err("assistant_connection_changed".into());
        }
        let conversation = inner
            .snapshot
            .conversations
            .iter()
            .find(|c| c.id == conversation_id)
            .ok_or("assistant_conversation_unavailable")?;
        if conversation.connection_binding.as_deref() != Some(binding) {
            return Err("assistant_connection_changed".into());
        }
        let current_connection = inner
            .snapshot
            .connections
            .iter()
            .find(|connection| connection.binding == binding)
            .map(|connection| connection.connection_id.as_str());
        if conversation.connection_id.as_deref() != current_connection {
            return Err("assistant_connection_changed".into());
        }
        if conversation.active_pass_id.is_some() {
            return Err("busy".into());
        }
        Ok(())
    }

    pub(crate) fn begin(
        &self,
        conversation_id: &str,
        pass_id: u64,
        message: &str,
        expected_binding: &str,
    ) -> Result<String, String> {
        self.mutate(|s| {
            // Queue admission is the consent boundary. Compare the frozen
            // dispatch command to both bindings under this one store lock;
            // a reconnect after command validation cannot redirect a turn.
            if s.binding.as_deref() != Some(expected_binding) {
                return Err("assistant_connection_changed".into());
            }
            if serde_json::to_vec(s).map_err(|_| STORAGE_ERROR)?.len() > MAX_STORE - 2 * 1024 * 1024
            {
                return Err("assistant_history_full".into());
            }
            let current_connection = s
                .connections
                .iter()
                .find(|connection| connection.binding == expected_binding)
                .map(|connection| connection.connection_id.clone());
            let c = s
                .conversations
                .iter_mut()
                .find(|c| c.id == conversation_id)
                .ok_or("assistant_conversation_unavailable")?;
            if c.connection_binding.as_deref() != Some(expected_binding) {
                return Err("assistant_connection_changed".into());
            }
            if c.connection_id.as_ref() != current_connection.as_ref() {
                return Err("assistant_connection_changed".into());
            }
            if c.active_pass_id.is_some() || c.messages.len() + 2 > MAX_MESSAGES {
                return Err("assistant_history_full".into());
            }
            let request_id = id();
            envelope(c, &request_id, message)?;
            let timestamp = now();
            for (role, content) in [("user", message), ("assistant", "")] {
                c.messages.push(Message {
                    id: id(),
                    request_id: request_id.clone(),
                    role: role.into(),
                    content: content.into(),
                    created_at_ms: timestamp,
                    status: "pending".into(),
                    error_code: None,
                    actions: vec![],
                });
            }
            if c.messages.len() == 2 && !message.is_empty() {
                c.title = message.chars().take(60).collect();
            }
            c.active_pass_id = Some(pass_id);
            c.updated_at_ms = timestamp;
            Ok(request_id)
        })
    }
    pub(crate) fn for_pass(&self, pass_id: u64) -> Option<String> {
        self.0
            .lock_or_recover()
            .snapshot
            .conversations
            .iter()
            .find(|c| c.active_pass_id == Some(pass_id))
            .map(|c| c.id.clone())
    }
    pub(crate) fn prompt(&self, pass_id: u64, message: &str) -> Result<String, String> {
        self.mutate(|s| {
            let c = s
                .conversations
                .iter_mut()
                .find(|c| c.active_pass_id == Some(pass_id))
                .ok_or("cancelled")?;
            let request = c.messages.last().ok_or(STORAGE_ERROR)?.request_id.clone();
            let prompt = envelope(c, &request, message)?;
            let user = c
                .messages
                .iter_mut()
                .rev()
                .find(|m| m.role == "user")
                .ok_or(STORAGE_ERROR)?;
            user.content = message.into();
            user.status = "ready".into();
            if c.messages.len() == 2 {
                c.title = message.chars().take(60).collect();
            }
            c.messages.last_mut().unwrap().status = "running".into();
            Ok(prompt)
        })
    }
    pub(crate) fn begin_action(
        &self,
        action_id: &str,
        pass_id: u64,
        operation: &str,
    ) -> Result<ActionDispatch, String> {
        if !valid_id(action_id) || !matches!(operation, "confirm" | "cancel" | "status") {
            return Err("invalid_assistant_action".into());
        }
        self.mutate(|snapshot| {
            let binding = snapshot
                .binding
                .as_ref()
                .ok_or("assistant_not_connected")?
                .clone();
            let current_connection = snapshot
                .connections
                .iter()
                .find(|connection| connection.binding == binding)
                .map(|connection| connection.connection_id.clone())
                .ok_or("assistant_not_connected")?;
            let conversation = snapshot
                .conversations
                .iter_mut()
                .find(|conversation| {
                    conversation.messages.iter().any(|message| {
                        message
                            .actions
                            .iter()
                            .any(|action| action.action_id == action_id)
                    })
                })
                .ok_or("assistant_action_unavailable")?;
            if conversation.connection_binding.as_deref() != Some(binding.as_str())
                || conversation.connection_id.as_deref() != Some(current_connection.as_str())
                || conversation.active_pass_id.is_some()
            {
                return Err(if conversation.active_pass_id.is_some() {
                    "busy"
                } else {
                    "assistant_connection_changed"
                }
                .into());
            }
            let action = conversation
                .messages
                .iter()
                .flat_map(|message| &message.actions)
                .find(|action| action.action_id == action_id)
                .ok_or("assistant_action_unavailable")?;
            validate_action(action)?;
            if action.connection_id != current_connection
                || action.conversation_id != conversation.id
            {
                return Err("assistant_connection_changed".into());
            }
            let dispatch = ActionDispatch {
                conversation_id: conversation.id.clone(),
                request_id: id(),
                connection_id: current_connection,
                parameter_digest: action.parameter_digest.clone(),
            };
            conversation.active_pass_id = Some(pass_id);
            conversation.active_action_id = Some(action_id.to_string());
            conversation.updated_at_ms = now();
            Ok(dispatch)
        })
    }

    pub(crate) fn record_actions(
        &self,
        pass_id: u64,
        actions: Vec<AssistantAction>,
    ) -> Result<(), String> {
        if actions.is_empty() || actions.len() > MAX_ACTIONS_PER_MESSAGE {
            return Err("invalid_assistant_response".into());
        }
        for action in &actions {
            validate_action(action)?;
        }
        let conversation_id = self.mutate(|snapshot| {
            let conversation = snapshot
                .conversations
                .iter_mut()
                .find(|conversation| conversation.active_pass_id == Some(pass_id))
                .ok_or("cancelled")?;
            for action in actions {
                if conversation.connection_id.as_deref() != Some(action.connection_id.as_str())
                    || conversation.id != action.conversation_id
                {
                    return Err("invalid_assistant_response".into());
                }
                if conversation
                    .active_action_id
                    .as_ref()
                    .is_some_and(|expected| expected != &action.action_id)
                {
                    return Err("invalid_assistant_response".into());
                }
                let existing_location = conversation.messages.iter().enumerate().find_map(
                    |(message_index, message)| {
                        message
                            .actions
                            .iter()
                            .position(|existing| existing.action_id == action.action_id)
                            .map(|action_index| (message_index, action_index))
                    },
                );
                if let Some((message_index, action_index)) = existing_location {
                    let existing = &mut conversation.messages[message_index].actions[action_index];
                    if existing.parameter_digest != action.parameter_digest {
                        return Err("invalid_assistant_response".into());
                    }
                    *existing = action;
                    continue;
                }
                if conversation.active_action_id.is_some() {
                    return Err("invalid_assistant_response".into());
                }
                let request_id = conversation
                    .messages
                    .last()
                    .map(|message| message.request_id.as_str())
                    .ok_or(STORAGE_ERROR)?;
                if request_id != action.request_id {
                    return Err("invalid_assistant_response".into());
                }
                let message = conversation.messages.last_mut().ok_or(STORAGE_ERROR)?;
                if message.role != "assistant" || message.actions.len() >= MAX_ACTIONS_PER_MESSAGE {
                    return Err("output_too_large".into());
                }
                message.actions.push(action);
            }
            conversation.updated_at_ms = now();
            Ok(conversation.id.clone())
        })?;
        self.emit_changed(&conversation_id, pass_id);
        Ok(())
    }

    #[cfg(test)]
    fn record_action(&self, pass_id: u64, action: AssistantAction) -> Result<(), String> {
        self.record_actions(pass_id, vec![action])
    }
    pub(crate) fn update(
        &self,
        pass_id: u64,
        answer: &str,
        terminal: Option<(&str, Option<&str>)>,
    ) -> Result<(), String> {
        let mut inner = self.0.lock_or_recover();
        let mut next = inner.snapshot.clone();
        let conversation_id = {
            let Some(c) = next
                .conversations
                .iter_mut()
                .find(|c| c.active_pass_id == Some(pass_id))
            else {
                return Ok(());
            };
            if c.active_action_id.is_some() {
                if terminal.is_some() {
                    c.active_pass_id = None;
                    c.active_action_id = None;
                }
                c.updated_at_ms = now();
                c.id.clone()
            } else {
                let m = c.messages.last_mut().ok_or(STORAGE_ERROR)?;
                m.content = answer.into();
                if let Some((status, error)) = terminal {
                    m.status = status.into();
                    m.error_code = error.map(str::to_string);
                    c.active_pass_id = None;
                    if status == "ready" {
                        c.seed_pending = false;
                    }
                    for user in c.messages.iter_mut().filter(|m| m.status == "pending") {
                        user.status = status.into();
                    }
                } else {
                    m.status = "running".into();
                }
                c.updated_at_ms = now();
                c.id.clone()
            }
        };
        let result = Self::write(&inner, &next);
        // A disk failure must stop dispatch, but cannot leave an already
        // terminated process as an un-cancellable active UI owner. Retain the
        // bounded latest content in memory and expose the storage failure.
        if result.is_err() && terminal.is_some() {
            if let Some(message) = next
                .conversations
                .iter_mut()
                .find(|c| c.id == conversation_id)
                .and_then(|c| c.messages.last_mut())
            {
                message.status = "failed".into();
                message.error_code = Some(STORAGE_ERROR.into());
            }
        }
        inner.snapshot = next;
        drop(inner);
        self.emit_changed(&conversation_id, pass_id);
        result
    }
    pub(crate) fn emit_changed(&self, conversation_id: &str, pass_id: u64) {
        let app = self.0.lock_or_recover().app.clone();
        if let Some(app) = app {
            let _ = app.emit_to(
                "main",
                "assistant-conversation-changed",
                serde_json::json!({"conversationId":conversation_id,"queryPassId":pass_id}),
            );
        }
    }
}

fn envelope(c: &Conversation, request_id: &str, message: &str) -> Result<String, String> {
    if message.len() > 32768 || message.contains('\0') {
        return Err("query_too_large".into());
    }
    let connection_id = c
        .connection_id
        .as_ref()
        .filter(|connection_id| valid_id(connection_id))
        .ok_or("assistant_connection_changed")?;
    let mut payload = serde_json::json!({
        "type": "message", "conversationId": c.id, "requestId": request_id,
        "message": message, "connectionId": connection_id,
    });
    if c.seed_pending {
        payload["seedMessages"] = serde_json::json!(c
            .messages
            .iter()
            .take(2)
            .map(|m| serde_json::json!({"role":m.role,"content":m.content}))
            .collect::<Vec<_>>());
    }
    let prompt = format!("MURMUR_ASSISTANT_V2\n{}", payload);
    if prompt.len() > 65536 {
        return Err("query_too_large".into());
    }
    Ok(prompt)
}

pub(crate) fn action_envelope(
    operation: &str,
    action_id: &str,
    dispatch: &ActionDispatch,
) -> Result<String, String> {
    if !matches!(operation, "confirm" | "cancel" | "status")
        || !valid_id(action_id)
        || !valid_id(&dispatch.conversation_id)
        || !valid_id(&dispatch.request_id)
        || !valid_id(&dispatch.connection_id)
        || !valid_digest(&dispatch.parameter_digest)
    {
        return Err("invalid_assistant_action".into());
    }
    let payload = serde_json::json!({
        "type": operation,
        "conversationId": dispatch.conversation_id,
        "requestId": dispatch.request_id,
        "connectionId": dispatch.connection_id,
        "actionId": action_id,
        "parameterDigest": dispatch.parameter_digest,
    });
    let prompt = format!("MURMUR_ASSISTANT_V2\n{payload}");
    (prompt.len() <= 65536)
        .then_some(prompt)
        .ok_or_else(|| "query_too_large".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    const BINDING: &str = "0000000000000000000000000000000000000000000000000000000000000000";
    const OTHER_BINDING: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    fn store(root: PathBuf) -> AssistantStore {
        let s = AssistantStore::default();
        s.initialize(root, None).unwrap();
        s.connect(Some(BINDING.into())).unwrap();
        s
    }
    #[test]
    fn draft_admission_is_read_only_and_enforces_connection_and_conversation_binding() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("assistant");
        let s = store(root.clone());
        let c = s.create(None).unwrap();
        let path = root.join("conversations-v1.json");
        let before = fs::read(&path).unwrap();
        assert!(s.validate_draft(&c.id, BINDING).is_ok());
        assert_eq!(fs::read(&path).unwrap(), before);
        let unchanged = s.get(&c.id).unwrap();
        assert!(unchanged.messages.is_empty());
        assert_eq!(unchanged.active_pass_id, None);
        assert_eq!(unchanged.updated_at_ms, c.updated_at_ms);
        assert!(s.for_pass(42).is_none());
        assert!(s.prompt(42, "unsent draft").is_err());
        assert!(s.validate_draft("missing", BINDING).is_err());
        assert!(s.validate_draft(&c.id, OTHER_BINDING).is_err());
        s.connect(Some(OTHER_BINDING.into())).unwrap();
        assert!(s.validate_draft(&c.id, OTHER_BINDING).is_err());
        s.connect(None).unwrap();
        assert!(s.validate_draft(&c.id, BINDING).is_err());
    }
    #[test]
    fn restart_preserves_partial_and_terminalizes_without_redispatch() {
        let temp = tempfile::tempdir().unwrap();
        let s = store(temp.path().join("assistant"));
        let c = s.create(None).unwrap();
        s.begin(&c.id, 7, "question", BINDING).unwrap();
        s.prompt(7, "question").unwrap();
        s.update(7, "partial", None).unwrap();
        let restored = store(temp.path().join("assistant"));
        let c = restored.get(&c.id).unwrap();
        assert_eq!(c.active_pass_id, None);
        assert_eq!(c.messages[1].content, "partial");
        assert_eq!(c.messages[1].status, "interrupted");
    }
    #[test]
    fn stale_update_delete_and_exact_pass_are_fenced() {
        let temp = tempfile::tempdir().unwrap();
        let s = store(temp.path().join("assistant"));
        let c = s.create(None).unwrap();
        s.begin(&c.id, 7, "question", BINDING).unwrap();
        assert!(s.delete(&c.id).is_err());
        s.update(6, "wrong", None).unwrap();
        assert_eq!(s.get(&c.id).unwrap().messages[1].content, "");
        s.update(7, "partial", Some(("cancelled", Some("cancelled"))))
            .unwrap();
        s.delete(&c.id).unwrap();
        s.update(7, "late", None).unwrap();
        assert!(s.list().unwrap().is_empty());
    }
    #[test]
    fn consent_is_separate_and_seed_is_bounded_and_only_first_dispatch() {
        let temp = tempfile::tempdir().unwrap();
        let s = store(temp.path().join("assistant"));
        assert!(!s.connected("another bridge"));
        let c = s
            .create(Some(("question".into(), "answer".into())))
            .unwrap();
        s.begin(&c.id, 8, "next", BINDING).unwrap();
        let prompt = s.prompt(8, "next").unwrap();
        assert!(prompt.contains("seedMessages"));
        assert!(prompt.starts_with("MURMUR_ASSISTANT_V2\n"));
        s.update(8, "reply", Some(("ready", None))).unwrap();
        s.begin(&c.id, 9, "later", BINDING).unwrap();
        assert!(!s.prompt(9, "later").unwrap().contains("seedMessages"));
        assert!(s
            .create(Some(("question".into(), "x".repeat(65536))))
            .is_err());
        s.connect(None).unwrap();
        assert!(s.create(None).is_err());
    }
    #[test]
    fn rejects_symlink_and_future_store_without_overwriting() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("assistant");
        fs::create_dir(&root).unwrap();
        let path = root.join("conversations-v1.json");
        fs::write(
            &path,
            br#"{"version":99,"binding":null,"conversations":[]}"#,
        )
        .unwrap();
        assert!(AssistantStore::default()
            .initialize(root.clone(), None)
            .is_err());
        assert!(fs::read_to_string(&path).unwrap().contains("99"));
        #[cfg(unix)]
        {
            fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(temp.path().join("outside"), &path).unwrap();
            assert!(AssistantStore::default().initialize(root, None).is_err());
        }
    }

    #[test]
    fn failed_import_dispatch_keeps_exact_seed_for_next_request() {
        let temp = tempfile::tempdir().unwrap();
        let s = store(temp.path().join("assistant"));
        let c = s
            .create(Some(("seed question".into(), "seed answer".into())))
            .unwrap();
        s.begin(&c.id, 1, "first", BINDING).unwrap();
        let first = s.prompt(1, "first").unwrap();
        s.update(1, "partial", Some(("failed", Some("exit_nonzero"))))
            .unwrap();
        s.begin(&c.id, 2, "second", BINDING).unwrap();
        let second = s.prompt(2, "second").unwrap();
        let first: serde_json::Value =
            serde_json::from_str(first.split_once('\n').unwrap().1).unwrap();
        let second: serde_json::Value =
            serde_json::from_str(second.split_once('\n').unwrap().1).unwrap();
        assert_eq!(first["seedMessages"], second["seedMessages"]);
        assert_ne!(first["requestId"], second["requestId"]);
        assert_eq!(second["seedMessages"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn failed_terminal_write_releases_memory_owner_and_reports_storage_error() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("assistant");
        let s = store(root.clone());
        let c = s.create(None).unwrap();
        s.begin(&c.id, 1, "question", BINDING).unwrap();
        s.prompt(1, "question").unwrap();
        fs::rename(&root, temp.path().join("moved")).unwrap();
        assert!(s
            .update(1, "partial", Some(("cancelled", Some("cancelled"))))
            .is_err());
        let c = s.get(&c.id).unwrap();
        assert_eq!(c.active_pass_id, None);
        assert_eq!(c.messages[1].content, "partial");
        assert_eq!(c.messages[1].error_code.as_deref(), Some(STORAGE_ERROR));
    }

    #[test]
    fn rejects_oversized_turn_without_queueing_and_uses_private_modes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("assistant");
        let s = store(root.clone());
        let c = s.create(None).unwrap();
        assert!(s.begin(&c.id, 1, &"x".repeat(32769), BINDING).is_err());
        assert!(s.get(&c.id).unwrap().messages.is_empty());
        assert!(s.get("../../outside").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&root).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(root.join("conversations-v1.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn reconnecting_another_bridge_cannot_route_an_existing_conversation() {
        let temp = tempfile::tempdir().unwrap();
        let s = store(temp.path().join("assistant"));
        let c = s.create(None).unwrap();
        s.connect(Some(OTHER_BINDING.into())).unwrap();
        assert_eq!(
            s.begin(&c.id, 1, "new message", OTHER_BINDING).unwrap_err(),
            "assistant_connection_changed"
        );
        assert!(s.get(&c.id).unwrap().messages.is_empty());
        s.connect(Some(BINDING.into())).unwrap();
        assert!(s.begin(&c.id, 2, "new message", BINDING).is_ok());
    }

    #[test]
    fn reconnect_between_validation_and_admission_rejects_frozen_command() {
        let temp = tempfile::tempdir().unwrap();
        let s = store(temp.path().join("assistant"));
        let original = s.create(None).unwrap();
        s.connect(Some(OTHER_BINDING.into())).unwrap();
        let other = s.create(None).unwrap();
        let frozen_binding = OTHER_BINDING;
        assert!(s.connected(frozen_binding));

        // Another command reconnects to the target conversation's original
        // bridge after the dispatch command was validated. Global and
        // conversation bindings now agree, but the frozen command does not.
        s.connect(Some(BINDING.into())).unwrap();
        assert_eq!(
            s.begin(&original.id, 1, "private message", frozen_binding)
                .unwrap_err(),
            "assistant_connection_changed"
        );
        assert!(s.get(&original.id).unwrap().messages.is_empty());
        assert_eq!(s.get(&original.id).unwrap().active_pass_id, None);
        assert!(s.get(&other.id).unwrap().messages.is_empty());

        // Revocation in the same interval also rejects queue admission.
        s.connect(None).unwrap();
        assert!(s
            .begin(&other.id, 2, "private message", frozen_binding)
            .is_err());
    }

    #[test]
    fn reconnect_between_popover_validation_and_import_cannot_rebind_private_seed() {
        let temp = tempfile::tempdir().unwrap();
        let s = store(temp.path().join("assistant"));
        assert!(s.connected(BINDING));
        s.connect(Some(OTHER_BINDING.into())).unwrap();
        assert!(
            matches!(s.import("private question".into(), "private answer".into(), BINDING), Err(error) if error == "assistant_connection_changed")
        );
        assert!(s.list().unwrap().is_empty());
        s.connect(Some(BINDING.into())).unwrap();
        assert!(s
            .import("private question".into(), "private answer".into(), BINDING)
            .is_ok());
    }

    fn action_fixture(name: &str) -> AssistantAction {
        // Copied from AgentOS backend/capabilities/tests/fixtures/action_responses.json
        // at the frozen 2026-09-14 action contract revision.
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/assistant-action-responses.json"
        ))
        .unwrap();
        serde_json::from_value(fixture[name]["result"].clone()).unwrap()
    }

    #[test]
    fn v2_parser_accepts_agentos_proposed_completed_and_uncertain_records() {
        for name in ["proposed", "completed", "uncertain"] {
            let action = action_fixture(name);
            let line = serde_json::to_vec(&serde_json::json!({
                "schema_version": 2, "type": "action", "action": action,
            }))
            .unwrap();
            let mut parser = AssistantStreamParser::new();
            let split = line.len() / 2;
            assert!(parser.push(&line[..split]).unwrap().is_empty());
            let mut second = line[split..].to_vec();
            second.extend_from_slice(b"\n{\"schema_version\":2,\"type\":\"done\"}\n");
            let updates = parser.push(&second).unwrap();
            assert!(matches!(
                updates.as_slice(),
                [AssistantStreamUpdate::Action(_)]
            ));
            let (tail, count) = parser.finish().unwrap();
            assert!(tail.is_empty());
            assert_eq!(count, 1);
        }
    }

    #[test]
    fn action_validation_matches_agentos_light_and_terminal_bounds() {
        let mut completed = action_fixture("completed");
        completed.verification.as_mut().unwrap().states[0]
            .capabilities
            .supported_color_modes = Some(vec!["unknown".into()]);
        completed.verification.as_mut().unwrap().states[0]
            .attributes
            .color_mode = Some("unknown".into());
        assert!(validate_action(&completed).is_ok());

        let mut missing_observation = serde_json::to_value(&completed).unwrap();
        missing_observation["verification"]["states"][0]["observed_at"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<AssistantAction>(missing_observation).is_err());

        let mut bad_alias = completed.clone();
        bad_alias.verification.as_mut().unwrap().states[0].aliases = vec!["Bad alias".into()];
        assert!(validate_action(&bad_alias).is_err());

        let mut too_many_features = completed.clone();
        too_many_features.verification.as_mut().unwrap().states[0]
            .attributes
            .supported_features = Some(2_147_483_648);
        assert!(validate_action(&too_many_features).is_err());

        let mut impossible_failure = completed;
        impossible_failure.status = ActionStatus::Failed;
        impossible_failure.verification = None;
        assert!(validate_action(&impossible_failure).is_err());
    }

    #[test]
    fn v2_parser_rejects_prose_bad_digest_duplicate_done_and_missing_done() {
        let mut prose = AssistantStreamParser::new();
        assert!(matches!(
            prose.push(b"please confirm\n"),
            Err("invalid_assistant_response")
        ));

        let mut value = serde_json::to_value(action_fixture("proposed")).unwrap();
        value["parameter_digest"] = serde_json::json!("0".repeat(64));
        let bad = format!(
            "{}\n",
            serde_json::json!({"schema_version":2,"type":"action","action":value})
        );
        assert!(matches!(
            AssistantStreamParser::new().push(bad.as_bytes()),
            Err("invalid_assistant_response")
        ));

        let mut duplicate = AssistantStreamParser::new();
        duplicate
            .push(b"{\"schema_version\":2,\"type\":\"done\"}\n")
            .unwrap();
        assert!(matches!(
            duplicate.push(b"{\"schema_version\":2,\"type\":\"done\"}\n"),
            Err("invalid_assistant_response")
        ));
        assert!(matches!(
            AssistantStreamParser::new().finish(),
            Err("invalid_assistant_response")
        ));
    }

    #[test]
    fn stored_action_dispatch_uses_only_bound_ids_and_digest() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("assistant");
        let s = store(root.clone());
        let conversation = s.create(None).unwrap();
        let request_id = s
            .begin(&conversation.id, 7, "make it blue", BINDING)
            .unwrap();
        let mut action = action_fixture("proposed");
        action.conversation_id = conversation.id.clone();
        action.connection_id = conversation.connection_id.clone().unwrap();
        action.request_id = request_id;
        action.parameter_digest = action_digest(&action).unwrap();
        let action_id = action.action_id.clone();
        s.record_action(7, action).unwrap();
        s.update(7, "", Some(("ready", None))).unwrap();

        let restored = store(root);
        let dispatch = restored.begin_action(&action_id, 8, "confirm").unwrap();
        let prompt = action_envelope("confirm", &action_id, &dispatch).unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(prompt.split_once('\n').unwrap().1).unwrap();
        assert_eq!(payload["actionId"], action_id);
        assert_eq!(payload["parameterDigest"], dispatch.parameter_digest);
        assert!(payload.get("targets").is_none());
        assert!(payload.get("parameters").is_none());
        assert_eq!(payload.as_object().unwrap().len(), 6);

        let mut completed = action_fixture("completed");
        completed.conversation_id = conversation.id;
        completed.connection_id = dispatch.connection_id;
        completed.request_id = s
            .get(&dispatch.conversation_id)
            .unwrap()
            .messages
            .last()
            .unwrap()
            .request_id
            .clone();
        completed.parameter_digest = action_digest(&completed).unwrap();
        restored.record_action(8, completed).unwrap();
        assert_eq!(
            restored.get(&dispatch.conversation_id).unwrap().messages[1].actions[0].status,
            ActionStatus::Completed
        );
    }

    #[test]
    fn action_status_can_update_an_old_proposal_after_a_later_message() {
        let temp = tempfile::tempdir().unwrap();
        let s = store(temp.path().join("assistant"));
        let conversation = s.create(None).unwrap();
        let original_request = s.begin(&conversation.id, 7, "lights", BINDING).unwrap();
        let mut proposed = action_fixture("proposed");
        proposed.conversation_id = conversation.id.clone();
        proposed.connection_id = conversation.connection_id.clone().unwrap();
        proposed.request_id = original_request.clone();
        proposed.parameter_digest = action_digest(&proposed).unwrap();
        let action_id = proposed.action_id.clone();
        s.record_action(7, proposed).unwrap();
        s.update(7, "", Some(("ready", None))).unwrap();

        s.begin(&conversation.id, 8, "another question", BINDING)
            .unwrap();
        s.update(8, "another answer", Some(("ready", None)))
            .unwrap();
        let dispatch = s.begin_action(&action_id, 9, "status").unwrap();
        let mut completed = action_fixture("completed");
        completed.conversation_id = conversation.id.clone();
        completed.connection_id = dispatch.connection_id;
        completed.request_id = original_request;
        completed.parameter_digest = action_digest(&completed).unwrap();

        s.record_action(9, completed).unwrap();
        assert_eq!(
            s.get(&conversation.id).unwrap().messages[1].actions[0].status,
            ActionStatus::Completed
        );
    }

    #[test]
    fn action_batch_is_atomic_when_a_later_record_breaks_binding() {
        let temp = tempfile::tempdir().unwrap();
        let s = store(temp.path().join("assistant"));
        let conversation = s.create(None).unwrap();
        let request_id = s.begin(&conversation.id, 7, "lights", BINDING).unwrap();
        let mut first = action_fixture("proposed");
        first.action_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into();
        first.conversation_id = conversation.id.clone();
        first.connection_id = conversation.connection_id.clone().unwrap();
        first.request_id = request_id.clone();
        first.parameter_digest = action_digest(&first).unwrap();
        let mut second = first.clone();
        second.action_id = "cccccccc-cccc-4ccc-8ccc-cccccccccccc".into();
        second.connection_id = "dddddddd-dddd-4ddd-8ddd-dddddddddddd".into();
        second.parameter_digest = action_digest(&second).unwrap();
        assert!(s.record_actions(7, vec![first, second]).is_err());
        assert!(s.get(&conversation.id).unwrap().messages[1]
            .actions
            .is_empty());
    }

    #[test]
    fn active_action_cannot_update_a_different_stored_action() {
        let temp = tempfile::tempdir().unwrap();
        let s = store(temp.path().join("assistant"));
        let conversation = s.create(None).unwrap();
        let request_id = s.begin(&conversation.id, 7, "lights", BINDING).unwrap();
        let mut first = action_fixture("proposed");
        first.action_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into();
        first.conversation_id = conversation.id.clone();
        first.connection_id = conversation.connection_id.clone().unwrap();
        first.request_id = request_id.clone();
        first.parameter_digest = action_digest(&first).unwrap();
        let mut second = first.clone();
        second.action_id = "cccccccc-cccc-4ccc-8ccc-cccccccccccc".into();
        second.parameter_digest = action_digest(&second).unwrap();
        s.record_actions(7, vec![first.clone(), second.clone()])
            .unwrap();
        s.update(7, "", Some(("ready", None))).unwrap();
        s.begin_action(&first.action_id, 8, "confirm").unwrap();

        assert!(s.record_action(8, second).is_err());
        let stored = s.get(&conversation.id).unwrap();
        assert_eq!(stored.messages[1].actions[1].status, ActionStatus::Proposed);
    }
}
