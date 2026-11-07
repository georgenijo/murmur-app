use super::{allowed, unique_strings, with_unknown};
use serde_json::{json, Map, Value};
fn keep_unknown(preserve: bool, original: &Value, normalized: Value, known: &[&str]) -> Value {
    if preserve {
        with_unknown(original, normalized, known)
    } else {
        normalized
    }
}
fn text(v: &Value) -> String {
    v.as_str().unwrap_or("").trim().into()
}
fn nullable(v: &Value) -> Value {
    if v.is_boolean() {
        v.clone()
    } else {
        Value::Null
    }
}
fn style(v: &Value) -> Value {
    if allowed(
        v,
        &[
            "conversational",
            "polished",
            "code_technical",
            "verbatim",
            "notes",
        ],
    ) {
        v.clone()
    } else {
        Value::Null
    }
}
fn rows(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}
fn truncate(s: &str) -> String {
    s.chars().take(256).collect()
}
pub(super) fn normalize(p: &mut Map<String, Value>, raw: &Value, catalog: &Value, preserve: bool) {
    let profiles: Vec<_> = rows(&p["appProfiles"]).filter(|v| !text(&v["bundleId"]).is_empty()).map(|v| {
        let mut out=json!({"bundleId":text(&v["bundleId"]),"label":v["label"].as_str().unwrap_or(""),"writingStyle":style(&v["writingStyle"]),"ideContextEnabled":v["ideContextEnabled"]==true,"ideProjectRoots":rows(&unique_strings(&v["ideProjectRoots"],usize::MAX)).filter(|v| v.as_str().unwrap().encode_utf16().count()<=4096).take(4).cloned().collect::<Vec<_>>()});
        for k in ["autoPasteOverride","cleanupOverride","smartFormattingOverride","cliFormattingOverride"] { out[k]=nullable(&v[k]); }
        if !text(&v["modeId"]).is_empty() { out["modeId"]=json!(text(&v["modeId"])); }
        keep_unknown(preserve, v, out, &["modeId", "persistedSymbols", "scanResults"])
    }).collect();
    p.insert("appProfiles".into(), json!(profiles));
    let mut ids = catalog["builtinModes"].as_array().unwrap().clone();
    let mut modes = vec![];
    for v in rows(&p["modes"]).take(100) {
        let id = text(&v["id"]);
        let name = text(&v["name"]);
        if id.is_empty()
            || id.encode_utf16().count() > 128
            || name.is_empty()
            || name.encode_utf16().count() > 128
            || ids.contains(&json!(id))
            || v["builtIn"] == true
        {
            continue;
        }
        if [("modelId", "models"), ("language", "languages")]
            .iter()
            .any(|(k, l)| !v[k].is_null() && !catalog[l].as_array().unwrap().contains(&v[k]))
        {
            continue;
        }
        if !v["writingStyle"].is_null() && style(&v["writingStyle"]).is_null() {
            continue;
        }
        if [
            "enabled",
            "cleanupEnabled",
            "smartFormattingEnabled",
            "cliFormattingEnabled",
            "autoPaste",
        ]
        .iter()
        .any(|k| !v[k].is_null() && !v[k].is_boolean())
        {
            continue;
        }
        if !v["vocabularyPolicy"].is_null()
            && !allowed(&v["vocabularyPolicy"], &["inherit", "general", "technical"])
        {
            continue;
        }
        if !v["contextPolicy"].is_null() && !allowed(&v["contextPolicy"], &["none", "project"]) {
            continue;
        }
        let mut out = json!({"id":id,"name":name,"builtIn":false,"enabled":v["enabled"]!=false,"writingStyle":style(&v["writingStyle"]),"vocabularyPolicy":v["vocabularyPolicy"].as_str().unwrap_or("inherit"),"contextPolicy":v["contextPolicy"].as_str().unwrap_or("none"),"modelId":v["modelId"],"language":v["language"]});
        for k in [
            "cleanupEnabled",
            "smartFormattingEnabled",
            "cliFormattingEnabled",
            "autoPaste",
        ] {
            out[k] = nullable(&v[k]);
        }
        ids.push(json!(id));
        modes.push(keep_unknown(preserve, v, out, &[]));
    }
    if !catalog["builtinModes"]
        .as_array()
        .unwrap()
        .contains(&p["activeModeId"])
        && !modes
            .iter()
            .any(|m| m["id"] == p["activeModeId"] && m["enabled"] == true)
    {
        p.insert("activeModeId".into(), json!("builtin.everyday"));
    }
    p.insert("modes".into(), json!(modes));
    let browsers = [
        "com.apple.Safari",
        "com.google.Chrome",
        "com.brave.Browser",
        "com.microsoft.edgemac",
        "org.chromium.Chromium",
        "company.thebrowser.Browser",
    ];
    let mut rules = vec![];
    let mut rule_ids = vec![];
    let mut sites = vec![];
    for v in rows(&p["browserSiteRules"]).take(128) {
        let id = text(&v["id"]);
        let host = text(&v["host"]).to_lowercase();
        let host = host.strip_suffix('.').unwrap_or(&host).to_string();
        let site = (text(&v["browserBundleId"]), host.clone());
        if id.is_empty()
            || id.encode_utf16().count() > 128
            || rule_ids.contains(&id)
            || sites.contains(&site)
            || !valid_host(&host)
            || !allowed(&v["browserBundleId"], &browsers)
            || !ids.contains(&v["modeId"])
        {
            continue;
        }
        rule_ids.push(id.clone());
        sites.push(site);
        rules.push(keep_unknown(preserve, v, json!({"id":id,"browserBundleId":v["browserBundleId"],"host":host,"modeId":v["modeId"],"enabled":v["enabled"]!=false}), &[]));
    }
    p.insert("browserSiteRules".into(), json!(rules));
    p.insert("voiceCommands".into(),json!(rows(&p["voiceCommands"]).filter(|v| !text(&v["phrase"]).is_empty()).map(|v| keep_unknown(preserve, v, json!({"phrase":text(&v["phrase"]),"replacement":v["replacement"].as_str().unwrap_or("")}), &[])).collect::<Vec<_>>()));
    let vocab = if raw["vocabularyEntries"].is_array() {
        rows(&raw["vocabularyEntries"]).filter(|v| !text(&v["written"]).is_empty()).enumerate().take(500).map(|(i,v)| {
        let mut scope=json!({"kind":"global"});let s=&v["scope"];
        if allowed(&s["kind"],&["app","project"]) && !text(&s["bundleId"]).is_empty() && (s["kind"]!="project" || !text(&s["root"]).is_empty()) { scope=json!({"kind":s["kind"],"bundleId":text(&s["bundleId"])});if s["kind"]=="project" { scope["root"]=json!(text(&s["root"])); } }
        scope=keep_unknown(preserve, s, scope, &["kind", "bundleId", "root"]);
        let mut aliases=vec![];
        for a in rows(&v["aliases"]).filter_map(Value::as_str) { let a=truncate(a.trim()); if !a.is_empty() && !aliases.iter().any(|v: &String| v.eq_ignore_ascii_case(&a)) && aliases.len()<16 { aliases.push(a); } }
        keep_unknown(preserve, v, json!({"id":if text(&v["id"]).is_empty() {format!("vocabulary-{i}")} else {v["id"].as_str().unwrap().into()},"written":truncate(&text(&v["written"])),"aliases":aliases,"enabled":v["enabled"].as_bool().unwrap_or(true),"scope":scope}), &[])
    }).collect::<Vec<_>>()
    } else {
        p["customVocabulary"].as_str().unwrap_or("").split([',','\r','\n']).map(str::trim).filter(|s| !s.is_empty()).take(500).enumerate().map(|(i,s)| json!({"id":format!("legacy-{i}"),"written":truncate(s),"aliases":[],"enabled":true,"scope":{"kind":"global"}})).collect()
    };
    p.insert(
        "customVocabulary".into(),
        json!(vocab
            .iter()
            .filter(|v| v["enabled"] == true && v["scope"]["kind"] == "global")
            .filter_map(|v| v["written"].as_str())
            .collect::<Vec<_>>()
            .join(", ")),
    );
    p.insert("vocabularyEntries".into(), json!(vocab));
    let scan = &p["codeVocabLastScan"];
    let scan = if scan.is_object()
        && ["files", "skipped", "terms", "bytes", "ms"]
            .iter()
            .all(|k| scan[k].is_number())
        && scan["capped"].is_boolean()
        && scan["sampleTerms"].is_array()
    {
        let ranked:Vec<_>=rows(&scan["rankedTerms"]).filter(|v| v["term"].as_str().is_some_and(|s| !s.is_empty()) && v["freq"].is_number()).take(500).map(|v| keep_unknown(preserve, v, json!({"term":v["term"],"freq":v["freq"].as_f64().unwrap().max(0.).trunc() as u64}), &[])).collect();
        let mut out = json!({"capped":scan["capped"],"sampleTerms":rows(&scan["sampleTerms"]).filter(|v| v.is_string()).take(50).collect::<Vec<_>>(),"rankedTerms":ranked,"whisperCount":scan["whisperCount"].as_f64().unwrap_or(0.).clamp(0.,ranked.len() as f64).trunc() as u64,"adopted":scan["adopted"].as_bool().unwrap_or(true)});
        for k in ["files", "skipped", "terms", "bytes", "ms"] {
            out[k] = json!(scan[k].as_f64().unwrap().max(0.).trunc() as u64);
        }
        // Legacy scan payloads can contain an accidental copy of the settings
        // document. Those known preference keys were always discarded here;
        // preserve only genuinely opaque scan metadata alongside scan fields.
        let defaults = super::settings_defaults();
        let known: Vec<_> = defaults
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keep_unknown(preserve, scan, out, &known)
    } else {
        Value::Null
    };
    p.insert("codeVocabLastScan".into(), scan);
}

fn valid_host(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 || host.contains(':') {
        return false;
    }
    if host == "localhost" {
        return true;
    }
    let labels: Vec<_> = host.split('.').collect();
    labels.len() > 1
        && labels.iter().all(|s| {
            !s.is_empty()
                && s.len() <= 63
                && s.as_bytes()[0].is_ascii_alphanumeric()
                && s.as_bytes()[s.len() - 1].is_ascii_alphanumeric()
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}
