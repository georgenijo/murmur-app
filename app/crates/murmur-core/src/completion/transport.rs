//! The permanent standalone importer and the native worker use one durable
//! migration protocol. Only transport reads/removes WebKit keys; this owner
//! decides which exact keys have durable destination receipts.
use super::{
    lifecycle::{Operation, TRANSCRIPT_KEYS, UPDATER_V3_KEYS},
    Command, CompletionService, Destination, Error, Result,
};
use serde_json::Value;
pub struct Removal {
    pub keys: Vec<String>,
    pub committed: bool,
}
pub fn import(service: &CompletionService, value: Value) -> Result<Removal> {
    let source = if value.get("error").is_none() {
        serde_json::from_value(value).ok()
    } else {
        None
    };
    let committed =
        source.is_some_and(|source| service.lifecycle(Operation::Import(source)).is_ok());
    let mut keys = service.execute(Command::Snapshot)?.remove_keys;
    if service
        .lifecycle(Operation::Status)
        .is_ok_and(|receipt| receipt.v3)
    {
        keys.extend(UPDATER_V3_KEYS.iter().map(|k| (*k).to_owned()));
    }
    Ok(Removal { keys, committed })
}
pub fn acknowledge(service: &CompletionService, value: &Value) -> Result<()> {
    let keys = value["removed"].as_array().ok_or(Error::Invalid)?;
    let mut destinations = vec![];
    for (destination, required) in [
        (Destination::History, &TRANSCRIPT_KEYS[0..1]),
        (Destination::Statistics, &TRANSCRIPT_KEYS[1..2]),
        (Destination::Reports, &TRANSCRIPT_KEYS[2..4]),
    ] {
        if required
            .iter()
            .all(|key| keys.iter().any(|value| value.as_str() == Some(key)))
        {
            destinations.push(destination);
        }
    }
    service.execute(Command::KeysRemoved(destinations))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::lifecycle::V2_KEYS;
    use super::*;
    use serde_json::json;
    fn source() -> Value {
        let mut source = serde_json::Map::new();
        for key in TRANSCRIPT_KEYS.iter().chain(V2_KEYS).chain(UPDATER_V3_KEYS) {
            source.insert((*key).into(), Value::Null);
        }
        source.insert("dictation-history".into(),json!(json!([{"schemaVersion":2,"id":"synthetic-import","text":"Synthetic fixture only","duration":1,"source":"recording","timestamp":1,"pinned":false}]).to_string()));
        Value::Object(source)
    }
    #[test]
    fn durable_import_replay_and_verified_removal_use_one_owner() {
        let root = tempfile::tempdir().unwrap();
        let service = CompletionService::start(root.path().into(), true).unwrap();
        let imported = import(&service, source()).unwrap();
        assert!(imported.committed);
        assert!(imported.keys.iter().any(|k| k == "dictation-history"));
        assert_eq!(service.execute(Command::Snapshot).unwrap().history.len(), 1);
        assert!(import(&service, source()).unwrap().committed);
        assert_eq!(service.execute(Command::Snapshot).unwrap().history.len(), 1);
        acknowledge(&service, &json!({"removed":["dictation-history"]})).unwrap();
        assert!(
            !service
                .lifecycle(Operation::Status)
                .unwrap()
                .transcript_keys_removed
        );
        acknowledge(&service, &json!({"removed":imported.keys})).unwrap();
        assert!(
            service
                .lifecycle(Operation::Status)
                .unwrap()
                .transcript_keys_removed
        );
    }
    #[test]
    fn malformed_transport_preserves_uncommitted_keys() {
        let root = tempfile::tempdir().unwrap();
        let service = CompletionService::start(root.path().into(), true).unwrap();
        let removal = import(&service, json!({"error":"storage_unreadable"})).unwrap();
        assert!(!removal.committed);
        assert!(removal.keys.is_empty());
        assert!(acknowledge(&service, &json!({"removed":"not-an-array"})).is_err());
        assert!(
            !service
                .lifecycle(Operation::Status)
                .unwrap()
                .transcript_keys_removed
        );
    }
}
