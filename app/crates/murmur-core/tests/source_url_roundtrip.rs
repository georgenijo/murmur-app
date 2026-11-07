//! Shared synthetic parser fixtures must remain readable after publication.
use murmur_core::{
    completion::{lifecycle::Operation, save_source, CompletionService},
    state::normalize,
};
use serde_json::{json, Value};

#[test]
fn accepted_library_fixtures_survive_publication_status_and_restart() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../src/state/appearance-parity-fixtures.json")).unwrap();
    for case in cases {
        if case["valid"] != true {
            continue;
        }
        let key = "murmur-theme-library";
        let input = json!({"version":1,"revision":17,"themes":[case["entry"]]});
        let validated = normalize(key, input.clone()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let published = save_source(dir.path(), key, &input.to_string())
            .unwrap_or_else(|_| panic!("publication failed: {}", case["name"]));
        let published: Value = serde_json::from_str(&published).unwrap();
        assert_eq!(published, validated, "{}", case["name"]);
        for _ in 0..2 {
            let service = CompletionService::start(dir.path().into(), true).unwrap();
            for _ in 0..2 {
                assert_eq!(
                    service.lifecycle(Operation::Status).unwrap().values[key],
                    validated,
                    "{}",
                    case["name"]
                );
            }
            drop(service);
        }
        assert_eq!(normalize(key, published).unwrap(), validated);
        // Publishing the already-validated document must also be idempotent.
        let republished: Value =
            serde_json::from_str(&save_source(dir.path(), key, &validated.to_string()).unwrap())
                .unwrap();
        assert_eq!(normalize(key, republished).unwrap(), validated);
        if case["entry"]["source"]["kind"] == "open-vsx" {
            if let Some(url) = case["entry"]["source"].get("sourceUrl") {
                assert_eq!(validated["themes"][0]["source"]["sourceUrl"], *url);
            }
        }
        assert!(std::fs::read_dir(dir.path()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("quarantine")));
    }
}
