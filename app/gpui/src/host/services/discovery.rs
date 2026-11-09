//! Production writer for the discovery checklist and hints. Every decision is
//! `murmur_core::discovery`; this module only reads the owners, applies the
//! decision with a compare-and-set source write and retries a lost race.
//! Without an existing state document nothing is written, as in the shipped app.
use super::{backend::Backend, projections as read};
use murmur_core::discovery as core;
use serde_json::Value;
use std::sync::atomic::Ordering;

const ATTEMPTS: usize = 3;

/// Reads the source, applies `change` and stores the result. `None` from
/// `change` (or a missing state) means there is nothing to write.
pub async fn mutate(b: &Backend, change: impl Fn(&Value) -> Option<Value>) -> Result<(), String> {
    for _ in 0..ATTEMPTS {
        let sources = b.sources().await?;
        let Some(old) = sources.get(core::SOURCE).filter(|v| !v.is_null()) else {
            return Ok(());
        };
        let Some(next) = change(old) else {
            return Ok(());
        };
        if b.source(core::SOURCE, old.clone(), next).await.is_ok() {
            return Ok(());
        }
    }
    Err("Discovery changed concurrently".into())
}

/// Marks a checklist item done after the user actually used the feature.
pub fn complete(b: &Backend, id: &'static str) {
    let b = b.clone();
    b.clone().spawn(async move {
        let _ = mutate(&b, |s| core::complete(s, id)).await;
    });
}

pub async fn evidence(b: &Backend) -> Result<core::Evidence, String> {
    let (_, completion) = b.services.history_snapshot().await?;
    let statistics =
        serde_json::to_value(&completion.statistics).map_err(|_| "Discovery unavailable")?;
    Ok(core::Evidence::read(&statistics, &read::settings(b).await?))
}
/// Follows local usage: meetings, taught corrections and mode bindings.
pub async fn sync(b: &Backend) -> Result<(), String> {
    let evidence = evidence(b).await?;
    mutate(b, |s| core::sync(s, &evidence)).await
}
async fn newest(b: &Backend) -> Result<Value, String> {
    let (_, completion) = b.services.history_snapshot().await?;
    // History is oldest first.
    Ok(completion
        .history
        .last()
        .map_or(Value::Null, |entry| entry["id"].clone()))
}
/// Called once at startup so the entry that already exists does not teach.
pub async fn prime(b: &Backend) {
    if let Ok(id) = newest(b).await {
        *b.newest_entry.lock().unwrap_or_else(|p| p.into_inner()) = id;
    }
    let _ = sync(b).await;
}

/// Reacts to a Main event. Each branch is independent and best effort.
pub fn observe(b: &Backend, event: &str, payload: &str) {
    if !b.booted.load(Ordering::Acquire) {
        return;
    }
    let hint = core::hint_for_event(event, payload);
    let history = event == "history-changed";
    let usage = matches!(event, "statistics-changed" | "dictation-settings-changed");
    if hint.is_none() && !history && !usage {
        return;
    }
    let b = b.clone();
    b.clone().spawn(async move {
        if let Some(id) = hint {
            let _ = mutate(&b, |s| core::enqueue_hint(s, id)).await;
        }
        if history && let Ok(id) = newest(&b).await {
            let fresh = {
                let mut seen = b.newest_entry.lock().unwrap_or_else(|p| p.into_inner());
                let fresh = !id.is_null() && *seen != id;
                *seen = id;
                fresh
            };
            if fresh {
                let _ = mutate(&b, |s| core::enqueue_hint(s, "correct_and_teach")).await;
            }
        }
        if usage {
            let _ = sync(&b).await;
        }
    });
}
