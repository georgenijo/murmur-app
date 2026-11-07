//! Passive projections of shared owners. These functions execute on Tokio;
//! they never request permissions, enumerate calendars, download or start capture.
use super::backend::Backend;
use serde_json::{Value, json};
use std::{future::Future, pin::Pin};
type Read<S> = Pin<Box<dyn Future<Output = Result<S, String>> + Send>>;
fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_owned()
}
pub async fn settings(b: &Backend) -> Result<Value, String> {
    b.services
        .settings_snapshot()
        .await
        .map(|(_, settings)| settings)
}
pub fn insights(b: Backend) -> Read<crate::insights::model::Snapshot> {
    Box::pin(async move {
        let c = b.call("completion_snapshot", json!({})).await?;
        let settings = settings(&b).await?;
        let modes = crate::modes::model::builtins()
            .into_iter()
            .chain(settings["modes"].as_array().into_iter().flatten().cloned())
            .filter_map(|m| Some((m["id"].as_str()?.into(), m["name"].as_str()?.into())))
            .collect();
        Ok(crate::insights::model::Snapshot {
            revision: b.events.revision(),
            stats: serde_json::from_value(c["statistics"].clone())
                .map_err(|_| "Statistics unavailable")?,
            today: chrono::Local::now().date_naive(),
            mode_names: modes,
        })
    })
}
pub fn modes(b: Backend) -> Read<crate::modes::model::Snapshot> {
    Box::pin(async move {
        let (revision, settings) = b.services.settings_snapshot().await?;
        Ok(crate::modes::model::Snapshot {
            revision,
            settings,
            import_preview: None,
            probe: None,
            notice: None,
        })
    })
}
pub fn themes(b: Backend) -> Read<crate::themes::model::Snapshot> {
    Box::pin(async move {
        let source = b.sources().await?;
        let library = b.call("load_theme_library_blob", json!({})).await?;
        let library = if let Some(text) = library.as_str() {
            serde_json::from_str(text).map_err(|_| "Theme library unavailable")?
        } else {
            json!({"version":1,"themes":[]})
        };
        let snapshot = crate::themes::model::Snapshot {
            revision: b.events.revision(),
            appearance: murmur_core::state::normalize(
                "murmur-appearance",
                source.get("murmur-appearance").cloned().unwrap_or_else(||json!({"version":1,"revision":0,"mode":"system","theme":{"version":1,"presetId":"sonic"}})),
            )
            .map_err(|_| "Appearance unavailable")?,
            library: murmur_core::state::normalize(
                "murmur-theme-library",
                library,
            )
            .map_err(|_| "Theme library unavailable")?,
            library_available: true,
            results: vec![],
            import_preview: None,
            notice: None,
        };
        *b.theme_snapshot.lock().unwrap_or_else(|p| p.into_inner()) = Some((
            snapshot.clone(),
            source
                .get("murmur-appearance")
                .cloned()
                .unwrap_or(Value::Null),
        ));
        Ok(snapshot)
    })
}
pub fn knowledge(b: Backend) -> Read<crate::knowledge::model::Snapshot> {
    Box::pin(async move {
        let (filter, offset) = b
            .knowledge_page
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let result = b
            .call("list_knowledge", json!({"request":filter.wire(offset)}))
            .await?;
        knowledge_page(&b, result, offset).await
    })
}
pub async fn knowledge_page(
    b: &Backend,
    result: Value,
    offset: usize,
) -> Result<crate::knowledge::model::Snapshot, String> {
    let status = b.call("get_knowledge_store_status", json!({})).await?;
    Ok(crate::knowledge::model::Snapshot {
        revision: b.events.revision(),
        availability: text(&status, "availability"),
        store_revision: result["storeRevision"].as_u64().unwrap_or_default(),
        records: result["entries"].as_array().cloned().unwrap_or_default(),
        total: result["total"].as_u64().unwrap_or_default() as usize,
        offset,
        next: result["nextOffset"].as_u64().map(|n| n as usize),
        import_preview: None,
        bulk: None,
        notice: None,
    })
}
pub fn customize(b: Backend) -> Read<crate::customize::model::Snapshot> {
    Box::pin(async move {
        let (revision, settings) = b.services.settings_snapshot().await?;
        let commands = b
            .call(
                "list_knowledge",
                json!({"request":{"voiceCommand":true,"limit":50,"offset":0}}),
            )
            .await?;
        let mut ide = serde_json::Map::new();
        for profile in settings["appProfiles"].as_array().into_iter().flatten() {
            if profile["ideContextEnabled"] == true
                && let Some(id) = profile["bundleId"].as_str()
            {
                ide.insert(
                    id.into(),
                    b.call("get_ide_context_status", json!({"bundleId":id}))
                        .await?,
                );
            }
        }
        Ok(crate::customize::model::Snapshot {
            revision,
            settings,
            commands: commands["entries"].as_array().cloned().unwrap_or_default(),
            command_total: commands["total"].as_u64().unwrap_or_default() as usize,
            next_offset: commands["nextOffset"].as_u64().map(|n| n as usize),
            running_apps: vec![],
            ide: Value::Object(ide),
            scan: Value::Null,
            preview: None,
            import_preview: None,
        })
    })
}
pub fn models(b: Backend) -> Read<crate::models::model::Snapshot> {
    Box::pin(async move {
        use crate::models::model::{Install, Lifecycle, ModelInfo, Snapshot};
        let (settings_revision, settings) = b.services.settings_snapshot().await?;
        let rows = b.call("get_model_runtime_catalog", json!({})).await?;
        let guidance = b.call("get_model_hardware_guidance", json!({})).await?;
        let status = b.call("get_status", json!({})).await?;
        let catalog = rows
            .as_array()
            .ok_or("Model catalog unavailable")?
            .iter()
            .map(|m| ModelInfo {
                id: text(m, "modelName"),
                label: text(m, "label"),
                size: text(m, "size"),
                backend: text(m, "backend"),
                accelerator: text(m, "accelerator"),
                generation: m["generation"].as_u64().unwrap_or_default(),
                supported: m["supported"].as_bool().unwrap_or(false),
                failure: m["failurePresent"].as_bool().unwrap_or(false),
                capabilities: m["capabilities"]
                    .as_object()
                    .into_iter()
                    .flat_map(|m| m.iter())
                    .filter(|(_, v)| **v == true)
                    .map(|(k, _)| k.clone())
                    .collect(),
                install: match m["installState"].as_str() {
                    Some("installed") => Install::Installed,
                    Some("installing") => Install::Installing,
                    Some("validating") => Install::Validating,
                    Some("invalid") => Install::Invalid,
                    _ => Install::Missing,
                },
                lifecycle: match m["lifecycleState"].as_str() {
                    Some("loading") => Lifecycle::Loading,
                    Some("warming") => Lifecycle::Warming,
                    Some("ready") => Lifecycle::Ready,
                    Some("unloading") => Lifecycle::Unloading,
                    Some("failed") => Lifecycle::Failed,
                    _ => Lifecycle::Unloaded,
                },
            })
            .collect();
        let attempt = b
            .downloads
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .next()
            .cloned();
        Ok(Snapshot {
            settings_revision,
            revision: b.events.revision(),
            selected: text(&settings, "model"),
            catalog,
            busy: status["state"].as_str().is_some_and(|s| s != "idle"),
            attempt: attempt.as_ref().map(|a| a.id.clone()),
            attempt_model: attempt.as_ref().map(|a| a.model.clone()),
            percent: attempt.as_ref().and_then(|a| a.percent),
            phase: attempt.and_then(|a| a.phase),
            recommended: guidance["recommendedModel"].as_str().map(String::from),
            warned: guidance["warnedModels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect(),
            hardware: hardware(&guidance),
        })
    })
}

fn hardware(guidance: &Value) -> Option<String> {
    let chip = guidance["chip"].as_str()?;
    Some(match guidance["physicalMemoryGib"].as_f64() {
        Some(gib) if gib.is_finite() && gib > 0. => format!("{chip} · {gib:.0} GB memory"),
        _ => chip.to_owned(),
    })
}

pub fn access(v: &Value) -> crate::onboarding::model::Access {
    use crate::onboarding::model::Access;
    match v.as_str() {
        Some("granted" | "authorized" | "allowed") => Access::Granted,
        Some("denied") => Access::Denied,
        Some("restricted") => Access::Restricted,
        Some("notDetermined" | "not_determined") => Access::NotDetermined,
        Some("unsupported") => Access::Unsupported,
        _ if v == true => Access::Granted,
        _ if v == false => Access::Denied,
        _ => Access::Unknown,
    }
}
pub async fn permissions(b: &Backend) -> Result<[crate::onboarding::model::Access; 4], String> {
    Ok([
        access(
            &b.call("check_microphone_permission_status", json!({}))
                .await?,
        ),
        access(&b.call("check_accessibility_permission", json!({})).await?),
        access(
            &b.call("get_system_audio_permission_status", json!({}))
                .await?,
        ),
        *b.calendar_access.lock().unwrap_or_else(|p| p.into_inner()), // Only explicit Calendar visits populate this cache.
    ])
}
pub fn onboarding(b: Backend) -> Read<crate::onboarding::model::Snapshot> {
    Box::pin(async move {
        let settings = settings(&b).await?;
        let models = models(b.clone()).await?;
        let source = b.sources().await?;
        Ok(crate::onboarding::model::Snapshot {
            revision: b.events.revision(),
            settings,
            complete: source["murmur_onboarding_complete"] == true,
            permissions: permissions(&b).await?,
            catalog: Some(
                models
                    .catalog
                    .into_iter()
                    .map(|m| crate::onboarding::model::ModelInfo {
                        id: m.id,
                        label: m.label,
                        size: m.size,
                        detail: format!("{} · {}", m.backend, m.accelerator),
                        installed: m.install == crate::models::model::Install::Installed,
                    })
                    .collect(),
            ),
            catalog_error: false,
            download_percent: models.percent,
            installing: models.attempt.is_some(),
            needs_relaunch: false,
            recommended: models.recommended,
            warned_models: models.warned,
            hardware_summary: models.hardware,
        })
    })
}
pub fn home(b: Backend) -> Read<crate::home::model::Snapshot> {
    Box::pin(async move {
        use crate::home::model::{Snapshot, Status};
        let (revision, settings) = b.services.settings_snapshot().await?;
        let sources = b.sources().await?;
        let status = b.call("get_status", json!({})).await?;
        let meetings = b.call("get_meeting_status", json!({})).await?;
        let (_, completion) = b.services.history_snapshot().await?;
        let now = chrono::Local::now();
        let month = now.format("%Y-%m").to_string();
        let (words, recordings) = completion
            .statistics
            .daily_buckets
            .iter()
            .filter(|(date, _)| date.starts_with(&month))
            .fold((0, 0), |(words, recordings), (_, day)| {
                (words + day.words, recordings + day.recordings)
            });
        let phase = match status["state"].as_str() {
            Some("starting") => Status::Starting,
            Some("recording") => Status::Recording,
            Some("processing") => Status::Processing,
            Some("recovering") => Status::Recovering,
            _ => Status::Idle,
        };
        Ok(Snapshot {
            revision: b.events.revision().max(revision),
            settings,
            discovery: sources
                .get("murmur_discovery_v1")
                .cloned()
                .unwrap_or(Value::Null),
            status: phase,
            initialized: status["model"].as_str().is_some_and(|s| !s.is_empty()),
            recording_generation: status["recordingId"].as_u64().filter(|g| *g > 0),
            recording_seconds: status["recordingStartedAtMs"].as_i64().map_or(0, |ms| {
                now.timestamp_millis().saturating_sub(ms).max(0) as u64 / 1000
            }),
            audio_level: 0.,
            meeting_busy: meetings["phase"].as_str().is_some_and(|s| s != "idle"),
            words_this_month: words,
            recordings_this_month: recordings,
            permissions: permissions(&b).await?,
            permissions_checked: true,
            destination: None,
        })
    })
}
pub fn preferences(b: Backend) -> Read<crate::settings::model::Snapshot> {
    Box::pin(async move {
        use crate::settings::model::{Device, Install, Runtime, Snapshot, Update};
        let (revision, settings) = b.services.settings_snapshot().await?;
        let sources = b.sources().await?;
        let inventory = b.call("get_audio_input_inventory", json!({})).await?;
        let models = models(b.clone()).await?;
        let update = b.call("updater_snapshot", json!({})).await?;
        let speakers = b.call("get_diarization_model_status", json!({})).await?;
        let smart_turn = b.call("smart_turn_model_status", json!({})).await?;
        let activity = b.call("get_benchmark_activity", json!({})).await?;
        let preview = b.call("get_microphone_preview_status", json!({})).await?;
        let startup_running = b
            .preview
            .check
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some();
        let login = b.os.login_status().await?;
        let login_requires_approval = login == super::autostart::Registration::RequiresApproval;
        let login_enabled = match login {
            super::autostart::Registration::Enabled
            | super::autostart::Registration::RequiresApproval => Some(true),
            super::autostart::Registration::Disabled => Some(false),
            _ => None,
        };
        let mut runtime = Runtime {
            login_requires_approval,
            login_enabled,
            speakers_supported: speakers["supported"].as_bool(),
            speakers: if speakers["installing"] == true {
                Install::Busy
            } else if speakers["installed"] == true {
                Install::Installed
            } else {
                Install::Missing
            },
            smart_turn: match smart_turn["state"].as_str() {
                Some("ready") => Install::Installed,
                Some("downloading") => Install::Busy,
                _ => Install::Missing,
            },
            lab_running: activity["benchmarkRunning"] == true,
            startup_running,
            monitor_paused: preview["state"] == "stopping",

            recording: models.busy,
            english_only: settings["model"]
                .as_str()
                .is_some_and(|s| s.ends_with(".en")),
            devices: inventory["devices"]
                .as_array()
                .into_iter()
                .flatten()
                .take(32)
                .filter_map(|v| {
                    Some(Device {
                        id: v["id"].as_str()?.into(),
                        label: v["name"].as_str()?.into(),
                        available: v["connected"] == true && v["hasInput"] == true,
                        continuity: v["kind"] == "continuity",
                    })
                })
                .collect(),
            inventory_stale: inventory["status"] != "available",
            version: b.services.version().into(),
            installed_models: Some(
                models
                    .catalog
                    .iter()
                    .filter(|m| m.install == crate::models::model::Install::Installed)
                    .map(|m| m.id.clone())
                    .collect(),
            ),
            model_install: if models.catalog.iter().any(|m| {
                m.id == models.selected && m.install == crate::models::model::Install::Installed
            }) {
                Install::Installed
            } else {
                Install::Missing
            },
            accessibility: Some(
                access(&b.call("check_accessibility_permission", json!({})).await?)
                    == crate::onboarding::model::Access::Granted,
            ),
            notchpill: b.call("is_notchpill_installed", json!({})).await?.as_bool(),
            ..Default::default()
        };
        runtime.update = match update["updateStatus"]["phase"].as_str() {
            Some("checking") => Update::Checking,
            Some("up-to-date") => Update::Current,
            Some("available") => Update::Available,
            Some("downloading") => Update::Downloading,
            Some("ready") => Update::Ready,
            Some("error") => Update::Failed,
            _ => Update::Idle,
        };
        runtime.update_percent = update["updateStatus"]["progress"]
            .as_f64()
            .filter(|v| v.is_finite())
            .map(|v| v.clamp(0., 100.) as u8);
        Ok(Snapshot {
            revision,
            settings,
            disclosure: sources
                .get("murmur.settings-disclosure.v1")
                .cloned()
                .unwrap_or_else(|| json!({"version":1,"panes":{}})),
            runtime,
        })
    })
}
pub fn palette(b: Backend) -> Read<crate::palette::model::Snapshot> {
    Box::pin(async move {
        use crate::palette::registry::{self, Meeting, Recording, State};
        let home = home(b.clone()).await?;
        let modes = b.call("get_mode_runtime_status", json!({})).await?;
        let (_, history) = b.services.history_snapshot().await?;
        let state = State {
            recording: match home.status {
                crate::home::model::Status::Starting => Recording::Starting,
                crate::home::model::Status::Recording => Recording::Recording,
                _ => Recording::Idle,
            },
            meeting: if home.meeting_busy {
                Meeting::Recording
            } else {
                Meeting::Idle
            },
            disabled: home.settings["disabled"] == true,
            has_history: !history.history.is_empty(),
            paste_shortcut: home.settings["pasteLastShortcut"]
                .as_str()
                .map(String::from),
            enabled_modes: modes["available"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| Some((m["id"].as_str()?.into(), m["name"].as_str()?.into())))
                .collect(),
            pending_mode: modes["pending"]["id"].as_str().map(String::from),
        };
        Ok(crate::palette::model::Snapshot {
            revision: b.events.revision(),
            commands: registry::commands(&state),
        })
    })
}
