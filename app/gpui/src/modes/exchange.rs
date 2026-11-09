use super::model::*;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
fn exact(v: &Value, keys: &[&str]) -> bool {
    v.as_object()
        .is_some_and(|m| m.len() == keys.len() && keys.iter().all(|k| m.contains_key(*k)))
}
fn text(v: &Value, max: usize) -> bool {
    v.as_str().is_some_and(|s| {
        !s.is_empty() && s.trim() == s && s.len() <= max && !s.chars().any(char::is_control)
    })
}
/// Shipped wording (`src/lib/modeExchange.ts`, `ModeFileActions.tsx`) for every
/// Mode-file outcome, so the production adapter reports the real reason.
pub mod reason {
    pub const SIZE: &str = "Mode files must be 256 KiB or smaller.";
    pub const SYNTAX: &str = "Choose a valid JSON Mode file.";
    pub const DUPLICATE_KEYS: &str = "Duplicate JSON keys are not allowed.";
    pub const DEPTH: &str = "The Mode file is nested too deeply.";
    pub const FIELDS: &str = "The Mode file has missing or unknown fields.";
    pub const STRING: &str = "The Mode file contains an invalid string.";
    pub const BOOLEAN: &str = "The Mode file contains an invalid boolean.";
    pub const CHOICE: &str =
        "The Mode file contains an unsupported policy, model, language, or browser.";
    pub const FORMAT: &str = "This Mode file format or version is not supported.";
    pub const BUILTIN: &str = "Built-in Modes cannot be imported or replaced.";
    pub const REFERENCE: &str = "Every binding must reference a custom Mode included in the file.";
    pub const BUNDLE: &str = "An app binding has an invalid bundle ID.";
    pub const HOST: &str =
        "Site rules require a lowercase exact host without a URL or trailing dot.";
    pub const LIST_100: &str =
        "The Mode file exceeds a list limit of 100 or contains an invalid list.";
    pub const LIST_128: &str =
        "The Mode file exceeds a list limit of 128 or contains an invalid list.";
    /// The file could not be chosen, opened, or read at all.
    pub const READ: &str = "Murmur could not read the Mode file. Choose a regular UTF-8 JSON file, 256 KiB or smaller.";
    pub const EXPORT: &str = "Murmur could not export custom modes. Check the destination and that modes and bindings are valid.";
    pub const STALE: &str =
        "Modes or bindings changed. Choose the file again to review a fresh preview.";
    pub const CONFLICTED: &str =
        "Resolve the listed conflicts before importing. Nothing was changed.";
    pub const APPLIED: &str = "Import applied. Browser-site activation preference was preserved.";
    pub const EXPORTED: &str = "Custom modes exported.";
    /// Every message the presentation may show verbatim for a failed Modes action.
    pub const FAILURES: [&str; 19] = [
        EXPORT,
        STALE,
        CONFLICTED,
        SIZE,
        SYNTAX,
        DUPLICATE_KEYS,
        DEPTH,
        FIELDS,
        STRING,
        BOOLEAN,
        CHOICE,
        FORMAT,
        BUILTIN,
        REFERENCE,
        BUNDLE,
        HOST,
        LIST_100,
        LIST_128,
        READ,
    ];
}
fn bundle_id(s: &str) -> bool {
    let part = |p: &str| !p.is_empty() && p.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-');
    s.contains('.') && s.split('.').all(part)
}
pub fn parse(text_source: &str) -> Result<Value, Problem> {
    parse_reason(text_source).map_err(|_| Problem::Invalid)
}
/// The same checks as `parse`, naming the first shipped rejection reason.
pub fn parse_reason(text_source: &str) -> Result<Value, &'static str> {
    use crate::json::Fault;
    let v = crate::json::parse_fault(text_source, 256 * 1024, 8).map_err(|f| match f {
        Fault::Size => reason::SIZE,
        Fault::Syntax => reason::SYNTAX,
        Fault::Duplicate => reason::DUPLICATE_KEYS,
        Fault::Depth => reason::DEPTH,
    })?;
    if !exact(
        &v,
        &[
            "format",
            "version",
            "modes",
            "appBindings",
            "browserSiteRules",
        ],
    ) {
        return Err(reason::FIELDS);
    }
    if v["format"] != "murmur-modes" || v["version"] != 1 {
        return Err(reason::FORMAT);
    }
    let modes = v["modes"]
        .as_array()
        .filter(|a| a.len() <= 100)
        .ok_or(reason::LIST_100)?;
    for m in modes {
        if !exact(
            m,
            &[
                "id",
                "name",
                "builtIn",
                "enabled",
                "writingStyle",
                "cleanupEnabled",
                "smartFormattingEnabled",
                "cliFormattingEnabled",
                "vocabularyPolicy",
                "contextPolicy",
                "modelId",
                "language",
                "autoPaste",
            ],
        ) {
            return Err(reason::FIELDS);
        }
        if !text(&m["id"], 128) || !text(&m["name"], 128) {
            return Err(reason::STRING);
        }
        let id = m["id"].as_str().unwrap_or("");
        if m["builtIn"] != false
            || id.starts_with("builtin.")
            || builtins().iter().any(|b| b["id"] == m["id"])
        {
            return Err(reason::BUILTIN);
        }
        let mut settings = murmur_core::state::settings_defaults();
        settings["modes"] = json!([m]);
        let normalized = murmur_core::state::normalize("dictation-settings", settings)
            .map_err(|_| reason::CHOICE)?;
        if normalized["modes"] != json!([m]) {
            return Err(reason::CHOICE);
        }
    }
    // Repeated ids, apps and sites are reviewed by `preview`, as shipped does.
    let ids: BTreeSet<&str> = modes.iter().filter_map(|m| m["id"].as_str()).collect();
    let bindings = v["appBindings"]
        .as_array()
        .filter(|a| a.len() <= 100)
        .ok_or(reason::LIST_100)?;
    for b in bindings {
        if !exact(b, &["bundleId", "modeId"]) {
            return Err(reason::FIELDS);
        }
        if !text(&b["bundleId"], 255) || !text(&b["modeId"], 128) {
            return Err(reason::STRING);
        }
        if !bundle_id(b["bundleId"].as_str().unwrap_or("")) {
            return Err(reason::BUNDLE);
        }
        if !ids.contains(b["modeId"].as_str().unwrap_or("")) {
            return Err(reason::REFERENCE);
        }
    }
    let rules = v["browserSiteRules"]
        .as_array()
        .filter(|a| a.len() <= 128)
        .ok_or(reason::LIST_128)?;
    for r in rules {
        if !exact(r, &["id", "browserBundleId", "host", "modeId", "enabled"]) {
            return Err(reason::FIELDS);
        }
        if !text(&r["host"], 253) || !text(&r["id"], 128) {
            return Err(reason::STRING);
        }
        if normalize_host(r["host"].as_str().unwrap_or("")).as_deref() != r["host"].as_str() {
            return Err(reason::HOST);
        }
        if !BROWSERS.iter().any(|(b, _)| r["browserBundleId"] == *b) {
            return Err(reason::CHOICE);
        }
        if !text(&r["modeId"], 128) || !ids.contains(r["modeId"].as_str().unwrap_or("")) {
            return Err(reason::REFERENCE);
        }
        if !r["enabled"].is_boolean() {
            return Err(reason::BOOLEAN);
        }
    }
    Ok(v)
}
pub fn subset(settings: &Value) -> Value {
    json!({"modes":settings["modes"],"appProfiles":settings["appProfiles"],"browserSiteRules":settings["browserSiteRules"],"siteModeLookupEnabled":settings["siteModeLookupEnabled"]})
}
/// Conflicts are always counted; only this many are listed.
pub const LISTED_CONFLICTS: usize = 50;
#[derive(Clone)]
pub struct Preview {
    pub baseline: Value,
    pub next: Value,
    pub additions: usize,
    pub bindings: usize,
    pub sites: usize,
    pub duplicates: usize,
    pub missing_apps: usize,
    pub conflicts: usize,
    /// The first `LISTED_CONFLICTS` conflict descriptions, in review order.
    pub lines: Vec<String>,
}
impl Preview {
    fn conflict(&mut self, line: String) {
        self.conflicts += 1;
        if self.lines.len() < LISTED_CONFLICTS {
            self.lines.push(line)
        }
    }
}
pub fn preview(file: &Value, settings: &Value) -> Result<Preview, Problem> {
    let mut p = Preview {
        baseline: subset(settings),
        next: subset(settings),
        additions: 0,
        bindings: 0,
        sites: 0,
        duplicates: 0,
        missing_apps: 0,
        conflicts: 0,
        lines: vec![],
    };
    for m in file["modes"].as_array().ok_or(Problem::Invalid)? {
        let modes = p.next["modes"].as_array_mut().ok_or(Problem::Invalid)?;
        if let Some(old) = modes.iter().find(|v| v["id"] == m["id"]) {
            if old == m {
                p.duplicates += 1
            } else {
                p.conflict(format!(
                    "Mode ID conflict: {}",
                    m["id"].as_str().unwrap_or("")
                ))
            }
        } else {
            modes.push(m.clone());
            p.additions += 1
        }
    }
    let mut imported = BTreeMap::new();
    for b in file["appBindings"].as_array().ok_or(Problem::Invalid)? {
        let bundle = b["bundleId"].as_str().unwrap_or("");
        let mode = b["modeId"].as_str().unwrap_or("");
        if imported
            .get(bundle)
            .is_some_and(|previous| *previous != mode)
        {
            p.conflict(format!("App binding conflict: {bundle}"));
            continue;
        }
        imported.insert(bundle, mode);
        let profiles = p.next["appProfiles"]
            .as_array_mut()
            .ok_or(Problem::Invalid)?;
        let count = profiles
            .iter()
            .filter(|v| v["bundleId"] == b["bundleId"])
            .count();
        if count == 0 {
            p.missing_apps += 1;
            continue;
        }
        if count > 1 {
            p.conflict(format!("Ambiguous app profile: {bundle}"));
            continue;
        }
        let profile = profiles
            .iter_mut()
            .find(|v| v["bundleId"] == b["bundleId"])
            .unwrap();
        if profile["modeId"] == b["modeId"] {
            p.duplicates += 1
        } else if !profile["modeId"].is_null() {
            p.conflict(format!("App binding conflict: {bundle}"))
        } else {
            profile["modeId"] = b["modeId"].clone();
            p.bindings += 1
        }
    }
    for r in file["browserSiteRules"]
        .as_array()
        .ok_or(Problem::Invalid)?
    {
        let rules = p.next["browserSiteRules"]
            .as_array_mut()
            .ok_or(Problem::Invalid)?;
        let existing = rules.iter().find(|v| {
            v["id"] == r["id"]
                || (v["browserBundleId"] == r["browserBundleId"] && v["host"] == r["host"])
        });
        match existing {
            None => {
                rules.push(r.clone());
                p.sites += 1
            }
            Some(old) if old == r => p.duplicates += 1,
            Some(_) => p.conflict(format!(
                "Site rule conflict: {}",
                r["host"].as_str().unwrap_or("")
            )),
        }
    }
    if p.next["modes"].as_array().unwrap().len() > 100 {
        p.conflict("Import would exceed the 100 custom Mode limit.".into())
    }
    if p.next["browserSiteRules"].as_array().unwrap().len() > 128 {
        p.conflict("Import would exceed the 128 site rule limit.".into())
    }
    Ok(p)
}
pub fn commit(p: &Preview, current: &Value) -> Result<Value, Problem> {
    commit_reason(p, current).map_err(|_| Problem::Conflict)
}
/// Shipped order: a changed baseline is reported before listed conflicts.
pub fn commit_reason(p: &Preview, current: &Value) -> Result<Value, &'static str> {
    if p.baseline != subset(current) {
        return Err(reason::STALE);
    }
    if p.conflicts > 0 {
        return Err(reason::CONFLICTED);
    }
    Ok(p.next.clone())
}
pub fn export(settings: &Value) -> Result<String, Problem> {
    let known =
        murmur_core::state::settings_export(settings.clone()).map_err(|_| Problem::Invalid)?;
    let settings = &known;
    let modes = settings["modes"].as_array().ok_or(Problem::Invalid)?;
    let ids: BTreeSet<_> = modes.iter().filter_map(|m| m["id"].as_str()).collect();
    let file = json!({"format":"murmur-modes","version":1,"modes":modes,"appBindings":settings["appProfiles"].as_array().ok_or(Problem::Invalid)?.iter().filter(|p|ids.contains(p["modeId"].as_str().unwrap_or(""))).map(|p|json!({"bundleId":p["bundleId"],"modeId":p["modeId"]})).collect::<Vec<_>>(),"browserSiteRules":settings["browserSiteRules"].as_array().ok_or(Problem::Invalid)?.iter().filter(|r|ids.contains(r["modeId"].as_str().unwrap_or(""))).collect::<Vec<_>>()});
    // Shipped writes pretty JSON plus a newline, compact only past the size limit.
    let pretty = serde_json::to_string_pretty(&file).map_err(|_| Problem::Invalid)? + "\n";
    let text = if pretty.len() <= 256 * 1024 {
        pretty
    } else {
        serde_json::to_string(&file).map_err(|_| Problem::Invalid)?
    };
    parse(&text)?;
    Ok(text)
}
