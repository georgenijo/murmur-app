pub use crate::workspace::{Event, Problem, Session, Ticket};
use crate::{section, workspace::Projection};
use serde_json::{Value, json};
use std::collections::BTreeSet;
#[derive(Clone)]
pub struct Snapshot {
    pub revision: u64,
    pub availability: String,
    pub store_revision: u64,
    pub records: Vec<Value>,
    pub total: usize,
    pub offset: usize,
    pub next: Option<usize>,
    pub import_preview: Option<Value>,
    pub bulk: Option<Bulk>,
    pub notice: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub query: String,
    pub kind: Option<String>,
    pub enabled: Option<bool>,
    pub scope: Option<String>,
}
impl Filter {
    pub fn wire(&self, offset: usize) -> Value {
        let mut v = json!({"limit":50,"offset":offset});
        if !self.query.is_empty() {
            v["query"] = json!(self.query.trim())
        }
        if let Some(k) = &self.kind {
            v["kind"] = json!(k)
        }
        if let Some(e) = self.enabled {
            v["enabled"] = json!(e)
        }
        if let Some(s) = &self.scope {
            v["scopeKind"] = json!(s)
        }
        v
    }
}
pub fn title(v: &Value) -> &str {
    let p = &v["payload"];
    ["source", "written", "trigger", "name"]
        .into_iter()
        .find_map(|k| p[k].as_str())
        .unwrap_or("Untitled")
}
pub fn detail(v: &Value) -> String {
    let p = &v["payload"];
    ["replacement", "body", "instruction"]
        .into_iter()
        .find_map(|k| p[k].as_str().map(String::from))
        .unwrap_or_else(|| {
            p["aliases"]
                .as_array()
                .map_or("No spoken aliases".into(), |a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
        })
}
pub fn valid_record(v: &Value) -> bool {
    v["id"]
        .as_str()
        .is_some_and(|s| !s.is_empty() && section::text(s, 128))
        && v["revision"].as_u64().is_some()
        && v["enabled"].is_boolean()
        && valid_draft(v).is_ok()
}
pub fn valid_draft(v: &Value) -> Result<(), Problem> {
    let p = &v["payload"];
    let required = match p["kind"].as_str() {
        Some("replacement_rule") => {
            if v["voiceCommand"].is_object() {
                ["source", "source"]
            } else {
                ["source", "replacement"]
            }
        }
        Some("vocabulary_term") => ["written", "written"],
        Some("snippet") => ["trigger", "body"],
        Some("transform") => ["name", "instruction"],
        _ => return Err(Problem::Invalid),
    };
    if !required.into_iter().all(|k| {
        p[k].as_str()
            .is_some_and(|s| !s.trim().is_empty() && section::text(s, 65536))
    }) {
        return Err(Problem::Invalid);
    }
    if p["kind"] == "vocabulary_term"
        && !p["aliases"].as_array().is_some_and(|a| {
            a.len() <= 64
                && a.iter()
                    .all(|s| s.as_str().is_some_and(|s| section::text(s, 1024)))
        })
    {
        return Err(Problem::Invalid);
    }
    let scope = &v["scope"];
    match scope["kind"].as_str() {
        Some("global") => (),
        Some("app" | "project") => {
            if !scope["bundleId"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty() && section::text(s, 255))
            {
                return Err(Problem::Invalid);
            }
            if scope["kind"] == "project"
                && !scope["root"]
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty() && section::text(s, 4096))
            {
                return Err(Problem::Invalid);
            }
        }
        _ => return Err(Problem::Invalid),
    }
    Ok(())
}
#[derive(Clone)]
pub struct Bulk {
    pub filter: Filter,
    pub revision: u64,
    pub total: usize,
    pub entries: Vec<Value>,
    pub complete: bool,
}
impl Bulk {
    pub fn new(filter: Filter, revision: u64, total: usize) -> Result<Self, Problem> {
        if total > 10000 {
            return Err(Problem::Invalid);
        }
        Ok(Self {
            filter,
            revision,
            total,
            entries: vec![],
            complete: false,
        })
    }
    pub fn append(
        &mut self,
        revision: u64,
        total: usize,
        entries: Vec<Value>,
        next: Option<usize>,
    ) -> Result<(), Problem> {
        if self.complete
            || revision != self.revision
            || total != self.total
            || entries.len() > 50
            || entries.iter().any(|v| !valid_record(v))
        {
            return Err(Problem::Conflict);
        }
        let ids: BTreeSet<_> = self
            .entries
            .iter()
            .chain(entries.iter())
            .filter_map(|v| v["id"].as_str())
            .collect();
        let len = self.entries.len() + entries.len();
        if ids.len() != len
            || len > self.total
            || next.is_some_and(|n| n != len || n <= self.entries.len())
            || next.is_none() && len != total
        {
            return Err(Problem::Conflict);
        }
        if section::bytes(&json!(&self.entries)) + section::bytes(&json!(&entries))
            > 8 * 1024 * 1024
        {
            return Err(Problem::Invalid);
        }
        self.entries.extend(entries);
        self.complete = next.is_none();
        Ok(())
    }
    pub fn commands(&self, action: BulkAction) -> Result<Vec<(&'static str, Value)>, Problem> {
        if !self.complete {
            return Err(Problem::Invalid);
        }
        Ok(self.entries.iter().filter(|v|action==BulkAction::Delete||v["enabled"]!=json!(action==BulkAction::Enable)).map(|v|if action==BulkAction::Delete{("delete_knowledge",json!({"id":v["id"],"expectedRevision":v["revision"]}))}else{("set_knowledge_enabled",json!({"id":v["id"],"expectedRevision":v["revision"],"enabled":action==BulkAction::Enable}))}).collect())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BulkAction {
    Enable,
    Disable,
    Delete,
}
impl Projection for Snapshot {
    fn inbox_limit() -> usize {
        9 * 1024 * 1024 + 32768
    }
    fn revision(&self) -> u64 {
        self.revision
    }
    fn bytes(&self) -> usize {
        section::bytes(&json!(&self.records))
            .saturating_add(
                self.bulk
                    .as_ref()
                    .map_or(0, |b| section::bytes(&json!(&b.entries))),
            )
            .saturating_add(4096)
    }
    fn valid(&self) -> bool {
        ["ready", "recovered", "reinitialized", "unavailable"].contains(&self.availability.as_str())
            && self.records.len() <= 50
            && self.records.iter().all(valid_record)
            && self.total <= 10000
            && self.offset <= self.total
            && self.next.is_none_or(|n| n > self.offset && n <= self.total)
            && self.bytes() <= 9 * 1024 * 1024
            && self.notice.as_ref().is_none_or(|s| section::text(s, 512))
    }
}
