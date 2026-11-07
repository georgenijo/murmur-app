//! Import confirmation owns the exact reviewed bytes, not a mutable path.
use super::backend::Backend;
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Read, sync::Mutex};
#[derive(Default)]
pub struct Reviews(Mutex<BTreeMap<String, (Vec<u8>, u64)>>);
impl Backend {
    pub async fn inspect_knowledge(&self, path: String) -> Result<Value, String> {
        let file = std::fs::File::open(path).map_err(|_| "Import unavailable")?;
        let mut bytes = Vec::new();
        file.take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Import unavailable")?;
        let (mut summary, revision) = self.services.inspect_knowledge(&bytes)?;
        let token = uuid::Uuid::new_v4().to_string();
        summary["token"] = json!(token);
        let mut reviews = self
            .knowledge_reviews
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        reviews.clear();
        reviews.insert(token, (bytes, revision));
        Ok(summary)
    }
    pub async fn import_knowledge(&self, preview: Value) -> Result<(), String> {
        if preview["conflicts"] != 0 {
            return Err("Resolve import conflicts first".into());
        }
        let token = preview["token"]
            .as_str()
            .ok_or("Import review unavailable")?;
        let (bytes, revision) = self
            .knowledge_reviews
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(token)
            .ok_or("Import review expired")?;
        self.services.import_knowledge(&bytes, revision)?;
        self.events
            .publish("main", "native-services-changed", "null");
        Ok(())
    }
}
