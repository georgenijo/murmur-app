//! Strict spelling exchange and deterministic alias conflicts; no matcher copy.
use crate::section::Problem;
use serde_json::{Value, json};
use std::collections::BTreeSet;
fn exact(v: &Value, keys: &[&str]) -> bool {
    v.as_object()
        .is_some_and(|v| v.len() == keys.len() && keys.iter().all(|k| v.contains_key(*k)))
}
fn bundle(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        s.len() <= 255
            && s.split('.').count() >= 2
            && s.split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
    })
}
fn text(v: &Value, max: usize) -> bool {
    v.as_str().is_some_and(|s| {
        !s.trim().is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control)
    })
}
fn key(v: &str) -> String {
    v.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
fn overlap(a: &Value, b: &Value) -> bool {
    a["kind"] == "global"
        || b["kind"] == "global"
        || a["bundleId"] == b["bundleId"]
            && (a["kind"] != "project" || b["kind"] != "project" || a["root"] == b["root"])
}
pub fn validate(entries: &[Value], commands: &[Value]) -> Result<(), Problem> {
    if entries.len() > 500 {
        return Err(Problem::Invalid);
    }
    let enabled: Vec<_> = entries.iter().filter(|v| v["enabled"] == true).collect();
    let reserved: BTreeSet<_> = [
        "new paragraph",
        "new line",
        "scratch that",
        "open paren",
        "close paren",
        "question mark",
        "period",
        "comma",
    ]
    .into_iter()
    .map(String::from)
    .chain(
        commands
            .iter()
            .filter_map(|v| v["phrase"].as_str())
            .map(key),
    )
    .collect();
    let mut ids = BTreeSet::new();
    for v in entries {
        if !exact(v, &["id", "written", "aliases", "enabled", "scope"])
            || !text(&v["id"], 256)
            || !ids.insert(v["id"].to_string())
            || !text(&v["written"], 256)
            || !v["enabled"].is_boolean()
            || !v["aliases"]
                .as_array()
                .is_some_and(|a| a.len() <= 16 && a.iter().all(|s| text(s, 256)))
        {
            return Err(Problem::Invalid);
        }
        let scope = &v["scope"];
        let valid = match scope["kind"].as_str() {
            Some("global") => exact(scope, &["kind"]),
            Some("app") => exact(scope, &["kind", "bundleId"]) && bundle(&scope["bundleId"]),
            Some("project") => {
                exact(scope, &["kind", "bundleId", "root"])
                    && bundle(&scope["bundleId"])
                    && text(&scope["root"], 4096)
                    && scope["root"].as_str().unwrap().len() <= 4096
            }
            _ => false,
        };
        if !valid {
            return Err(Problem::Invalid);
        }
    }
    for (i, v) in enabled.iter().enumerate() {
        let written = key(v["written"].as_str().unwrap());
        let mut aliases = BTreeSet::new();
        for alias in v["aliases"].as_array().unwrap() {
            let alias = key(alias.as_str().unwrap());
            if !aliases.insert(alias.clone()) || alias == written || reserved.contains(&alias) {
                return Err(Problem::Conflict);
            }
            for (j, other) in enabled.iter().enumerate() {
                if i != j
                    && overlap(&v["scope"], &other["scope"])
                    && (key(other["written"].as_str().unwrap()) == alias
                        || other["aliases"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|a| key(a.as_str().unwrap()) == alias))
                {
                    return Err(Problem::Conflict);
                }
            }
        }
        if enabled.iter().skip(i + 1).any(|other| {
            overlap(&v["scope"], &other["scope"])
                && key(other["written"].as_str().unwrap()) == written
        }) {
            return Err(Problem::Conflict);
        }
    }
    Ok(())
}
pub fn parse(contents: &str) -> Result<Value, Problem> {
    let v = crate::json::parse(contents, 256 * 1024, 8)?;
    if !exact(&v, &["format", "version", "entries"])
        || v["format"] != "murmur-vocabulary"
        || v["version"] != 1
    {
        return Err(Problem::Invalid);
    }
    validate(v["entries"].as_array().ok_or(Problem::Invalid)?, &[])?;
    Ok(v)
}
#[derive(Clone)]
pub struct Preview {
    pub baseline: Value,
    pub entries: Vec<Value>,
    pub added: usize,
    pub duplicates: usize,
    pub conflicts: usize,
}
fn baseline(settings: &Value) -> Value {
    json!({"vocabularyEntries":settings["vocabularyEntries"],"voiceCommands":settings["voiceCommands"]})
}
pub fn preview(file: &Value, settings: &Value) -> Result<Preview, Problem> {
    let mut p = Preview {
        baseline: baseline(settings),
        entries: settings["vocabularyEntries"]
            .as_array()
            .ok_or(Problem::Invalid)?
            .clone(),
        added: 0,
        duplicates: 0,
        conflicts: 0,
    };
    for entry in file["entries"].as_array().ok_or(Problem::Invalid)? {
        if let Some(old) = p.entries.iter().find(|v| v["id"] == entry["id"]) {
            if old == entry {
                p.duplicates += 1
            } else {
                p.conflicts += 1
            }
        } else {
            p.entries.push(entry.clone());
            p.added += 1
        }
    }
    if validate(
        &p.entries,
        settings["voiceCommands"]
            .as_array()
            .ok_or(Problem::Invalid)?,
    )
    .is_err()
    {
        p.conflicts += 1
    }
    Ok(p)
}
pub fn commit(p: &Preview, settings: &Value) -> Result<Value, Problem> {
    if p.conflicts > 0 || p.baseline != baseline(settings) {
        return Err(Problem::Conflict);
    }
    Ok(json!({"vocabularyEntries":p.entries}))
}
pub fn export(settings: &Value) -> Result<String, Problem> {
    let v =
        json!({"format":"murmur-vocabulary","version":1,"entries":settings["vocabularyEntries"]});
    let s = serde_json::to_string_pretty(&v).map_err(|_| Problem::Invalid)?;
    parse(&s)?;
    Ok(s)
}
