//! Production presentation adapters. Every future runs on the shared Tokio
//! runtime; views retain no capture, store, transcript pipeline or drain owner.
use super::{
    backend::{Backend, Route},
    os::Live,
    projections as read,
};
use crate::{
    section,
    workspace::{Event, Inbox, Problem},
};
use serde_json::{Value, json};
use std::sync::Arc;
fn live(lease: crate::workspace::Lease) -> Live {
    Arc::new(move || lease.is_current())
}
fn problem<T>(r: Result<T, String>) -> Result<T, Problem> {
    r.map_err(|_| Problem::Unavailable)
}
fn discovery(value: Option<&Value>) -> Value {
    value.cloned().unwrap_or_else(||json!({"version":1,"checklistDismissed":false,"completed":[],"baseline":{"transforms":0,"meetings":0,"correctionsTaught":0,"modeBindings":[]},"pendingHints":[],"dismissedHints":[]}))
}
fn home_route(d: crate::home::model::Destination) -> Route {
    use crate::{home::model::Destination as D, settings::schema::Page};
    match d {
        D::Home => Route::Home,
        D::Meetings => Route::Meetings,
        D::Insights => Route::Insights,
        D::General => Route::Settings(Page::General),
        D::Palette => Route::Palette,
        D::Modes => Route::Modes,
        D::TeachLatest => Route::HistoryAction("teach-latest".into()),
        D::Shortcuts => Route::Settings(Page::Shortcuts),
        D::Setup => Route::SetupReplay,
    }
}
impl crate::home::host::Host for Backend {
    fn subscribe(
        &self,
        c: murmur_core::events::Consumer,
        a: u64,
        inbox: crate::home::inbox::Inbox,
    ) -> Box<dyn crate::home::host::Subscription> {
        self.services
            .waveform_interest(murmur_core::events::WaveformConsumer::MainRecording, true);
        let services = self.services.clone();
        Box::new(
            self.watch(c, read::home, move |snapshot| {
                let _ = inbox.publish(Event::Changed {
                    attachment: a,
                    snapshot,
                });
            })
            .with_cleanup(move || {
                services
                    .waveform_interest(murmur_core::events::WaveformConsumer::MainRecording, false)
            }),
        )
    }
    fn request(&self, r: crate::home::host::Request, inbox: crate::home::inbox::Inbox) {
        use crate::home::host::Action;
        if !r.lease.is_current() || inbox.is_closed() {
            return;
        }
        let b = self.clone();
        let active = b.activity.enter();
        let current = live(r.lease);
        // Admission to the source owner happens before returning to GPUI.
        let accepted = if matches!(
            r.action,
            Action::DismissChecklist | Action::Checklist(_) | Action::Hint { .. }
        ) {
            b.services.accept_work().ok()
        } else {
            None
        };
        self.spawn(async move {
            let _active = active;
            let _accepted = accepted;
            let result = async {
                if !current() && _accepted.is_none() {
                    return Err("Presentation changed".into());
                }
                match r.action {
                    Action::Read | Action::RecheckPermissions => {}
                    Action::Start => b.services.start_idle()?,
                    Action::Stop { generation } => {
                        b.effect(
                            "stop_native_recording",
                            json!({"stopTrigger":"button","expectedRecordingId":generation}),
                            current.clone(),
                        )
                        .await?;
                    }
                    Action::Open(d) => b.navigate(home_route(d)),
                    Action::PermissionSettings(p) => {
                        permission(
                            &b,
                            crate::onboarding::host::Action::OpenSettings(p),
                            current.clone(),
                        )
                        .await?;
                    }
                    Action::ResetPermission(p) => {
                        permission(
                            &b,
                            crate::onboarding::host::Action::Reset(p),
                            current.clone(),
                        )
                        .await?;
                    }
                    action => {
                        let sources = b.sources().await?;
                        let old = sources
                            .get("murmur_discovery_v1")
                            .cloned()
                            .unwrap_or(Value::Null);
                        let mut next = discovery(sources.get("murmur_discovery_v1"));
                        match action {
                            Action::DismissChecklist => next["checklistDismissed"] = json!(true),
                            Action::Checklist(id) => {
                                if matches!(id, "command_palette" | "shortcuts")
                                    && let Some(rows) = next["completed"].as_array_mut()
                                    && !rows.iter().any(|v| v == id)
                                {
                                    rows.push(json!(id));
                                }
                                if let Some(d) = crate::home::model::checklist_destination(id) {
                                    b.navigate(home_route(d));
                                }
                            }
                            Action::Hint { id, open } => {
                                if let Some(rows) = next["pendingHints"].as_array_mut() {
                                    rows.retain(|v| v != id);
                                }
                                if let Some(rows) = next["dismissedHints"].as_array_mut()
                                    && !rows.iter().any(|v| v == id)
                                {
                                    rows.push(json!(id));
                                }
                                if open {
                                    b.navigate(match id {
                                        "double_tap_timing" => Route::Settings(
                                            crate::settings::schema::Page::Recording,
                                        ),
                                        "browser_modes" => Route::Modes,
                                        _ => Route::Home,
                                    });
                                }
                            }
                            _ => {}
                        }
                        b.source("murmur_discovery_v1", old, next).await?;
                    }
                }
                read::home(b.clone()).await
            }
            .await;
            if current() && !inbox.is_closed() {
                let _ = inbox.publish(Event::Reply {
                    ticket: r.ticket,
                    result: problem(result),
                });
            }
        });
    }
}
async fn permission(
    b: &Backend,
    action: crate::onboarding::host::Action,
    current: Live,
) -> Result<Value, String> {
    if !current() {
        return Err("Presentation changed".into());
    }
    let calendar = matches!(
        action,
        crate::onboarding::host::Action::Probe(crate::onboarding::model::Permission::Calendar)
            | crate::onboarding::host::Action::Request(
                crate::onboarding::model::Permission::Calendar
            )
    );
    let result = b
        .effect(action.command(), json!({}), current.clone())
        .await?;
    if calendar && current() {
        *b.calendar_access.lock().unwrap_or_else(|p| p.into_inner()) = read::access(&result);
    }
    Ok(result)
}
impl crate::settings::host::Host for Backend {
    fn subscribe(
        &self,
        c: murmur_core::events::Consumer,
        a: u64,
        inbox: crate::settings::inbox::Inbox,
    ) -> Box<dyn crate::settings::host::Subscription> {
        Box::new(self.watch(c, read::preferences, move |snapshot| {
            let _ = inbox.publish(crate::settings::model::Event::Runtime {
                attachment: a,
                runtime: Box::new(snapshot.runtime.clone()),
            });
            let _ = inbox.publish(crate::settings::model::Event::Changed {
                attachment: a,
                snapshot,
            });
        }))
    }
    fn request(&self, r: crate::settings::host::Request, inbox: crate::settings::inbox::Inbox) {
        use crate::settings::{host::RequestKind, model::Event};
        if !r.lease.is_current() || inbox.is_closed() {
            return;
        }
        let b = self.clone();
        let active = b.activity.enter();
        let current: Live = Arc::new(move || r.lease.is_current());
        let accepted = match &r.kind {
            RequestKind::Patch{key:"launchAtLogin",value}=>{
                let future=b.login.set(value==true);
                let rollback=b.login.restoration(value!=true);
                Some(b.services.accept_settings_effect(json!({"launchAtLogin":value}),r.ticket.revision,false,async move{future.await.map(|_|())},rollback))
            },
            RequestKind::Patch{key,value}=>Some(b.services.accept_settings(json!({*key:value}),r.ticket.revision)),
            RequestKind::ManualMicrophone{id}=>Some(b.services.accept_settings(json!({"microphone":id,"microphoneIdMigrationComplete":true,"smartAutoMicrophoneEnabled":false}),r.ticket.revision)),
            _=>None,
        };
        let durable =
            matches!(r.kind, RequestKind::Disclosure { .. }).then(|| b.services.accept_work());
        self.spawn(async move {
            let _active = active;
            let result = async {
                if let Some(accepted) = accepted {
                    accepted?
                        .await
                        .map_err(|_| "Settings owner unavailable")??;
                } else {
                    let _durable = durable.transpose()?;
                    if !current() && _durable.is_none() {
                        return Err("Presentation changed".into());
                    }
                    match r.kind {
                        RequestKind::Read => {}
                        RequestKind::Disclosure { page, open } => {
                            let sources = b.sources().await?;
                            let old = sources
                                .get("murmur.settings-disclosure.v1")
                                .cloned()
                                .unwrap_or(Value::Null);
                            let mut next = if old.is_null() {
                                json!({"version":1,"panes":{}})
                            } else {
                                old.clone()
                            };
                            next["panes"][page.id()] = json!(open);
                            b.source("murmur.settings-disclosure.v1", old, next).await?;
                        }
                        RequestKind::Action(action) => {
                            settings_action(&b, action, r.ticket.revision, current.clone()).await?
                        }
                        RequestKind::Patch { .. } | RequestKind::ManualMicrophone { .. } => {
                            return Err("Invalid preference admission".into());
                        }
                    }
                }
                read::preferences(b.clone()).await
            }
            .await;
            if current() && !inbox.is_closed() {
                let _ = inbox.publish(Event::Reply {
                    ticket: r.ticket,
                    result: result.map_err(|_| crate::settings::model::Problem::Unavailable),
                });
            }
        });
    }
    fn page(&self, page: Option<crate::settings::schema::Page>, attachment: u64) {
        self.preview_page(page, attachment);
    }
}
fn settings_route(d: crate::settings::host::Destination) -> Route {
    use crate::settings::host::Destination as D;
    match d {
        D::Vocabulary | D::Aliases => {
            Route::CustomizePage(crate::customize::model::Page::Spellings)
        }
        D::VoiceCommands => Route::CustomizePage(crate::customize::model::Page::Commands),
        D::AppOverrides => Route::CustomizePage(crate::customize::model::Page::Apps),
        D::ProjectScan => Route::CustomizePage(crate::customize::model::Page::Scan),
        D::Knowledge => Route::Knowledge,
        D::Modes => Route::Modes,
        D::Appearance => Route::Themes,
        D::ModelStorage => Route::Models,
        D::Diagnostics => Route::Diagnostics,
        D::Setup => Route::SetupReplay,
        D::Checklist => Route::Home,
        D::UpdateDetails => Route::Updates,
    }
}
async fn settings_action(
    b: &Backend,
    action: crate::settings::host::Action,
    revision: u64,
    current: Live,
) -> Result<(), String> {
    use crate::settings::host::Action;
    let (command, args) = match action {
        Action::Open(d) => {
            b.navigate(settings_route(d));
            return Ok(());
        }
        Action::ChooseFolder(key) => {
            if let Some(path) = b
                .panel(json!({"kind":"open","directory":true}), current.clone())
                .await?
                && current()
            {
                b.patch(json!({key:path}), revision).await?;
            }
            return Ok(());
        }
        Action::LoginSettings => {
            b.os.open(
                "x-apple.systempreferences:com.apple.LoginItems-Settings.extension".into(),
                false,
                current,
            )
            .await?;
            return Ok(());
        }
        Action::CalendarSettings => ("open_calendar_preferences", json!({})),
        Action::Accessibility => ("request_accessibility_permission", json!({})),
        Action::CheckUpdate => (
            "updater_action",
            json!({"action":{"kind":"check","background":false}}),
        ),
        Action::DownloadUpdate => ("updater_action", json!({"action":{"kind":"download"}})),
        Action::RestartUpdate => ("updater_action", json!({"action":{"kind":"install"}})),
        Action::InstallSmartTurn => ("download_smart_turn_model", json!({})),
        Action::RemoveSmartTurn => ("remove_smart_turn_model", json!({})),
        Action::InstallSpeakers => {
            if current() {
                b.services.install_speakers().await?;
            }
            return Ok(());
        }
        Action::RemoveSpeakers => ("remove_diarization_model", json!({})),
        Action::PreviewCue(cue) => {
            if current() {
                b.services.preview_cue(match cue {
                    crate::settings::host::Cue::Start => "start",
                    crate::settings::host::Cue::Stop => "stop",
                    crate::settings::host::Cue::Success => "success",
                    crate::settings::host::Cue::Failure => "failure",
                })?;
            }
            return Ok(());
        }
        Action::ResetStatistics => (
            "clear_completion_destination",
            json!({"destination":"statistics"}),
        ),
        Action::RunLab { models } => {
            let report = b
                .effect(
                    "run_benchmark",
                    json!({"request":{"modelNames":models,"preset":"quick","corpus":"bundled"}}),
                    current.clone(),
                )
                .await?;
            if current() {
                b.effect("store_benchmark_report", json!({"report":report}), current)
                    .await?;
            }
            return Ok(());
        }
        Action::CancelLab => ("cancel_benchmark", json!({})),
        Action::ClearReports => (
            "clear_completion_destination",
            json!({"destination":"reports"}),
        ),
        Action::MicrophoneStartup => {
            b.start_microphone_check(current).await?;
            return Ok(());
        }
        Action::CancelMicrophoneStartup => {
            b.cancel_microphone_check().await?;
            return Ok(());
        }
        Action::Calibration { offset, finish } => {
            if current() {
                b.navigate(Route::Calibration {
                    offset: Some(offset),
                    finish,
                });
                if finish {
                    b.patch(json!({"overlayVerticalOffset":offset}), revision)
                        .await?;
                }
            }
            return Ok(());
        }
        Action::CancelCalibration => {
            b.navigate(Route::Calibration {
                offset: None,
                finish: true,
            });
            return Ok(());
        }
        Action::ExportReport(index) => {
            let c = b.call("completion_snapshot", json!({})).await?;
            let report = c["reports"].get(index).ok_or("Report changed")?.clone();
            if let Some(path) = b
                .panel(
                    json!({"kind":"save","name":"murmur-report.json","extensions":["json"]}),
                    current.clone(),
                )
                .await?
                && current()
            {
                b.effect(
                    "save_text_export",
                    json!({"path":path,"contents":report.to_string()}),
                    current,
                )
                .await?;
            }
            return Ok(());
        }
    };
    b.effect(command, args, current).await?;
    Ok(())
}

