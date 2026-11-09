pub use crate::workspace::{Event, Problem, Session, Ticket};
use crate::{section, workspace::Projection};
use serde_json::{Value, json};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Spellings,
    Commands,
    Apps,
    Scan,
}
impl Page {
    pub const ALL: [Self; 4] = [Self::Spellings, Self::Commands, Self::Apps, Self::Scan];
    pub fn label(self) -> &'static str {
        match self {
            Self::Spellings => "Saved spellings",
            Self::Commands => "Voice commands",
            Self::Apps => "App overrides",
            Self::Scan => "Project vocabulary",
        }
    }
}
#[derive(Clone)]
pub struct Snapshot {
    pub revision: u64,
    pub settings: Value,
    pub commands: Vec<Value>,
    pub command_total: usize,
    pub next_offset: Option<usize>,
    pub running_apps: Vec<Value>,
    pub ide: Value,
    pub scan: Value,
    pub preview: Option<String>,
    pub preview_note: Option<&'static str>,
    pub import_preview: Option<super::vocabulary::Preview>,
}
impl Projection for Snapshot {
    fn revision(&self) -> u64 {
        self.revision
    }
    fn bytes(&self) -> usize {
        section::bytes(&self.settings)
            + section::bytes(&json!(self.commands))
            + section::bytes(&self.ide)
            + section::bytes(&self.scan)
            + self.preview.as_ref().map_or(0, String::len)
            + self.import_preview.as_ref().map_or(0, |p| {
                section::bytes(&json!(&p.entries)) + section::bytes(&p.baseline)
            })
            + 32768
    }
    fn valid(&self) -> bool {
        murmur_core::state::normalize("dictation-settings", self.settings.clone())
            .is_ok_and(|v| v == self.settings)
            && self.commands.len() <= 50
            && self
                .commands
                .iter()
                .all(crate::knowledge::model::valid_record)
            && self.command_total <= 10000
            && self.scan["rankedTerms"]
                .as_array()
                .is_none_or(|a| a.len() <= 500)
            && self.running_apps.len() <= 64
            && self.running_apps.iter().all(|v| {
                v["bundleId"]
                    .as_str()
                    .is_some_and(|s| section::text(s, 255))
                    && v["name"].as_str().is_some_and(|s| section::text(s, 256))
            })
            && self
                .preview
                .as_ref()
                .is_none_or(|s| section::text(s, 65536))
            && self.bytes() <= 2 * 1024 * 1024
    }
}
pub fn profile(bundle: &str, label: &str) -> Value {
    json!({"bundleId":bundle,"label":label,"autoPasteOverride":null,"cleanupOverride":null,"smartFormattingOverride":null,"cliFormattingOverride":null,"writingStyle":null,"ideContextEnabled":false,"ideProjectRoots":[]})
}
pub fn duplicate_command(entry: &Value) -> Value {
    let mut v = entry.clone();
    v.as_object_mut().unwrap().remove("id");
    v.as_object_mut().unwrap().remove("revision");
    match v["payload"]["kind"].as_str() {
        Some("snippet") => v["payload"]["trigger"] = json!(""),
        _ => v["payload"]["source"] = json!(""),
    };
    v
}
pub fn command_draft(values: &Value, entry: Option<&Value>) -> Result<Value, Problem> {
    let string = |k| values[k].as_str().unwrap_or("");
    let snippet = string("kind") == "snippet";
    let phrase = string("phrase").trim();
    let content = string("content");
    if phrase.is_empty()
        || phrase.len() > 256
        || snippet && content.is_empty()
        || content.len() > 65536
    {
        return Err(Problem::Invalid);
    }
    let clipboard = snippet && string("allowClipboard") == "true";
    if snippet {
        let mut rest = content;
        while let Some(start) = rest.find("{{") {
            let tail = &rest[start + 2..];
            let end = tail.find("}}").ok_or(Problem::Invalid)?;
            if !["date", "time", "clipboard"].contains(&&tail[..end])
                || &tail[..end] == "clipboard" && !clipboard
            {
                return Err(Problem::Invalid);
            }
            rest = &tail[end + 2..];
        }
    }
    let scope = if string("scope") == "app" {
        let bundle = string("bundleId").trim();
        if bundle.is_empty() {
            return Err(Problem::Invalid);
        }
        json!({"kind":"app","bundleId":bundle})
    } else {
        json!({"kind":"global"})
    };
    let mut draft = json!({"payload":if snippet{json!({"kind":"snippet","trigger":phrase,"body":content})}else{json!({"kind":"replacement_rule","source":phrase,"replacement":content})},"scope":scope,"enabled":string("enabled")!="false","voiceCommand":{"commandType":if snippet{"snippet"}else{"text_replacement"},"allowClipboardRead":clipboard}});
    if let Some(e) = entry
        && e["id"].is_string()
    {
        draft["id"] = e["id"].clone();
        draft["expectedRevision"] = e["revision"].clone();
    }
    Ok(draft)
}
