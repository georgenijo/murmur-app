pub use crate::section::{Host as SectionHost, Request as SectionRequest, Subscription};
pub type Host = dyn SectionHost<super::model::Snapshot, Action>;
pub type Request = SectionRequest<Action>;
use serde_json::{Value, json};
#[derive(Clone)]
pub enum Action {
    Read,
    Patch(Value),
    ListCommands {
        query: String,
        offset: usize,
    },
    Upsert(Value),
    Delete(Value),
    Enable {
        entry: Value,
        enabled: bool,
    },
    PreviewCommand {
        draft: Value,
        text: String,
        read_clipboard: bool,
    },
    PreviewAliases {
        entries: Value,
        voice_commands: Value,
        text: String,
        cli: bool,
    },
    RunningApps,
    ChooseRoot(String),
    RefreshIde(String),
    ClearIde(String),
    ChooseProject,
    Scan {
        folder: String,
        id: String,
    },
    CancelScan(String),
    CopyTerms(Vec<String>),
    InspectSpellings,
    ImportSpellings(super::vocabulary::Preview),
    ExportSpellings,
    VoiceCommandSettings,
    Cancel,
}
impl Action {
    pub fn command(&self) -> Option<(&'static str, Value)> {
        Some(match self {
            Self::ListCommands { query, offset } => (
                "list_knowledge",
                json!({"request":{"query":query,"offset":offset,"limit":50,"voiceCommand":true}}),
            ),
            Self::Upsert(draft) => ("upsert_knowledge", json!({"draft":draft})),
            Self::Delete(e) => (
                "delete_knowledge",
                json!({"id":e["id"],"expectedRevision":e["revision"]}),
            ),
            Self::Enable { entry, enabled } => (
                "set_knowledge_enabled",
                json!({"id":entry["id"],"expectedRevision":entry["revision"],"enabled":enabled}),
            ),
            Self::PreviewCommand {
                draft,
                text,
                read_clipboard,
            } => (
                "preview_voice_command",
                json!({"request":{"draft":draft,"text":text,"readClipboard":read_clipboard}}),
            ),
            Self::PreviewAliases {
                entries,
                voice_commands,
                text,
                cli,
            } => (
                "preview_vocabulary_aliases",
                json!({"entries":entries,"voiceCommands":voice_commands,"text":text,"cliFormatting":cli}),
            ),
            Self::RunningApps => ("list_running_applications", json!({})),
            Self::RefreshIde(id) => ("refresh_ide_context", json!({"bundleId":id})),
            Self::ClearIde(id) => ("clear_ide_context", json!({"bundleId":id})),
            Self::Scan { folder, id } => ("scan_code_vocab", json!({"folder":folder,"scanId":id})),
            Self::CancelScan(id) => ("cancel_code_vocab_scan", json!({"scanId":id})),
            _ => return None,
        })
    }
}
// Patch uses the existing configure_dictation validation before durable source
// publication: validate alias/Voice Command conflicts atomically, preserve unknown
// settings and other profile fields, and keep an existing recording's frozen
// context. PreviewAliases adds current voiceCommands to its exact command payload.
// Read includes get_ide_context_status only for configured opted-in profiles and
// never launches scans or enumerates apps. Native panels are explicit/revocable;
// roots max four, no persistence until the panel result is accepted under lease.
// InspectSpellings uses read_modes_file + vocabulary::parse/preview, keeps
// reviewed bytes in memory; import repeats vocabulary::commit against current.
// Export uses vocabulary::export + save_text_export (native save panel).
// Scan progress is correlated by exact scan ID and settings generation. Hide/drop
// stops only this owner's scan and subscriptions; existing IDE generation owners
// invalidate before refresh/clear/remove. Memory-only symbols never enter sources.
// Clipboard preview requires BOTH command permission and explicit read_clipboard;
// listing/editing/saving never reads it. CopyTerms is explicit host clipboard write.
