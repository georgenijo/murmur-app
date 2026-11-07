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
