use super::compile;
use crate::section::Problem;
use serde_json::{Value, json};
pub fn builtin(id: &str) -> Option<Value> {
    match id {
        "sonic" => Some(
            json!({"version":1,"id":"sonic","label":"Sonic","modes":["light","dark"],"theme":{"version":1,"presetId":"sonic"},"source":{"kind":"local"}}),
        ),
        "opencode" => {
            let v: Value = serde_json::from_str(include_str!("palettes.json")).unwrap();
            Some(
                json!({"version":1,"id":"opencode","label":"OpenCode","modes":["light","dark"],"theme":{"version":1,"presetId":"custom","light":v["opencode_light"],"dark":v["opencode_dark"]},"source":{"kind":"local"}}),
            )
        }
        _ => None,
    }
}
pub fn entries(library: &Value) -> Vec<Value> {
    [builtin("sonic").unwrap(), builtin("opencode").unwrap()]
        .into_iter()
        .chain(library["themes"].as_array().into_iter().flatten().cloned())
        .collect()
}
/// The active custom appearance remains editable even without a saved library entry.
pub fn display_entries(current: &Value, library: &Value) -> Vec<Value> {
    let mut all = entries(library);
    if ["light", "dark"]
        .iter()
        .any(|mode| current["selection"][mode] == "custom")
    {
        all.insert(2, json!({"version":1,"id":"custom","label":"Custom","modes":["light","dark"],"theme":current["theme"]}));
    }
    all
}
pub fn selection(
    current: &Value,
    library: &Value,
    id: &str,
    appearance: Option<&str>,
) -> Result<Value, Problem> {
    let all = display_entries(current, library);
    let entry = all.iter().find(|v| v["id"] == id).ok_or(Problem::Invalid)?;
    let modes = entry["modes"].as_array().ok_or(Problem::Invalid)?;
    let mut selection=current.get("selection").cloned().unwrap_or(json!({"light":if current["theme"]["presetId"]=="sonic"{"sonic"}else{"custom"},"dark":if current["theme"]["presetId"]=="sonic"{"sonic"}else{"custom"}}));
    let mut out = current.clone();
    if let Some(mode) = appearance {
        if !modes.contains(&json!(mode)) {
            return Err(Problem::Invalid);
        }
        selection[mode] = json!(id);
        out["mode"] = json!(mode);
    } else if modes.len() == 1 {
        let mode = modes[0].as_str().ok_or(Problem::Invalid)?;
        selection[mode] = json!(id);
        out["mode"] = json!(mode);
    } else {
        selection = json!({"light":id,"dark":id})
    }
    let theme = if selection["light"] == "sonic" && selection["dark"] == "sonic" {
        json!({"version":1,"presetId":"sonic"})
    } else {
        let mut t = json!({"version":1,"presetId":"custom"});
        for (mode, dark) in [("light", false), ("dark", true)] {
            let id = selection[mode].as_str().ok_or(Problem::Invalid)?;
            let source = all
                .iter()
                .find(|v| {
                    v["id"] == id
                        && v["modes"]
                            .as_array()
                            .is_some_and(|a| a.contains(&json!(mode)))
                })
                .map(|v| &v["theme"])
                .unwrap_or(&current["theme"]);
            t[mode] = compile::value(&compile::resolve(source, dark));
        }
        t
    };
    out["theme"] = theme;
    out["selection"] = selection;
    out["revision"] = json!(next_revision(current["revision"].as_u64().unwrap_or(0)));
    Ok(out)
}
pub fn next_revision(revision: u64) -> u64 {
    if revision >= 2147483646 {
        1
    } else {
        revision + 1
    }
}
pub fn install(
    library: &Value,
    entries: &[Value],
    expected: Option<(&str, &[Value])>,
) -> Result<Value, Problem> {
    let mut out = library.clone();
    let themes = out["themes"].as_array_mut().ok_or(Problem::Invalid)?;
    if let Some((id, expected)) = expected {
        let existing: Vec<_> = themes
            .iter()
            .filter(|v| v["collection"]["id"] == id)
            .cloned()
            .collect();
        if existing != expected || entries.iter().any(|v| v["collection"]["id"] != id) {
            return Err(Problem::Conflict);
        }
        themes.retain(|v| v["collection"]["id"] != id);
    }
    for entry in entries {
        if ["sonic", "custom", "opencode"].contains(&entry["id"].as_str().unwrap_or("")) {
            return Err(Problem::Invalid);
        }
        if let Some(old) = themes.iter().find(|v| v["id"] == entry["id"]) {
            if old != entry {
                return Err(Problem::Conflict);
            }
        } else {
            themes.push(entry.clone())
        }
    }
    out["revision"] = json!(next_revision(library["revision"].as_u64().unwrap_or(0)));
    murmur_core::state::normalize("murmur-theme-library", out).map_err(|_| Problem::Invalid)
}
pub fn remove(library: &Value, ids: &[String]) -> Result<Value, Problem> {
    if ids
        .iter()
        .any(|id| ["sonic", "opencode"].contains(&id.as_str()))
    {
        return Err(Problem::Invalid);
    }
    let mut out = library.clone();
    out["themes"]
        .as_array_mut()
        .ok_or(Problem::Invalid)?
        .retain(|v| !ids.iter().any(|id| v["id"] == *id));
    out["revision"] = json!(next_revision(library["revision"].as_u64().unwrap_or(0)));
    Ok(out)
}
