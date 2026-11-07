use super::{exchange, fixtures, model::*};
use serde_json::json;
#[test]
fn builtin_modes_and_frozen_policy_inputs() {
    let b = builtins();
    assert_eq!(b.len(), 7);
    assert_eq!(b[5]["cliFormattingEnabled"], true);
    assert_eq!(preview(" um  hello ", &b[6]), " um  hello ");
    assert_eq!(preview("um draft a note", &b[0]), "Draft a note");
    assert_eq!(
        normalize_host(" GitHub.COM. ").as_deref(),
        Some("github.com")
    );
    assert!(normalize_host("https://github.com").is_none());
}
#[test]
fn deleting_mode_clears_app_and_site_references() {
    let mut s = fixtures::snapshot();
    s.settings["modes"] = json!([blank("mode.test", "Test")]);
    s.settings["appProfiles"] =
        json!([{"bundleId":"example.synthetic","modeId":"mode.test","privateFineTuning":"kept"}]);
    s.settings["browserSiteRules"] = json!([{"id":"site.test","modeId":"mode.test"}]);
    let p = s.patch_without("mode.test");
    assert!(p["appProfiles"][0]["modeId"].is_null());
    assert_eq!(p["appProfiles"][0]["privateFineTuning"], "kept");
    assert_eq!(p["browserSiteRules"], json!([]));
}
#[test]
fn mode_import_is_strict_and_preserves_activation() {
    let mut s = fixtures::snapshot();
    s.settings["modes"] = json!([blank("mode.test", "Test")]);
    let text = exchange::export(&s.settings).unwrap();
    let file = exchange::parse(&text).unwrap();
    let empty = fixtures::snapshot();
    let p = exchange::preview(&file, &empty.settings).unwrap();
    assert_eq!(p.additions, 1);
    let next = exchange::commit(&p, &empty.settings).unwrap();
    assert_eq!(next["siteModeLookupEnabled"], false);
    let mut changed = empty.settings.clone();
    changed["siteModeLookupEnabled"] = json!(true);
    assert!(exchange::commit(&p, &changed).is_err());
    assert!(
        exchange::parse(&text.replace("\"version\": 1", "\"version\":1,\"version\":1")).is_err()
    );
    assert!(exchange::parse("{/*comment*/}").is_err());
}

#[test]
fn mode_and_spelling_exports_exclude_retained_opaque_fields() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../src-tauri/crates/murmur-core/src/state/gpui-downgrade-fixtures.json"
    ))
    .unwrap();
    let mut raw = fixture["settings"].clone();
    raw["browserSiteRules"][0]["modeId"] = json!("synthetic-mode");
    let settings = murmur_core::state::normalize("dictation-settings", raw).unwrap();
    let modes = exchange::export(&settings).unwrap();
    let modes = exchange::parse(&modes).unwrap();
    assert_eq!(modes["modes"][0]["id"], "synthetic-mode");
    assert_eq!(modes["browserSiteRules"][0]["id"], "synthetic-rule");
    assert!(!modes.to_string().contains("future"));
    let spellings = crate::customize::vocabulary::export(&settings).unwrap();
    let spellings = crate::customize::vocabulary::parse(&spellings).unwrap();
    assert!(!spellings.to_string().contains("future"));
    assert!(settings["modes"][0].get("futureMode").is_some());
    assert!(
        settings["vocabularyEntries"][0]["scope"]
            .get("futureScope")
            .is_some()
    );
}
