use super::*;
use std::collections::{BTreeMap, HashSet};
fn color(v: &Value) -> Option<String> {
    let s = v.as_str()?.strip_prefix('#')?;
    if ![3, 6].contains(&s.len()) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(if s.len() == 3 {
        format!(
            "#{}",
            s.chars()
                .flat_map(|c| [c, c])
                .collect::<String>()
                .to_ascii_lowercase()
        )
    } else {
        format!("#{}", s.to_ascii_lowercase())
    })
}
pub(super) fn theme(v: &Value) -> Value {
    if !v.is_object() || v["version"] != 1 {
        return json!({"version":1,"presetId":"sonic"});
    }
    let mut out =
        json!({"version":1,"presetId":if v["presetId"]=="custom" {"custom"} else {"sonic"}});
    for k in ["accent", "background", "foreground"] {
        if let Some(c) = color(&v[k]) {
            out[k] = json!(c);
        }
    }
    if let Some(c) = v["contrast"].as_f64() {
        let c = c.clamp(-100., 100.).round() as i64;
        if c != 0 {
            out["contrast"] = json!(c);
        }
    }
    for mode in ["light", "dark"] {
        let mut colors = serde_json::Map::new();
        for k in [
            "background",
            "surface",
            "surface-container-low",
            "surface-container",
            "surface-container-high",
            "surface-container-lowest",
            "surface-container-highest",
            "primary",
            "primary-dim",
            "on-primary",
            "on-surface",
            "on-surface-variant",
            "outline-variant",
            "error",
            "success",
            "warning",
        ] {
            if let Some(c) = color(&v[mode][k]) {
                colors.insert(k.into(), json!(c));
            }
        }
        if !colors.is_empty() {
            out[mode] = json!(colors);
        }
    }
    out
}
fn revision(v: &Value) -> u64 {
    v.as_u64().filter(|n| *n < 2147483646).unwrap_or(0)
}
fn id(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        !s.is_empty()
            && s.len() <= 64
            && s.bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            && s.as_bytes()[0].is_ascii_alphanumeric()
    })
}
pub(super) fn document(v: Value) -> Result<Value> {
    if !v.is_object() || v["version"] != 1 {
        return Err(Error::Invalid);
    }
    let mut out = json!({"version":1,"revision":revision(&v["revision"]),"mode":if allowed(&v["mode"],&["system","light","dark"]) {v["mode"].clone()} else {json!("system")},"theme":theme(&v["theme"])});
    if id(&v["selection"]["light"]) && id(&v["selection"]["dark"]) {
        out["selection"] = json!({"light":v["selection"]["light"],"dark":v["selection"]["dark"]});
    }
    Ok(out)
}
pub(super) fn library(v: Value) -> Result<Value> {
    if v["version"] != 1 || !v["themes"].is_array() || v["themes"].as_array().unwrap().len() > 128 {
        return Err(Error::Invalid);
    }
    let reserved = ["sonic", "custom", "opencode"];
    let mut occupied: HashSet<String> = v["themes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["id"].as_str().map(String::from))
        .collect();
    let mut renamed: BTreeMap<String, String> = v
        .get("legacyRenamedIds")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| Error::Invalid)?
        .unwrap_or_default();
    let mut out = vec![];
    let mut seen = HashSet::new();
    for t in v["themes"].as_array().unwrap() {
        let label = t["label"].as_str().unwrap_or("").trim();
        if t["version"] != 1
            || !id(&t["id"])
            || label.is_empty()
            || label.encode_utf16().count() > 64
            || !t["modes"].is_array()
            || !t["theme"].is_object()
            || t["theme"]["version"] != 1
        {
            return Err(Error::Invalid);
        }
        let modes: Vec<_> = ["light", "dark"]
            .into_iter()
            .filter(|m| t["modes"].as_array().unwrap().contains(&json!(m)))
            .collect();
        if modes.is_empty() {
            return Err(Error::Invalid);
        }
        let source = &t["source"];
        let source = if source["kind"] == "local" {
            // Legacy local metadata extras are ignored, including sourceUrl.
            json!({"kind":"local"})
        } else {
            let extension = source["extensionId"].as_str().unwrap_or("");
            if source["kind"] != "open-vsx"
                || !ascii_id(extension, true)
                || ["version", "license"].iter().any(|k| {
                    !source[k]
                        .as_str()
                        .is_some_and(|s| !s.is_empty() && s.encode_utf16().count() <= 64)
                })
            {
                return Err(Error::Invalid);
            }
            let mut out = json!({"kind":"open-vsx","extensionId":extension,"version":source["version"],"license":source["license"]});
            if let Some(raw) = source.get("sourceUrl") {
                let raw = raw.as_str().ok_or(Error::Invalid)?;
                if raw.encode_utf16().count() > 2048 {
                    return Err(Error::Invalid);
                }
                let url = url::Url::parse(raw).map_err(|_| Error::Invalid)?;
                if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some()
                {
                    return Err(Error::Invalid);
                }
                out["sourceUrl"] = json!(url.as_str());
            }
            out
        };
        let original = t["id"].as_str().unwrap();
        let mut key = original.to_string();
        if reserved.contains(&original) {
            let mut base = String::new();
            for c in label.to_ascii_lowercase().chars() {
                if c.is_ascii_alphanumeric() {
                    base.push(c);
                } else if !base.ends_with('-') {
                    base.push('-');
                }
            }
            let base = base.trim_matches('-').chars().take(64).collect::<String>();
            let base = if base.is_empty() {
                "custom-theme".into()
            } else {
                base
            };
            key = base.clone();
            for n in 1..1000 {
                if !occupied.contains(&key) && !reserved.contains(&key.as_str()) {
                    break;
                }
                let suffix = format!("-{}", n + 1);
                key = format!("{}{}", &base[..base.len().min(64 - suffix.len())], suffix);
            }
            if occupied.contains(&key) || reserved.contains(&key.as_str()) {
                return Err(Error::Invalid);
            }
            renamed.entry(original.into()).or_insert(key.clone());
            occupied.insert(key.clone());
        }
        if !seen.insert(key.clone()) {
            return Err(Error::Invalid);
        }
        let mut entry = json!({"version":1,"id":key,"label":label,"modes":modes,"theme":theme(&t["theme"]),"source":source});
        if let Some(collection) = t.get("collection") {
            let label = collection["label"].as_str().unwrap_or("").trim();
            if !collection["id"]
                .as_str()
                .is_some_and(|s| ascii_id(s, false))
                || label.is_empty()
                || label.encode_utf16().count() > 64
            {
                return Err(Error::Invalid);
            }
            entry["collection"] = json!({"id":collection["id"],"label":label});
        }
        out.push(entry);
    }
    let mut result = json!({"version":1,"revision":revision(&v["revision"]),"themes":out});
    if !renamed.is_empty() {
        result["legacyRenamedIds"] = json!(renamed);
    }
    Ok(result)
}

fn ascii_id(s: &str, extension: bool) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes().all(|b| {
            (b.is_ascii_lowercase() || b.is_ascii_digit() || (extension && b.is_ascii_uppercase()))
                || b == b'.'
                || b == b'-'
                || b == if extension { b'_' } else { b':' }
        })
        && (extension || s.as_bytes()[0].is_ascii_lowercase() || s.as_bytes()[0].is_ascii_digit())
}
