use serde::{Deserialize, Serialize};

pub const EXPORT_FORMAT: &str = "murmur-personal-knowledge";
pub const EXPORT_VERSION: u32 = 2;
pub const DEFAULT_PAGE_SIZE: u32 = 50;
pub const MAX_PAGE_SIZE: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeKind {
    ReplacementRule,
    VocabularyTerm,
    Snippet,
    /// User-defined selected-text transform (issue #312 D1): spoken name expands
    /// to a full rewrite instruction before the local LLM runs.
    Transform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceCommandKind {
    TextReplacement,
    Snippet,
}

impl VoiceCommandKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::TextReplacement => "text_replacement",
            Self::Snippet => "snippet",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "text_replacement" => Ok(Self::TextReplacement),
            "snippet" => Ok(Self::Snippet),
            _ => Err("Voice command has an unsupported type.".to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceCommandMetadata {
    pub command_type: VoiceCommandKind,
    #[serde(default)]
    pub allow_clipboard_read: bool,
}

impl KnowledgeKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ReplacementRule => "replacement_rule",
            Self::VocabularyTerm => "vocabulary_term",
            Self::Snippet => "snippet",
            Self::Transform => "transform",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeProvenance {
    Manual,
    CodeScan,
    LearnedCorrection,
    Import,
}

impl KnowledgeProvenance {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "manual" => Ok(Self::Manual),
            "code_scan" => Ok(Self::CodeScan),
            "learned_correction" => Ok(Self::LearnedCorrection),
            "import" => Ok(Self::Import),
            _ => Err("Knowledge record has unsupported provenance.".to_string()),
        }
    }

    pub(crate) fn precedence(self) -> u8 {
        match self {
            Self::Manual => 4,
            Self::Import => 3,
            Self::LearnedCorrection => 2,
            Self::CodeScan => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KnowledgeScope {
    Global,
    App {
        #[serde(rename = "bundleId")]
        bundle_id: String,
    },
    Project {
        #[serde(rename = "bundleId")]
        bundle_id: String,
        root: String,
    },
}

impl KnowledgeScope {
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::App { .. } => "app",
            Self::Project { .. } => "project",
        }
    }

    pub(crate) fn bundle_id(&self) -> Option<&str> {
        match self {
            Self::Global => None,
            Self::App { bundle_id } | Self::Project { bundle_id, .. } => Some(bundle_id),
        }
    }

    pub(crate) fn root(&self) -> Option<&str> {
        match self {
            Self::Project { root, .. } => Some(root),
            _ => None,
        }
    }

    pub(crate) fn specificity(&self) -> u8 {
        match self {
            Self::Global => 1,
            Self::App { .. } => 2,
            Self::Project { .. } => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KnowledgePayload {
    ReplacementRule {
        source: String,
        replacement: String,
    },
    VocabularyTerm {
        written: String,
        aliases: Vec<String>,
    },
    Snippet {
        trigger: String,
        body: String,
    },
    /// Named transform instruction for the selected-text flow (#312 D1).
    Transform {
        name: String,
        instruction: String,
    },
}

impl KnowledgePayload {
    pub fn kind(&self) -> KnowledgeKind {
        match self {
            Self::ReplacementRule { .. } => KnowledgeKind::ReplacementRule,
            Self::VocabularyTerm { .. } => KnowledgeKind::VocabularyTerm,
            Self::Snippet { .. } => KnowledgeKind::Snippet,
            Self::Transform { .. } => KnowledgeKind::Transform,
        }
    }

    pub(crate) fn storage_parts(&self) -> (String, String, Vec<String>) {
        match self {
            Self::ReplacementRule {
                source,
                replacement,
            } => (source.clone(), replacement.clone(), Vec::new()),
            Self::VocabularyTerm { written, aliases } => {
                (written.clone(), String::new(), aliases.clone())
            }
            Self::Snippet { trigger, body } => (trigger.clone(), body.clone(), Vec::new()),
            Self::Transform { name, instruction } => {
                (name.clone(), instruction.clone(), Vec::new())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeEntry {
    pub id: String,
    pub payload: KnowledgePayload,
    pub enabled: bool,
    pub scope: KnowledgeScope,
    pub provenance: KnowledgeProvenance,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub revision: u64,
    #[serde(default)]
    pub voice_command: Option<VoiceCommandMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeDraft {
    pub id: Option<String>,
    pub expected_revision: Option<u64>,
    pub payload: KnowledgePayload,
    pub enabled: bool,
    pub scope: KnowledgeScope,
    #[serde(default)]
    pub voice_command: Option<VoiceCommandMetadata>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeListRequest {
    pub query: Option<String>,
    pub kind: Option<KnowledgeKind>,
    pub enabled: Option<bool>,
    pub scope_kind: Option<String>,
    pub voice_command: Option<bool>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceCommandPreviewRequest {
    pub draft: KnowledgeDraft,
    pub text: String,
    #[serde(default)]
    pub read_clipboard: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceCommandPreviewResponse {
    pub output: String,
    pub matched: bool,
    pub clipboard_required: bool,
    pub clipboard_read: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeListResponse {
    pub entries: Vec<KnowledgeEntry>,
    pub total: u64,
    pub next_offset: Option<u32>,
    pub store_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeResolveRequest {
    pub kind: KnowledgeKind,
    pub trigger: String,
    pub bundle_id: Option<String>,
    pub project_root: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreAvailability {
    Ready,
    Recovered,
    Reinitialized,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeStoreStatus {
    pub availability: StoreAvailability,
    pub schema_version: u32,
    pub record_count: u64,
    pub store_revision: u64,
    pub recovery_at_ms: Option<i64>,
    pub message: Option<String>,
}

impl Default for KnowledgeStoreStatus {
    fn default() -> Self {
        Self {
            availability: StoreAvailability::Unavailable,
            schema_version: 0,
            record_count: 0,
            store_revision: 0,
            recovery_at_ms: None,
            message: Some("The local knowledge store has not been initialized.".to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeExport {
    pub format: String,
    pub version: u32,
    pub exported_at_ms: i64,
    pub entries: Vec<KnowledgeEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeImportSummary {
    pub total: u64,
    pub new: u64,
    pub duplicates: u64,
    pub conflicts: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeImportResult {
    pub imported: u64,
    pub duplicates: u64,
    pub store_revision: u64,
}
