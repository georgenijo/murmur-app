//! Synthetic service owners only. Never OS permission, input or pasteboard APIs.
use super::{host::*, inbox::Inbox, model::*, schema::Page};
use serde_json::json;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};
pub fn snapshot() -> Snapshot {
    Snapshot {
        revision: 1,
        settings: murmur_core::state::settings(murmur_core::state::settings_defaults()).unwrap(),
        disclosure: json!({"version":1,"panes":{}}),
        runtime: Runtime {
            devices: vec![
                Device {
                    id: "synthetic.microphone".into(),
                    label: "Synthetic microphone".into(),
                    available: true,
                    continuity: false,
                },
                Device {
                    id: "synthetic.phone".into(),
                    label: "Synthetic phone".into(),
                    available: false,
                    continuity: true,
                },
            ],
            smart_turn: Install::Installed,
            speakers: Install::Missing,
            model_install: Install::Installed,
            accessibility: Some(true),
            notchpill: Some(false),
            installed_models: Some(vec![
                "tiny.en".into(),
                "base.en".into(),
                "parakeet-tdt-0.6b-v3-coreml".into(),
            ]),
            version: "0.62.1-synthetic".into(),
            ..Default::default()
        },
    }
}
pub struct FixtureHost {
    pub state: RefCell<Snapshot>,
    pub calls: RefCell<Vec<&'static str>>,
    pub actions: RefCell<Vec<Action>>,
    pub pages: RefCell<Vec<Option<Page>>>,
    pub stops: Rc<Cell<usize>>,
    pub fail: Cell<Option<Problem>>,
    pub cancel_folder: Cell<bool>,
    pub root: Option<PathBuf>,
}
impl Default for FixtureHost {
    fn default() -> Self {
        Self {
            state: RefCell::new(snapshot()),
            calls: RefCell::new(vec![]),
            actions: RefCell::new(vec![]),
            pages: RefCell::new(vec![]),
            stops: Default::default(),
            fail: Cell::new(None),
            cancel_folder: Cell::new(false),
            root: None,
        }
    }
}
struct Stop(Rc<Cell<usize>>);
impl Subscription for Stop {
    fn stop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}
impl Host for FixtureHost {
    fn subscribe(
        &self,
        c: murmur_core::events::Consumer,
        _: u64,
        _: Inbox,
    ) -> Box<dyn Subscription> {
        assert_eq!(c, murmur_core::events::Consumer::Main);
        self.calls.borrow_mut().push("subscribe");
        Box::new(Stop(self.stops.clone()))
    }
    fn page(&self, page: Option<Page>, _: u64) {
        self.pages.borrow_mut().push(page);
    }
    fn request(&self, r: Request, inbox: Inbox) {
        if !r.lease.is_current() || inbox.is_closed() {
            return;
        }
        self.calls.borrow_mut().push("request");
        let mut next = self.state.borrow().clone();
        let result = (|| {
            if let Some(error) = self.fail.take() {
                return Err(error);
            }
            if !matches!(&r.kind, RequestKind::Read) && r.ticket.revision != next.revision {
                return Err(Problem::Conflict);
            }
            let mut source = None;
            match &r.kind {
                RequestKind::Read => (),
                RequestKind::Patch { key, value } => {
                    next.settings =
                        murmur_core::state::settings_edit(&next.settings, key, value.clone())
                            .map_err(|_| Problem::Invalid)?;
                    source = Some("dictation-settings");
                }
                RequestKind::Disclosure { page, open } => {
                    next.disclosure["panes"][page.id()] = json!(open);
                    source = Some("murmur.settings-disclosure.v1");
                }
                RequestKind::Action(action) => {
                    self.actions.borrow_mut().push(action.clone());
                    match action {
                        Action::ChooseFolder(key) if !self.cancel_folder.get() => {
                            next.settings[*key] = json!("/synthetic/chosen-folder");
                            source = Some("dictation-settings");
                        }
                        Action::MicrophoneStartup => next.runtime.startup_running = true,
                        Action::CancelMicrophoneStartup => next.runtime.startup_running = false,
                        Action::InstallSmartTurn => next.runtime.smart_turn = Install::Installed,
                        Action::RemoveSmartTurn => next.runtime.smart_turn = Install::Missing,
                        Action::InstallSpeakers => next.runtime.speakers = Install::Installed,
                        Action::RemoveSpeakers => next.runtime.speakers = Install::Missing,
                        Action::CheckUpdate => next.runtime.update = Update::Current,
                        Action::DownloadUpdate => next.runtime.update = Update::Ready,
                        Action::RunLab { models } => {
                            if models.is_empty() || next.runtime.recording {
                                return Err(Problem::Denied);
                            }
                            next.runtime.lab_running = true;
                            next.runtime.lab_progress = Some(0);
                        }
                        Action::CancelLab => {
                            next.runtime.lab_running = false;
                            next.runtime.lab_progress = None;
                        }
                        Action::ClearReports => next.runtime.reports.clear(),
                        Action::CancelCalibration => next.runtime.calibration = None,
                        Action::Calibration {
                            offset,
                            finish: false,
                        } => {
                            if !(-12..=12).contains(offset) {
                                return Err(Problem::Invalid);
                            }
                            let (min, max) = next
                                .runtime
                                .calibration
                                .map(|c| (c.min, c.max))
                                .unwrap_or((-12, 12));
                            next.runtime.calibration = Some(Calibration {
                                requested: *offset,
                                applied: (*offset).clamp(min, max),
                                min,
                                max,
                            });
                        }
                        Action::Calibration {
                            offset,
                            finish: true,
                        } => {
                            if !(-12..=12).contains(offset) {
                                return Err(Problem::Invalid);
                            }
                            next.settings["overlayVerticalOffset"] = json!(offset);
                            next.runtime.calibration = None;
                            source = Some("dictation-settings");
                        }
                        _ => (),
                    }
                }
            }
            // Exercise the real durable core path only in explicitly supplied test roots.
            if let (Some(root), Some(key)) = (&self.root, source) {
                if !r.lease.is_current() {
                    return Err(Problem::Conflict);
                }
                if key == "dictation-settings" {
                    let _guard = murmur_core::completion::lifecycle::SOURCE_WRITE_LOCK
                        .lock()
                        .unwrap();
                    murmur_core::completion::save_source(root, key, &next.settings.to_string())
                        .map_err(|_| Problem::Unavailable)?;
                } else {
                    let service =
                        murmur_core::completion::CompletionService::start(root.clone(), false)
                            .map_err(|_| Problem::Unavailable)?;
                    service
                        .lifecycle(murmur_core::completion::lifecycle::Operation::Save {
                            key: key.into(),
                            value: next.disclosure.clone(),
                        })
                        .map_err(|_| Problem::Unavailable)?;
                }
            }

            if !matches!(r.kind, RequestKind::Read) {
                next.revision += 1;
            }
            *self.state.borrow_mut() = next.clone();
            Ok(next)
        })();
        let _ = inbox.publish(Event::Reply {
            ticket: r.ticket,
            result,
        });
    }
}
