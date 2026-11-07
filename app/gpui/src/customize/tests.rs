use super::{fixtures, model::*, vocabulary};
use serde_json::json;
#[test]
fn command_permissions_are_separate_and_duplication_has_blank_phrase() {
    let e = fixtures::snapshot().commands.remove(0);
    let copy = duplicate_command(&e);
    assert_eq!(copy["payload"]["trigger"], "");
    assert_eq!(copy["voiceCommand"]["allowClipboardRead"], false);
    assert!(copy.get("id").is_none());
    let mut v = json!({"kind":"snippet","phrase":"greet","content":"Hello {{clipboard}}","scope":"global","enabled":"true","allowClipboard":"false"});
    assert!(command_draft(&v, None).is_err());
    v["allowClipboard"] = json!("true");
    let d = command_draft(&v, None).unwrap();
    assert_eq!(d["voiceCommand"]["allowClipboardRead"], true);
    v["content"] = json!("{{execute}}");
    assert!(command_draft(&v, None).is_err());
}
#[test]
fn spelling_exchange_rejects_conflicts_and_stale_previews() {
    let s = fixtures::snapshot();
    let text = vocabulary::export(&s.settings).unwrap();
    let file = vocabulary::parse(&text).unwrap();
    let p = vocabulary::preview(&file, &s.settings).unwrap();
    assert_eq!(p.duplicates, 1);
    let mut changed = s.settings.clone();
    changed["voiceCommands"] = json!([{"phrase":"synthetic term","replacement":"other"}]);
    assert!(vocabulary::commit(&p, &changed).is_err());
    assert!(
        vocabulary::parse(&text.replace("\"version\": 1", "\"version\":1,\"version\":1")).is_err()
    );
    let mut entries = s.settings["vocabularyEntries"].as_array().unwrap().clone();
    entries[0]["aliases"] = json!(["new line"]);
    assert!(vocabulary::validate(&entries, &[]).is_err());
    entries[0]["aliases"] = json!(["SyntheticTerm"]);
    assert!(vocabulary::validate(&entries, &[]).is_err());
}
#[test]
fn default_profile_has_no_context_or_delivery_opt_in() {
    let p = profile("example.synthetic", "Synthetic editor");
    assert_eq!(p["ideContextEnabled"], false);
    assert!(p["autoPasteOverride"].is_null());
    assert_eq!(p["ideProjectRoots"], json!([]));
}
