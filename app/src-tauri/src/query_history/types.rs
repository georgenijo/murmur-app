use crate::query_provider::QueryProviderId;
use serde::{Deserialize, Serialize};

pub const QUERY_HISTORY_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_QUERY_HISTORY_PAGE_SIZE: u32 = 50;
pub const MAX_QUERY_HISTORY_PAGE_SIZE: u32 = 100;

/// The deliberately narrow usage contract allowed in durable query history.
/// Provider cost estimates are intentionally excluded from this type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryHistoryTokenCountsV1 {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryHistoryEntryV1 {
    pub schema_version: u32,
    pub id: String,
    pub timestamp_ms: i64,
    pub provider: QueryProviderId,
    pub question: String,
    pub answer: String,
    pub tokens: Option<QueryHistoryTokenCountsV1>,
    pub duration_ms: u64,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryHistoryPageV1 {
    pub schema_version: u32,
    pub entries: Vec<QueryHistoryEntryV1>,
    pub total: u32,
    pub offset: u32,
    pub has_more: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct QueryHistoryDraft {
    pub timestamp_ms: i64,
    pub provider: QueryProviderId,
    pub question: String,
    pub answer: String,
    pub tokens: Option<QueryHistoryTokenCountsV1>,
    pub duration_ms: u64,
    pub error_code: Option<String>,
}
