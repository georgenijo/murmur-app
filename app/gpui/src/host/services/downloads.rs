//! Each presentation retains the exact native download ID across cancellation.
use super::backend::Backend;
use serde_json::json;
use std::{collections::BTreeMap, sync::Mutex};
#[derive(Clone)]
pub struct Attempt {
    pub id: String,
    pub model: String,
    pub attempt: Option<u64>,
    pub percent: Option<u8>,
    pub phase: Option<String>,
}
impl Attempt {
    pub fn progress(&mut self, value: &serde_json::Value) {
        if value["requestId"] != self.id || value["modelName"] != self.model {
            return;
        }
        let Some(attempt) = value["attemptId"].as_u64() else {
            return;
        };
        if self.attempt.is_some_and(|old| attempt < old) {
            return;
        }
        self.attempt = Some(attempt);
        self.percent = value["received"]
            .as_u64()
            .zip(value["total"].as_u64().filter(|n| *n > 0))
            .map(|(received, total)| ((received as u128 * 100 / total as u128).min(100)) as u8);
        self.phase = value["phase"]
            .as_str()
            .filter(|p| p.len() <= 64)
            .map(String::from);
    }
}
#[derive(Default)]
pub struct Downloads(pub Mutex<BTreeMap<String, Attempt>>);
impl Backend {
    pub fn download(
        &self,
        owner: String,
        model: String,
    ) -> impl std::future::Future<Output = Result<(), String>> + Send + 'static {
        let id = uuid::Uuid::new_v4().to_string();
        self.downloads
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                owner.clone(),
                Attempt {
                    id: id.clone(),
                    model: model.clone(),
                    attempt: None,
                    percent: None,
                    phase: None,
                },
            );
        let b = self.clone();
        async move {
            let result = b
                .call("download_model", json!({"modelName":model,"requestId":id}))
                .await
                .map(|_| ());
            let mut map = b.downloads.0.lock().unwrap_or_else(|p| p.into_inner());
            if map.get(&owner).is_some_and(|a| a.id == id) {
                map.remove(&owner);
            }
            result
        }
    }
    pub async fn cancel_download(&self, owner: &str) -> Result<(), String> {
        let attempt = self
            .downloads
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(owner)
            .cloned();
        if let Some(a) = attempt {
            self.call(
                "cancel_model_download",
                json!({"modelName":a.model,"requestId":a.id}),
            )
            .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_progress_cannot_regress_or_cross_request_ownership() {
        let mut a = Attempt {
            id: "current".into(),
            model: "synthetic".into(),
            attempt: None,
            percent: None,
            phase: None,
        };
        a.progress(&json!({"requestId":"current","modelName":"synthetic","attemptId":3,"received":3,"total":4,"phase":"download"}));
        assert_eq!(a.percent, Some(75));
        a.progress(&json!({"requestId":"current","modelName":"synthetic","attemptId":2,"received":4,"total":4}));
        a.progress(&json!({"requestId":"old","modelName":"synthetic","attemptId":9,"received":4,"total":4}));
        assert_eq!(a.percent, Some(75));
        assert_eq!(a.attempt, Some(3));
        a.progress(&json!({"requestId":"current","modelName":"synthetic","attemptId":4,"received":0,"total":0}));
        assert_eq!(a.percent, None);
    }
}
