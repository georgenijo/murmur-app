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
    pub delay_preview: Cell<bool>,
    pub fail_preview: Cell<bool>,
    pub fail_shortcut: Cell<bool>,
    pub delayed: RefCell<Vec<(Request, Inbox)>>,
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
            delay_preview: Cell::new(false),
            fail_preview: Cell::new(false),
            fail_shortcut: Cell::new(false),
            delayed: RefCell::new(vec![]),
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
        if self.delay_preview.get()
            && matches!(
                r.kind,
                RequestKind::Action(Action::Calibration { finish: false, .. })
            )
        {
            self.delayed.borrow_mut().push((r, inbox));
            return;
        }
        self.calls.borrow_mut().push("request");
        let mut next = self.state.borrow().clone();
        let result = (|| {
            if matches!(
                r.kind,
                RequestKind::Action(Action::Calibration { finish: false, .. })
            ) && self.fail_preview.replace(false)
            {
                return Err(Problem::Unavailable);
            }
            if matches!(
                r.kind,
                RequestKind::Patch {
                    key: "doubleTapKey" | "pasteLastShortcut",
                    ..
                }
            ) && self.fail_shortcut.get()
            {
                return Err(Problem::Conflict);
            }
            if let Some(error) = self.fail.take() {
                return Err(error);
            }
            if !matches!(&r.kind, RequestKind::Read) && r.ticket.revision != next.revision {
                return Err(Problem::Conflict);
            }
            let mut source = None;
            match &r.kind {
                RequestKind::Read => (),
                RequestKind::ManualMicrophone { id } => {
                    let mut candidate = next.settings.clone();
                    candidate["microphone"] = json!(id);
                    candidate["microphoneIdMigrationComplete"] = json!(true);
                    candidate["smartAutoMicrophoneEnabled"] = json!(false);
                    let normalized = murmur_core::state::settings(candidate.clone())
                        .map_err(|_| Problem::Invalid)?;
                    if normalized != candidate {
                        return Err(Problem::Invalid);
                    }
                    next.settings = normalized;
                    source = Some("dictation-settings");
                }
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

// This capture surface exists only in tests/gallery builds. It sends ordinary
// Settings edits to the synthetic owner and never installs a global key hook.
impl super::Settings {
    pub(super) fn fixture_capture_buttons(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        use gpui::IntoElement;
        let mut controls = vec![];
        if !self.fixture_shortcuts {
            return controls;
        }
        for (key, label) in [
            ("doubleTapKey", "Capture dictation shortcut (synthetic)"),
            (
                "pasteLastShortcut",
                "Capture Paste last shortcut (synthetic)",
            ),
        ] {
            let e = cx.entity();
            controls.push(
                crate::Button::new(key, label)
                    .caption(if self.fixture_capture == Some(key) {
                        "Press a shortcut; Escape cancels"
                    } else {
                        label
                    })
                    .with_scroll_parent(Some(&self.scroll))
                    .track_focus(&self.focus_for(format!("capture-{key}"), cx))
                    .disabled(self.model.pending)
                    .on_click(move |_, w, cx| {
                        e.update(cx, |s, cx| {
                            s.fixture_capture = Some(key);
                            s.fixture_capture_message = None;
                            w.focus(&s.focus_for(format!("capture-{key}"), cx), cx);
                            cx.notify();
                        })
                    })
                    .into_any_element(),
            );
        }
        if let Some(message) = self.fixture_capture_message {
            use gpui::{InteractiveElement, ParentElement, StatefulInteractiveElement};
            controls.push(
                gpui::div()
                    .id("synthetic-shortcut-status")
                    .role(gpui::Role::Status)
                    .aria_label(message)
                    .child(message)
                    .into_any_element(),
            );
        }
        controls
    }
    pub(super) fn fixture_capture_key(
        &mut self,
        e: &gpui::KeyDownEvent,
        w: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(key) = self.fixture_capture else {
            return false;
        };
        let k = &e.keystroke;
        if k.key == "escape" && k.modifiers.number_of_modifiers() == 0 {
            self.fixture_capture = None;
            self.fixture_capture_message = Some("Synthetic shortcut capture cancelled");
        } else {
            let candidate = if key == "doubleTapKey" && k.modifiers.number_of_modifiers() == 0 {
                Some(k.key.as_str())
            } else if key == "pasteLastShortcut"
                && k.key == "v"
                && k.modifiers.platform
                && k.modifiers.number_of_modifiers() == 2
            {
                if k.modifiers.shift {
                    Some("command_shift_v")
                } else if k.modifiers.alt {
                    Some("command_option_v")
                } else if k.modifiers.control {
                    Some("command_control_v")
                } else {
                    None
                }
            } else {
                None
            };
            let row = *super::schema::ROWS.iter().find(|r| r.key == key).unwrap();
            let value = candidate.and_then(|v| {
                let super::schema::Kind::Choice(options) = row.kind else {
                    return None;
                };
                options
                    .iter()
                    .any(|(option, _)| *option == v)
                    .then(|| row.parse(v))
                    .flatten()
            });
            if let Some(value) = value {
                self.fixture_capture = None;
                self.edit(row, value, w, cx);
                self.fixture_capture_message =
                    Some(if self.model.problem == Some(Problem::Conflict) {
                        "Synthetic shortcut registration conflict; previous binding retained"
                    } else if self.model.problem.is_some() {
                        "Synthetic shortcut registration failed; previous binding retained"
                    } else {
                        "Synthetic shortcut registered"
                    });
            } else {
                self.fixture_capture_message = Some(
                    "Use F1–F20 for dictation, or Command-Shift/Option/Control-V for Paste last",
                );
            }
        }
        cx.notify();
        true
    }
}
