use super::{fixtures, model::*};
use serde_json::json;
#[test]
fn collect_all_before_write_and_refuse_page_drift() {
    let mut b = Bulk::new(Filter::default(), 8, 51).unwrap();
    b.append(8, 51, (0..50).map(fixtures::entry).collect(), Some(50))
        .unwrap();
    assert!(b.commands(BulkAction::Delete).is_err());
    assert!(b.append(9, 51, vec![fixtures::entry(50)], None).is_err());
    assert_eq!(b.entries.len(), 50);
    b.append(8, 51, vec![fixtures::entry(50)], None).unwrap();
    assert_eq!(b.commands(BulkAction::Delete).unwrap().len(), 51);
    assert_eq!(
        b.commands(BulkAction::Disable).unwrap()[0].1["expectedRevision"],
        1
    );
    assert!(b.commands(BulkAction::Enable).unwrap().is_empty());
}
#[test]
fn duplicate_ids_and_incomplete_terminal_are_rejected() {
    let mut b = Bulk::new(Filter::default(), 1, 2).unwrap();
    assert!(
        b.append(1, 2, vec![fixtures::entry(0), fixtures::entry(0)], None)
            .is_err()
    );
    assert!(b.append(1, 2, vec![fixtures::entry(0)], None).is_err());
    assert!(b.entries.is_empty());
}
#[test]
fn drafts_preserve_scope_and_require_content() {
    let mut v = fixtures::entry(1);
    assert!(valid_record(&v));
    v["scope"] = json!({"kind":"project","bundleId":"example.synthetic"});
    assert!(valid_draft(&v).is_err());
    v["scope"]["root"] = json!("/synthetic/project");
    assert!(valid_draft(&v).is_ok());
    v["payload"]["replacement"] = json!("");
    assert!(valid_draft(&v).is_err());
}

#[test]
fn editor_details_round_trip_empty_and_comma_containing_aliases() {
    for aliases in [json!([]), json!(["Tori", "Tory", "Tor, ee"])] {
        let entry =
            json!({"payload":{"kind":"vocabulary_term","written":"Tori","aliases":aliases}});
        assert_eq!(
            json!(editor_detail(&entry).lines().collect::<Vec<_>>()),
            aliases
        );
    }
    for (kind, key) in [
        ("replacement_rule", "replacement"),
        ("snippet", "body"),
        ("transform", "instruction"),
    ] {
        let entry = json!({"payload":{"kind":kind,key:"First\nSecond"}});
        assert_eq!(editor_detail(&entry), detail(&entry));
    }
}