impl crate::onboarding::host::Host for Backend {
    fn subscribe(
        &self,
        c: murmur_core::events::Consumer,
        a: u64,
        inbox: crate::onboarding::inbox::Inbox,
    ) -> Box<dyn crate::onboarding::host::Subscription> {
        Box::new(self.watch(c, read::onboarding, move |snapshot| {
            let _ = inbox.publish(Event::Changed {
                attachment: a,
                snapshot,
            });
        }))
    }
    fn detach(&self, attachment: u64) {
        let b = self.clone();
        self.spawn(async move {
            let _ = b.cancel_download(&format!("onboarding:{attachment}")).await;
        });
    }
    fn request(&self, r: crate::onboarding::host::Request, inbox: crate::onboarding::inbox::Inbox) {
        use crate::onboarding::host::Action;
        if !r.lease.is_current() || inbox.is_closed() {
            return;
        }
        let b = self.clone();
        let active = b.activity.enter();
        let current = live(r.lease);
        let owner = format!("onboarding:{}", r.ticket.attachment);
        let download = if let Action::Download(model) = &r.action {
            Some(b.download(owner.clone(), model.clone()))
        } else {
            None
        };
        let admitted = matches!(
            r.action,
            Action::Complete { .. } | Action::ResumeExisting | Action::Grandfather
        )
        .then(|| b.services.accept_work());
        let progress_stop = super::backend::ProgressStop::default();
        let finished = progress_stop.0.clone();
        let progress = b.clone();
        let progress_inbox = inbox.clone();
        let progress_live = current.clone();
        if download.is_some() {
            self.spawn(async move {
                while !finished.load(std::sync::atomic::Ordering::Acquire)
                    && progress_live()
                    && !progress_inbox.is_closed()
                {
                    if let Ok(mut snapshot) = read::onboarding(progress.clone()).await {
                        snapshot.revision = r.ticket.revision;
                        let _ = progress_inbox.publish(Event::Progress {
                            ticket: r.ticket,
                            snapshot,
                        });
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            });
        }
        self.spawn(async move {
            let _active = active;
            let _progress_stop = progress_stop;
            let result=async {
                if let Some(download)=download {download.await?;}
                else {
                    if !current() && admitted.is_none(){return Err("Presentation changed".into());}
                    match r.action {
                        Action::Read|Action::Catalog=>{},
                        Action::CancelDownload(_)=>b.cancel_download(&owner).await?,
                        Action::Complete{model,recording_mode,trigger_key}=>{
                            let _admitted=admitted.ok_or("Setup admission unavailable")??;
                            let (revision,settings)=b.services.settings_snapshot().await?;
                            let mut next=settings.clone();
                            for (key,value) in [("model",json!(model)),("recordingMode",json!(recording_mode)),("doubleTapKey",json!(trigger_key)),("settingsVersion",json!(7)),("launchOnboardingComplete",json!(true))] {next[key]=value;}
                            let rows=b.call("get_model_runtime_catalog",json!({})).await?;
                            if !rows.as_array().is_some_and(|rows|rows.iter().any(|m|m["modelName"]==next["model"]&&m["installState"]=="installed")){return Err("Install the selected model first".into());}
                            if read::permissions(&b).await?[0]!=crate::onboarding::model::Access::Granted {return Err("Microphone access is required".into());}
                            // The backend validates/conflict-checks the same options used by
                            // Tauri before enabling shortcut admission or publishing completion.
                            b.services.accept_settings_effect(json!({"model":next["model"],"recordingMode":next["recordingMode"],"doubleTapKey":next["doubleTapKey"],"settingsVersion":7,"launchOnboardingComplete":true}),revision,true,async{Ok(())},async{})?.await.map_err(|_|"Setup owner unavailable")??;
                            finish_setup(&b, false).await?;
                        },
                        Action::ResumeExisting=>{
                            let _admitted=admitted.ok_or("Setup admission unavailable")??;
                            let (revision,_)=b.services.settings_snapshot().await?;
                            b.services.accept_settings_effect(json!({"launchOnboardingComplete":true}),revision,true,async{Ok(())},async{})?.await.map_err(|_|"Setup owner unavailable")??;
                            finish_setup(&b, true).await?;
                        },
                        Action::Grandfather=>{
                            let _admitted=admitted.ok_or("Setup admission unavailable")??;
                            let (revision,_)=b.services.settings_snapshot().await?;
                            b.patch(json!({"launchOnboardingComplete":true}),revision).await?;
                            finish_setup(&b, true).await?;
                        },
                        action=>{permission(&b,action,current.clone()).await?;}
                    }
                }
                read::onboarding(b.clone()).await
            }.await;
            if current()&&!inbox.is_closed(){let _=inbox.publish(Event::Reply{ticket:r.ticket,result:problem(result)});}
        });
    }
}
async fn finish_setup(b: &Backend, grandfather: bool) -> Result<(), String> {
    let sources = b.sources().await?;
    let old = sources
        .get("murmur_onboarding_complete")
        .cloned()
        .unwrap_or(Value::Null);
    if old != true {
        b.source("murmur_onboarding_complete", old, json!(true))
            .await?;
    }
    if !grandfather
        || sources
            .get("murmur_discovery_v1")
            .is_none_or(Value::is_null)
    {
        let (_, completion) = b.services.history_snapshot().await?;
        let stats =
            serde_json::to_value(&completion.statistics).map_err(|_| "Discovery unavailable")?;
        let settings = read::settings(b).await?;
        let bindings: std::collections::BTreeSet<String> = settings["appProfiles"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| {
                let id = p["modeId"].as_str().filter(|s| !s.is_empty())?;
                Some(format!("{}:{id}", p["bundleId"].as_str()?))
            })
            .collect();
        let mut next = discovery(None);
        next["baseline"] = json!({"transforms":stats["activity"]["transforms"]["runs"].as_u64().unwrap_or(0),"meetings":stats["activity"]["meetings"]["count"].as_u64().unwrap_or(0),"correctionsTaught":stats["activity"]["corrections"]["taught"].as_u64().unwrap_or(0),"modeBindings":bindings});
        if grandfather {
            let mut completed = vec![];
            if next["baseline"]["meetings"].as_u64().unwrap_or(0) > 0 {
                completed.push("meeting");
            }
            if next["baseline"]["correctionsTaught"].as_u64().unwrap_or(0) > 0 {
                completed.push("correction");
            }
            if !bindings.is_empty() {
                completed.push("mode_binding");
            }
            next["completed"] = json!(completed);
        }
        b.source(
            "murmur_discovery_v1",
            sources
                .get("murmur_discovery_v1")
                .cloned()
                .unwrap_or(Value::Null),
            next,
        )
        .await?;
    }
    b.navigate(Route::Home);
    Ok(())
}

macro_rules! section_host {
    ($module:ident,$read:ident,$action:ident) => {
        impl section::Host<crate::$module::model::Snapshot, crate::$module::host::Action>
            for Backend
        {
            fn subscribe(
                &self,
                c: murmur_core::events::Consumer,
                a: u64,
                inbox: Inbox<crate::$module::model::Snapshot>,
            ) -> Box<dyn section::Subscription> {
                let cleanup = self.clone();
                Box::new(self.watch(c, read::$read, move |snapshot| {
                    let _ = inbox.publish(Event::Changed {
                        attachment: a,
                        snapshot,
                    });
                }).with_cleanup(move || {
                    if stringify!($module) == "customize" {
                        let b=cleanup.clone();
                        cleanup.spawn(async move {
                            let scan=b.scan.lock().unwrap_or_else(|p|p.into_inner()).take();
                            if let Some((id,_))=scan { let _=b.call("cancel_code_vocab_scan",json!({"scanId":id})).await; }
                        });
                    }
                }))
            }
            fn request(
                &self,
                r: crate::$module::host::Request,
                inbox: Inbox<crate::$module::model::Snapshot>,
            ) {
                if !r.lease.is_current() || inbox.is_closed() {
                    return;
                }
                let b = self.clone();
                let active = b.activity.enter();
                let current = live(r.lease);
                let progress = $action::progress(&b, &r.action, r.ticket, current.clone(), inbox.clone());
                // Capture durable field intent before leaving the main turn.
                let accepted = $action::admit(&b, &r.action, r.ticket.revision);
                self.spawn(async move {
                    let _active = active;
                    let _progress = progress;
                    let result = async {
                        if let Some(accepted) = accepted {
                            accepted?.await.map_err(|_| "Source owner unavailable")??;
                        } else {
                            if !current() {
                                return Err("Presentation changed".into());
                            }
                            if let Some(snapshot) =
                                $action::run(&b, r.action, r.ticket.revision, current.clone())
                                    .await?
                            {
                                return Ok(snapshot);
                            }
                        }
                        read::$read(b.clone()).await
                    }
                    .await;
                    if current() && !inbox.is_closed() {
                        let _ = inbox.publish(Event::Reply {
                            ticket: r.ticket,
                            result: problem(result),
                        });
                    }
                });
            }
        }
    };
}
type Accepted = Option<Result<tokio::sync::oneshot::Receiver<Result<Value, String>>, String>>;
mod model_action {
    use super::*;
    pub fn progress(
        b: &Backend,
        action: &crate::models::host::Action,
        ticket: crate::workspace::Ticket,
        current: Live,
        inbox: Inbox<crate::models::model::Snapshot>,
    ) -> super::super::backend::ProgressStop {
        let stop = super::super::backend::ProgressStop::default();
        if !matches!(action, crate::models::host::Action::Download(_)) {
            return stop;
        }
        let finished = stop.0.clone();
        let backend = b.clone();
        b.spawn(async move {
            while !finished.load(std::sync::atomic::Ordering::Acquire)
                && current()
                && !inbox.is_closed()
            {
                if let Ok(mut snapshot) = read::models(backend.clone()).await {
                    snapshot.revision = ticket.revision;
                    let _ = inbox.publish(Event::Progress { ticket, snapshot });
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        });
        stop
    }

    use crate::models::host::Action;
    pub fn admit(b: &Backend, a: &Action, _r: u64) -> Accepted {
        if let Action::Select {
            id,
            settings_revision,
        } = a
        {
            Some(
                b.services
                    .accept_settings(json!({"model":id}), *settings_revision),
            )
        } else {
            None
        }
    }
    pub async fn run(
        b: &Backend,
        a: Action,
        _revision: u64,
        current: Live,
    ) -> Result<Option<crate::models::model::Snapshot>, String> {
        match a {
            Action::Read | Action::Select { .. } => {}
            Action::Download(id) => b.download("models".into(), id).await?,
            Action::Cancel { model, request_id } => {
                b.effect(
                    "cancel_model_download",
                    json!({"modelName":model,"requestId":request_id}),
                    current,
                )
                .await?;
            }
            Action::Remove(id) => {
                b.effect("remove_model", json!({"modelName":id}), current)
                    .await?;
            }
        }
        Ok(None)
    }
}
section_host!(models, models, model_action);
mod insight_action {
    use super::*;
    pub fn progress(
        _: &Backend,
        _: &crate::insights::host::Action,
        _: crate::workspace::Ticket,
        _: Live,
        _: Inbox<crate::insights::model::Snapshot>,
    ) -> super::super::backend::ProgressStop {
        Default::default()
    }
    pub fn admit(_: &Backend, _: &crate::insights::host::Action, _: u64) -> Accepted {
        None
    }
    pub async fn run(
        b: &Backend,
        a: crate::insights::host::Action,
        _: u64,
        _: Live,
    ) -> Result<Option<crate::insights::model::Snapshot>, String> {
        if a == crate::insights::host::Action::Back {
            b.navigate(Route::Home);
        }
        Ok(None)
    }
}
section_host!(insights, insights, insight_action);

mod mode_action {
    use super::*;
    pub fn progress(
        _: &Backend,
        _: &crate::modes::host::Action,
        _: crate::workspace::Ticket,
        _: Live,
        _: Inbox<crate::modes::model::Snapshot>,
    ) -> super::super::backend::ProgressStop {
        Default::default()
    }
    use crate::modes::{exchange, host::Action, model::Snapshot};
    pub fn admit(b: &Backend, a: &Action, r: u64) -> Accepted {
        match a {
            Action::Patch(p) => Some(b.services.accept_settings(p.clone(), r)),
            Action::Import(preview) => Some(
                exchange::commit(preview, &preview.baseline)
                    .map_err(|_| "Modes changed".to_owned())
                    .and_then(|patch| b.services.accept_settings(patch, r)),
            ),
            _ => None,
        }
    }
    pub async fn run(
        b: &Backend,
        a: Action,
        revision: u64,
        current: Live,
    ) -> Result<Option<Snapshot>, String> {
        let mut snapshot = read::modes(b.clone()).await?;
        match a {
            Action::Read | Action::Patch(_) => {}
            Action::Probe => {
                let result = b.effect("probe_browser_site", json!({}), current).await?;
                snapshot.probe = Some(result["host"].as_str().map_or_else(
                    || "No supported frontmost browser site is available.".into(),
                    String::from,
                ));
                return Ok(Some(snapshot));
            }
            Action::AddApp { mode_id } => b.navigate(Route::BindApp(mode_id)),
            Action::InspectImport => {
                if let Some(path) = b
                    .panel(
                        json!({"kind":"open","extensions":["json"]}),
                        current.clone(),
                    )
                    .await?
                {
                    let contents = b
                        .effect("read_modes_file", json!({"path":path}), current.clone())
                        .await?;
                    if !current() {
                        return Err("Presentation changed".into());
                    }
                    let file = exchange::parse(contents.as_str().ok_or("Invalid Modes file")?)
                        .map_err(|_| "Invalid Modes file")?;
                    snapshot.import_preview = Some(
                        exchange::preview(&file, &snapshot.settings)
                            .map_err(|_| "Modes changed")?,
                    );
                }
                return Ok(Some(snapshot));
            }
            Action::Import(preview) => {
                if snapshot.revision != revision {
                    return Err("Modes changed".into());
                }
                let patch =
                    exchange::commit(&preview, &snapshot.settings).map_err(|_| "Modes changed")?;
                if !current() {
                    return Err("Presentation changed".into());
                }
                b.patch(patch, snapshot.revision).await?;
            }
            Action::Export => {
                let contents =
                    exchange::export(&snapshot.settings).map_err(|_| "Modes export unavailable")?;
                save_text(b, "murmur-modes.json", contents, current).await?;
            }
        }
        Ok(None)
    }
}
section_host!(modes, modes, mode_action);
async fn save_text(b: &Backend, name: &str, contents: String, current: Live) -> Result<(), String> {
    if let Some(path) = b
        .panel(json!({"kind":"save","name":name}), current.clone())
        .await?
    {
        b.effect(
            "save_text_export",
            json!({"path":path,"contents":contents}),
            current,
        )
        .await?;
    }
    Ok(())
}
mod knowledge_action {
    use super::*;
    pub fn progress(
        _: &Backend,
        _: &crate::knowledge::host::Action,
        _: crate::workspace::Ticket,
        _: Live,
        _: Inbox<crate::knowledge::model::Snapshot>,
    ) -> super::super::backend::ProgressStop {
        Default::default()
    }
    use crate::knowledge::{
        host::Action,
        model::{Bulk, BulkAction, Snapshot},
    };
    pub fn admit(b: &Backend, a: &Action, _: u64) -> Accepted {
        if let Action::List { filter, offset } = a {
            *b.knowledge_page.lock().unwrap_or_else(|p| p.into_inner()) = (filter.clone(), *offset);
        }
        None
    }
    pub async fn run(
        b: &Backend,
        a: Action,
        _revision: u64,
        current: Live,
    ) -> Result<Option<Snapshot>, String> {
        let mut snapshot = read::knowledge(b.clone()).await?;
        match a {
            Action::List { filter, offset } => {
                let result = b
                    .call("list_knowledge", json!({"request":filter.wire(offset)}))
                    .await?;
                return Ok(Some(read::knowledge_page(b, result, offset).await?));
            }
            Action::Collect { filter, action } => {
                let mut offset = 0;
                let mut bulk = None;
                loop {
                    if !current() {
                        return Err("Presentation changed".into());
                    }
                    let result = b
                        .call("list_knowledge", json!({"request":filter.wire(offset)}))
                        .await?;
                    let revision = result["storeRevision"]
                        .as_u64()
                        .ok_or("Knowledge unavailable")?;
                    let total = result["total"].as_u64().ok_or("Knowledge unavailable")? as usize;
                    let bulk = bulk.get_or_insert(
                        Bulk::new(filter.clone(), revision, total)
                            .map_err(|_| "Knowledge exceeds supported size")?,
                    );
                    let entries = result["entries"]
                        .as_array()
                        .ok_or("Knowledge unavailable")?
                        .clone();
                    let next = result["nextOffset"].as_u64().map(|n| n as usize);
                    bulk.append(revision, total, entries, next)
                        .map_err(|_| "Knowledge changed")?;
                    if let Some(next) = next {
                        offset = next;
                    } else {
                        break;
                    }
                }
                let bulk = bulk.ok_or("Knowledge unavailable")?;
                if action == BulkAction::Delete {
                    snapshot.bulk = Some(bulk);
                    return Ok(Some(snapshot));
                }
                return apply(b, bulk, action, current).await.map(Some);
            }
            Action::Apply { bulk, action } => {
                return apply(b, bulk, action, current).await.map(Some);
            }
            Action::InspectImport => {
                if let Some(path) = b
                    .panel(
                        json!({"kind":"open","extensions":["json"]}),
                        current.clone(),
                    )
                    .await?
                {
                    if !current() {
                        return Err("Presentation changed".into());
                    }
                    snapshot.import_preview = Some(b.inspect_knowledge(path).await?);
                }
                return Ok(Some(snapshot));
            }
            Action::Import { preview } => {
                if !current() {
                    return Err("Presentation changed".into());
                }
                b.import_knowledge(preview).await?;
            }
            Action::Export => {
                if let Some(path) = b
                    .panel(
                        json!({"kind":"save","name":"murmur-knowledge.json","extensions":["json"]}),
                        current.clone(),
                    )
                    .await?
                {
                    b.effect("export_knowledge_to_file", json!({"path":path}), current)
                        .await?;
                }
            }
            Action::Cancel => {}
            action => {
                if let Some((command, args)) = action.command() {
                    b.effect(command, args, current).await?;
                }
            }
        }
        Ok(None)
    }
    async fn apply(
        b: &Backend,
        bulk: Bulk,
        action: BulkAction,
        current: Live,
    ) -> Result<Snapshot, String> {
        let commands = bulk
            .commands(action)
            .map_err(|_| "Knowledge review is incomplete")?;
        let unchanged = bulk.entries.len().saturating_sub(commands.len());
        let mut succeeded = 0;
        let mut failed = Vec::new();
        for (command, args) in commands {
            if !current() {
                break;
            }
            let id = args["id"].as_str().unwrap_or_default().to_owned();
            if b.effect(command, args, current.clone()).await.is_ok() {
                succeeded += 1;
            } else {
                failed.push(id);
            }
        }
        let mut snapshot = read::knowledge(b.clone()).await?;
        snapshot.notice = Some(format!(
            "{succeeded} changed · {unchanged} unchanged · {} failed{}",
            failed.len(),
            if failed.is_empty() {
                String::new()
            } else {
                format!(
                    " · {}",
                    failed
                        .iter()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        ));
        Ok(snapshot)
    }
}
section_host!(knowledge, knowledge, knowledge_action);

mod customize_action {
    use super::*;
    pub fn progress(
        b: &Backend,
        action: &crate::customize::host::Action,
        ticket: crate::workspace::Ticket,
        current: Live,
        inbox: Inbox<crate::customize::model::Snapshot>,
    ) -> super::super::backend::ProgressStop {
        let stop = super::super::backend::ProgressStop::default();
        let crate::customize::host::Action::Scan { id, .. } = action else {
            return stop;
        };
        *b.scan.lock().unwrap_or_else(|p| p.into_inner()) = Some((id.clone(), Value::Null));
        let id = id.clone();
        let finished = stop.0.clone();
        let backend = b.clone();
        b.spawn(async move {
            while !finished.load(std::sync::atomic::Ordering::Acquire)
                && current()
                && !inbox.is_closed()
            {
                if let Ok(mut snapshot) = read::customize(backend.clone()).await {
                    let progress = backend
                        .scan
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .clone();
                    if let Some((owner, progress)) = progress
                        && owner == id
                    {
                        snapshot.revision = ticket.revision;
                        snapshot.scan = progress;
                        let _ = inbox.publish(Event::Progress { ticket, snapshot });
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        });
        stop
    }
    use crate::customize::{host::Action, model::Snapshot, vocabulary};
    pub fn admit(b: &Backend, a: &Action, r: u64) -> Accepted {
        match a {
            Action::Patch(p) => Some(b.services.accept_settings(p.clone(), r)),
            Action::ImportSpellings(preview) => Some(
                vocabulary::commit(preview, &preview.baseline)
                    .map_err(|_| "Spellings changed".to_owned())
                    .and_then(|patch| b.services.accept_settings(patch, r)),
            ),
            _ => None,
        }
    }
    pub async fn run(
        b: &Backend,
        a: Action,
        revision: u64,
        current: Live,
    ) -> Result<Option<Snapshot>, String> {
        let mut s = read::customize(b.clone()).await?;
        match a {
            Action::Read | Action::Patch(_) => {}
            Action::ChooseRoot(bundle) => {
                if let Some(path) = b
                    .panel(json!({"kind":"open","directory":true}), current.clone())
                    .await?
                {
                    if !current() || s.revision != revision {
                        return Err("App settings changed".into());
                    }
                    let mut profiles = s.settings["appProfiles"]
                        .as_array()
                        .cloned()
                        .ok_or("App settings unavailable")?;
                    let profile = profiles
                        .iter_mut()
                        .find(|p| p["bundleId"] == bundle)
                        .ok_or("App profile removed")?;
                    if profile["ideContextEnabled"] != true {
                        return Err("Enable project context first".into());
                    }
                    let roots = profile["ideProjectRoots"]
                        .as_array_mut()
                        .ok_or("App roots unavailable")?;
                    if roots.len() >= 4 {
                        return Err("At most four project roots are supported".into());
                    }
                    if !roots.iter().any(|v| v == &path) {
                        roots.push(json!(path));
                    }
                    b.patch(json!({"appProfiles":profiles}), s.revision).await?;
                }
            }
            Action::ChooseProject => {
                if let Some(path) = b
                    .panel(json!({"kind":"open","directory":true}), current.clone())
                    .await?
                    && current()
                {
                    b.patch(json!({"codeVocabFolder":path}), s.revision).await?;
                }
            }
            Action::CopyTerms(terms) => {
                if current() {
                    b.services.copy_text(&terms.join("\n"))?;
                }
            }
            Action::InspectSpellings => {
                if let Some(path) = b
                    .panel(
                        json!({"kind":"open","extensions":["json"]}),
                        current.clone(),
                    )
                    .await?
                {
                    let bytes = b
                        .effect("read_modes_file", json!({"path":path}), current.clone())
                        .await?;
                    if !current() {
                        return Err("Presentation changed".into());
                    }
                    let file = vocabulary::parse(bytes.as_str().ok_or("Invalid spelling file")?)
                        .map_err(|_| "Invalid spelling file")?;
                    s.import_preview = Some(
                        vocabulary::preview(&file, &s.settings)
                            .map_err(|_| "Spelling conflicts")?,
                    );
                }
                return Ok(Some(s));
            }
            Action::ImportSpellings(preview) => {
                if s.revision != revision || !current() {
                    return Err("Spellings changed".into());
                }
                let patch =
                    vocabulary::commit(&preview, &s.settings).map_err(|_| "Spellings changed")?;
                b.patch(patch, s.revision).await?;
            }
            Action::ExportSpellings => {
                save_text(
                    b,
                    "murmur-spellings.json",
                    vocabulary::export(&s.settings).map_err(|_| "Spelling export unavailable")?,
                    current,
                )
                .await?
            }
            Action::VoiceCommandSettings => {
                b.navigate(Route::Settings(crate::settings::schema::Page::Recording))
            }
            Action::Cancel => {}
            action => {
                if let Some((command, args)) = action.command() {
                    let result = b.effect(command, args, current.clone()).await?;
                    if !current() {
                        return Err("Presentation changed".into());
                    }
                    match action {
                        Action::Scan { folder, id } => {
                            if result["adopted"] == true {
                                b.patch(
                                    json!({"codeVocabFolder":folder,"codeVocabEnabled":true}),
                                    revision,
                                )
                                .await?;
                            }
                            if b.scan
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .as_ref()
                                .is_some_and(|(owner, _)| owner == &id)
                            {
                                b.scan.lock().unwrap_or_else(|p| p.into_inner()).take();
                            }
                            let mut snapshot = read::customize(b.clone()).await?;
                            snapshot.scan = result;
                            snapshot.scan["id"] = json!(id);
                            snapshot.scan["state"] = json!("done");
                            snapshot.scan["filesRead"] = snapshot.scan["files"].clone();
                            return Ok(Some(snapshot));
                        }
                        Action::RunningApps => {
                            s.running_apps = result
                                .as_array()
                                .cloned()
                                .unwrap_or_default()
                                .into_iter()
                                .take(64)
                                .collect();
                            return Ok(Some(s));
                        }
                        Action::ListCommands { .. } => {
                            s.commands = result["entries"].as_array().cloned().unwrap_or_default();
                            s.command_total = result["total"].as_u64().unwrap_or(0) as usize;
                            s.next_offset = result["nextOffset"].as_u64().map(|v| v as usize);
                            return Ok(Some(s));
                        }
                        Action::PreviewCommand { .. } => {
                            s.preview = result["output"].as_str().map(String::from);
                            return Ok(Some(s));
                        }
                        Action::PreviewAliases { .. } => {
                            s.preview = result
                                .as_str()
                                .map(String::from)
                                .or_else(|| result["text"].as_str().map(String::from));
                            return Ok(Some(s));
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(None)
    }
}
section_host!(customize, customize, customize_action);

mod theme_action {
    use super::*;
    pub fn progress(
        _: &Backend,
        _: &crate::themes::host::Action,
        _: crate::workspace::Ticket,
        _: Live,
        _: Inbox<crate::themes::model::Snapshot>,
    ) -> super::super::backend::ProgressStop {
        Default::default()
    }
    use crate::themes::{host::Action, import, library, model::Snapshot, open_vsx};
    fn map<T: Send + 'static>(
        b: &Backend,
        result: Result<tokio::sync::oneshot::Receiver<Result<T, String>>, String>,
    ) -> Result<tokio::sync::oneshot::Receiver<Result<Value, String>>, String> {
        let receive = result?;
        let (send, reply) = tokio::sync::oneshot::channel();
        let events = b.events.clone();
        b.spawn(async move {
            let result = receive
                .await
                .map_err(|_| "Theme owner unavailable".to_owned())
                .and_then(|r| r)
                .map(|_| Value::Null);
            if result.is_ok() {
                events.publish("main", "native-sources-changed", "null");
            }
            let _ = send.send(result);
        });
        Ok(reply)
    }
    pub fn admit(b: &Backend, action: &Action, revision: u64) -> Accepted {
        if !matches!(
            action,
            Action::Appearance(_)
                | Action::Install(_)
                | Action::ConfirmImport(_)
                | Action::Remove(_)
                | Action::Duplicate(_)
        ) {
            return None;
        }
        Some((|| {
            let (s, raw) = b
                .theme_snapshot
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
                .filter(|(s, _)| s.revision == revision)
                .ok_or("Appearance changed")?;
            if let Action::Appearance(value) = action {
                if value["revision"].as_u64()
                    != Some(library::next_revision(
                        s.appearance["revision"].as_u64().unwrap_or(0),
                    ))
                {
                    return Err("Appearance changed".into());
                }
                return map(
                    b,
                    b.services
                        .accept_source("murmur-appearance".into(), raw, value.clone()),
                );
            }
            let value = match action {
                Action::Install(entries) | Action::ConfirmImport(entries) => {
                    library::install(&s.library, entries, None)
                }
                Action::Remove(ids) => library::remove(&s.library, ids),
                Action::Duplicate(id) => {
                    let mut entry = library::entries(&s.library)
                        .into_iter()
                        .find(|e| e["id"] == *id)
                        .ok_or("Theme removed")?;
                    entry["id"] = json!(format!("local-{}", uuid::Uuid::new_v4()));
                    entry["label"] = json!(
                        format!("{} copy", entry["label"].as_str().unwrap_or("Theme"))
                            .chars()
                            .take(64)
                            .collect::<String>()
                    );
                    entry["source"] = json!({"kind":"local"});
                    entry
                        .as_object_mut()
                        .ok_or("Invalid theme")?
                        .remove("collection");
                    library::install(&s.library, &[entry], None)
                }
                _ => return Err("Invalid theme intent".into()),
            }
            .map_err(|_| "Theme library changed".to_owned())?;
            let library = b.services.accept_theme_library(s.library, value)?;
            if let Action::Remove(ids) = action {
                let mut appearance = s.appearance.clone();
                for mode in ["light", "dark"] {
                    if ids.iter().any(|id| appearance["selection"][mode] == *id) {
                        appearance["selection"][mode] = json!("custom");
                    }
                }
                if appearance != s.appearance {
                    appearance["revision"] = json!(library::next_revision(
                        s.appearance["revision"].as_u64().unwrap_or(0)
                    ));
                    let appearance =
                        b.services
                            .accept_source("murmur-appearance".into(), raw, appearance)?;
                    let (send, receive) = tokio::sync::oneshot::channel();
                    let events = b.events.clone();
                    b.spawn(async move {
                        let result = async {
                            library.await.map_err(|_| "Theme owner unavailable")??;
                            appearance
                                .await
                                .map_err(|_| "Appearance owner unavailable")??;
                            Ok(Value::Null)
                        }
                        .await;
                        events.publish("main", "native-sources-changed", "null");
                        let _ = send.send(result);
                    });
                    return Ok(receive);
                }
            }
            map(b, Ok(library))
        })())
    }
    async fn library_save(b: &Backend, expected: Value, value: Value) -> Result<(), String> {
        b.services
            .accept_theme_library(expected, value)?
            .await
            .map_err(|_| "Theme owner unavailable")??;
        b.events.publish("main", "native-sources-changed", "null");
        Ok(())
    }
    pub async fn run(
        b: &Backend,
        a: Action,
        _revision: u64,
        current: Live,
    ) -> Result<Option<Snapshot>, String> {
        let mut s = read::themes(b.clone()).await?;
        match a {
            Action::Read | Action::Cancel => {}
            Action::Appearance(value) => {
                let sources = b.sources().await?;
                let old = sources
                    .get("murmur-appearance")
                    .cloned()
                    .unwrap_or(Value::Null);
                if value["revision"].as_u64().is_none_or(|n| {
                    n != library::next_revision(s.appearance["revision"].as_u64().unwrap_or(0))
                }) {
                    return Err("Appearance changed".into());
                }
                b.source("murmur-appearance", old, value).await?;
            }
            Action::Install(entries) | Action::ConfirmImport(entries) => {
                let value = library::install(&s.library, &entries, None)
                    .map_err(|_| "Theme library changed")?;
                if !current() {
                    return Err("Presentation changed".into());
                }
                library_save(b, s.library, value).await?;
            }
            Action::Remove(ids) => {
                let value =
                    library::remove(&s.library, &ids).map_err(|_| "Theme library changed")?;
                library_save(b, s.library.clone(), value).await?;
                let mut appearance = s.appearance.clone();
                for mode in ["light", "dark"] {
                    if ids.iter().any(|id| appearance["selection"][mode] == *id) {
                        appearance["selection"][mode] = json!("custom");
                    }
                }
                if appearance != s.appearance {
                    appearance["revision"] = json!(library::next_revision(
                        s.appearance["revision"].as_u64().unwrap_or(0)
                    ));
                    let sources = b.sources().await?;
                    if sources.get("murmur-appearance") == Some(&s.appearance) {
                        b.source("murmur-appearance", s.appearance, appearance)
                            .await?;
                    }
                }
            }
            Action::Duplicate(id) => {
                let mut entry = library::entries(&s.library)
                    .into_iter()
                    .find(|e| e["id"] == id)
                    .ok_or("Theme removed")?;
                entry["id"] = json!(format!("local-{}", uuid::Uuid::new_v4()));
                entry["label"] = json!(
                    format!("{} copy", entry["label"].as_str().unwrap_or("Theme"))
                        .chars()
                        .take(64)
                        .collect::<String>()
                );
                entry["source"] = json!({"kind":"local"});
                entry
                    .as_object_mut()
                    .ok_or("Invalid theme")?
                    .remove("collection");
                let value = library::install(&s.library, &[entry], None)
                    .map_err(|_| "Theme library changed")?;
                library_save(b, s.library, value).await?;
            }
            Action::InspectLocal => {
                if let Some(path) = b
                    .panel(
                        json!({"kind":"open","extensions":["json","jsonc"]}),
                        current.clone(),
                    )
                    .await?
                {
                    let contents = b
                        .effect("read_theme_file", json!({"path":path}), current.clone())
                        .await?;
                    let (label, modes, theme) =
                        import::local(contents.as_str().ok_or("Invalid theme file")?)
                            .map_err(|_| "Invalid theme file")?;
                    s.import_preview = Some(vec![
                        json!({"version":1,"id":format!("local-{}",uuid::Uuid::new_v4()),"label":label,"modes":modes,"theme":theme,"source":{"kind":"local"}}),
                    ]);
                }
                return Ok(Some(s));
            }
            Action::ExportCurrent => {
                save_text(b, "murmur-theme.json", s.appearance.to_string(), current).await?
            }
            Action::ExportEntry(id) => {
                let entry = library::entries(&s.library)
                    .into_iter()
                    .find(|e| e["id"] == id)
                    .ok_or("Theme removed")?;
                save_text(b, "murmur-theme.json", entry.to_string(), current).await?;
            }
            Action::OpenSource(url) => b.os.open(url, false, current).await?,
            Action::Search { query, sort } => {
                let network = open_vsx::Network::new().map_err(|_| "Theme search unavailable")?;
                s.results = open_vsx::search(&network, &query, &sort, &*current)
                    .map_err(|_| "Theme search unavailable")?;
                return Ok(Some(s));
            }
            Action::Add(extension) => {
                let network = open_vsx::Network::new().map_err(|_| "Theme download unavailable")?;
                s.import_preview = Some(
                    open_vsx::download(&network, &extension, &*current)
                        .map_err(|_| "Theme download unavailable")?,
                );
                return Ok(Some(s));
            }
            Action::Update {
                extension,
                expected,
            } => {
                let network = open_vsx::Network::new().map_err(|_| "Theme update unavailable")?;
                let entries = open_vsx::download(&network, &extension, &*current)
                    .map_err(|_| "Theme update unavailable")?;
                if !current() {
                    return Err("Presentation changed".into());
                }
                let value = library::install(
                    &s.library,
                    &entries,
                    Some((&open_vsx::collection(&extension.id), &expected)),
                )
                .map_err(|_| "Theme collection changed")?;
                library_save(b, s.library, value).await?;
            }
        }
        Ok(None)
    }
}
section_host!(themes, themes, theme_action);
mod palette_action {
    use super::*;
    pub fn progress(
        _: &Backend,
        _: &crate::palette::host::Action,
        _: crate::workspace::Ticket,
        _: Live,
        _: Inbox<crate::palette::model::Snapshot>,
    ) -> super::super::backend::ProgressStop {
        Default::default()
    }
    use crate::palette::{host::Action, model::Snapshot};
    pub fn admit(_: &Backend, _: &Action, _: u64) -> Accepted {
        None
    }
    pub async fn run(
        b: &Backend,
        a: Action,
        _: u64,
        current: Live,
    ) -> Result<Option<Snapshot>, String> {
        let Action::Run(id) = a else {
            return Ok(None);
        };
        let registry = read::palette(b.clone()).await?;
        if !registry.commands.iter().any(|c| c.id == id) || !current() {
            return Err("Command changed".into());
        }
        match id.as_str() {
            "recording-toggle" => {
                let status = b.call("get_status", json!({})).await?;
                if status["state"] == "idle" {
                    b.services.start_idle()?;
                } else if let Some(generation) = status["recordingId"].as_u64() {
                    b.effect(
                        "stop_native_recording",
                        json!({"stopTrigger":"command_palette","expectedRecordingId":generation}),
                        current,
                    )
                    .await?;
                }
            }
            "paste-last" => b.services.retry_delivery(),
            "app-disable-toggle" => b.services.toggle_disabled()?,
            "transcribe-file" => b.transcribe_files(current).await?,
            "show-history" | "history-search" | "history-copy-last" | "history-export" => {
                b.navigate(Route::HistoryAction(id))
            }
            "show-meetings" => b.navigate(Route::Meetings),
            "meeting-toggle" => b.navigate(Route::MeetingToggle),
            "check-updates" => {
                b.effect(
                    "updater_action",
                    json!({"action":{"kind":"check","background":false}}),
                    current,
                )
                .await?;
                b.navigate(Route::Updates);
            }
            "about" => b.navigate(Route::About),
            "rerun-setup" => b.navigate(Route::SetupReplay),
            _ if id.starts_with("settings-") => b.navigate(Route::Settings(
                crate::settings::schema::Page::resolve(id.trim_start_matches("settings-")),
            )),
            "next-recording-clear" => {
                let mode = b.call("get_mode_runtime_status", json!({})).await?;
                if !current() {
                    return Err("Presentation changed".into());
                }
                b.services.next_mode(
                    mode["pendingRevision"].as_u64().ok_or("Mode changed")?,
                    None,
                )?;
            }
            _ if id.starts_with("next-recording-") => {
                let mode = b.call("get_mode_runtime_status", json!({})).await?;
                if !current() {
                    return Err("Presentation changed".into());
                }
                b.services.next_mode(
                    mode["pendingRevision"].as_u64().ok_or("Mode changed")?,
                    Some(id.trim_start_matches("next-recording-")),
                )?;
            }
            _ => return Err("Command unavailable".into()),
        }
        Ok(None)
    }
}
section_host!(palette, palette, palette_action);
