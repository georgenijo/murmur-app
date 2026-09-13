//! Private, bounded local mirror of explicitly connected Pi conversations.
//! Content is emitted only to the main webview; IDs never become filesystem paths.
use crate::MutexExt;
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::PathBuf, sync::Mutex};
use tauri::Emitter;

const MAX_STORE: usize = 16 * 1024 * 1024;
const MAX_CONVERSATIONS: usize = 100;
const MAX_MESSAGES: usize = 200;
const STORAGE_ERROR: &str = "assistant_storage_unavailable";

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
    conversations: Vec<Conversation>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            version: 1,
            binding: None,
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
fn validate(snapshot: &Snapshot) -> Result<(), String> {
    if snapshot.version != 1 || snapshot.conversations.len() > MAX_CONVERSATIONS {
        return Err(STORAGE_ERROR.into());
    }
    for c in &snapshot.conversations {
        if !valid_id(&c.id) || c.title.len() > 256 || c.messages.len() > MAX_MESSAGES {
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
        validate(&snapshot)?;
        for c in &mut snapshot.conversations {
            c.active_pass_id = None;
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
            s.binding = binding;
            Ok(())
        })
    }
    pub(crate) fn connected(&self, binding: &str) -> bool {
        self.0.lock_or_recover().snapshot.binding.as_deref() == Some(binding)
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
            let mut c = Conversation {
                id: id(),
                title: "New conversation".into(),
                created_at_ms: timestamp,
                updated_at_ms: timestamp,
                messages: vec![],
                active_pass_id: None,
                live_state: None,
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
            let c = s
                .conversations
                .iter_mut()
                .find(|c| c.id == conversation_id)
                .ok_or("assistant_conversation_unavailable")?;
            if c.connection_binding.as_deref() != Some(expected_binding) {
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
    let mut payload =
        serde_json::json!({ "conversationId": c.id, "requestId": request_id, "message": message });
    if c.seed_pending {
        payload["seedMessages"] = serde_json::json!(c
            .messages
            .iter()
            .take(2)
            .map(|m| serde_json::json!({"role":m.role,"content":m.content}))
            .collect::<Vec<_>>());
    }
    let prompt = format!("MURMUR_ASSISTANT_V1\n{}", payload);
    if prompt.len() > 65536 {
        return Err("query_too_large".into());
    }
    Ok(prompt)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store(root: PathBuf) -> AssistantStore {
        let s = AssistantStore::default();
        s.initialize(root, None).unwrap();
        s.connect(Some("bridge".into())).unwrap();
        s
    }
    #[test]
    fn restart_preserves_partial_and_terminalizes_without_redispatch() {
        let temp = tempfile::tempdir().unwrap();
        let s = store(temp.path().join("assistant"));
        let c = s.create(None).unwrap();
        s.begin(&c.id, 7, "question", "bridge").unwrap();
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
        s.begin(&c.id, 7, "question", "bridge").unwrap();
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
        s.begin(&c.id, 8, "next", "bridge").unwrap();
        let prompt = s.prompt(8, "next").unwrap();
        assert!(prompt.contains("seedMessages"));
        assert!(prompt.starts_with("MURMUR_ASSISTANT_V1\n"));
        s.update(8, "reply", Some(("ready", None))).unwrap();
        s.begin(&c.id, 9, "later", "bridge").unwrap();
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
        s.begin(&c.id, 1, "first", "bridge").unwrap();
        let first = s.prompt(1, "first").unwrap();
        s.update(1, "partial", Some(("failed", Some("exit_nonzero"))))
            .unwrap();
        s.begin(&c.id, 2, "second", "bridge").unwrap();
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
        s.begin(&c.id, 1, "question", "bridge").unwrap();
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
        assert!(s.begin(&c.id, 1, &"x".repeat(32769), "bridge").is_err());
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
        s.connect(Some("different bridge".into())).unwrap();
        assert_eq!(
            s.begin(&c.id, 1, "new message", "different bridge")
                .unwrap_err(),
            "assistant_connection_changed"
        );
        assert!(s.get(&c.id).unwrap().messages.is_empty());
        s.connect(Some("bridge".into())).unwrap();
        assert!(s.begin(&c.id, 2, "new message", "bridge").is_ok());
    }

    #[test]
    fn reconnect_between_validation_and_admission_rejects_frozen_command() {
        let temp = tempfile::tempdir().unwrap();
        let s = store(temp.path().join("assistant"));
        let original = s.create(None).unwrap();
        s.connect(Some("other bridge".into())).unwrap();
        let other = s.create(None).unwrap();
        let frozen_binding = "other bridge";
        assert!(s.connected(frozen_binding));

        // Another command reconnects to the target conversation's original
        // bridge after the dispatch command was validated. Global and
        // conversation bindings now agree, but the frozen command does not.
        s.connect(Some("bridge".into())).unwrap();
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
        assert!(s.connected("bridge"));
        s.connect(Some("other bridge".into())).unwrap();
        assert!(
            matches!(s.import("private question".into(), "private answer".into(), "bridge"), Err(error) if error == "assistant_connection_changed")
        );
        assert!(s.list().unwrap().is_empty());
        s.connect(Some("bridge".into())).unwrap();
        assert!(s
            .import("private question".into(), "private answer".into(), "bridge")
            .is_ok());
    }
}
