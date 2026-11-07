//! Synthetic host; no OS permission, audio, user storage or pasteboard APIs.
use super::{host::*, inbox::Inbox, model::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
pub fn snapshot() -> Snapshot {
    let mut settings = murmur_core::state::settings_defaults();
    settings = murmur_core::state::settings(settings).unwrap();
    Snapshot {
        revision: 1,
        settings,
        complete: false,
        permissions: [
            Access::NotDetermined,
            Access::Denied,
            Access::Unknown,
            Access::NotDetermined,
        ],
        catalog: Some(
            [
                (
                    "parakeet-tdt-0.6b-v3-coreml",
                    "Parakeet Core ML",
                    "~470 MB",
                    "Fastest on Apple Silicon — multilingual, Apple Neural Engine",
                ),
                ("tiny.en", "Whisper Tiny (English)", "~75 MB", ""),
                (
                    "base.en",
                    "Whisper Base (English)",
                    "~150 MB",
                    "Good balance of speed and accuracy",
                ),
                ("small.en", "Whisper Small (English)", "~500 MB", ""),
                ("medium.en", "Whisper Medium (English)", "~1.5 GB", ""),
                (
                    "large-v3-turbo",
                    "Whisper Large Turbo",
                    "~1.6 GB",
                    "Highest accuracy, slower (1-2 seconds)",
                ),
                (
                    "parakeet-tdt-0.6b-v2-fp16",
                    "Parakeet TDT 0.6B (English, fast)",
                    "~1.2 GB",
                    "Fast CPU fallback — English only",
                ),
            ]
            .into_iter()
            .map(|(id, label, size, detail)| ModelInfo {
                id: id.into(),
                label: label.into(),
                size: size.into(),
                detail: detail.into(),
                installed: id == "tiny.en",
            })
            .collect(),
        ),
        catalog_error: false,
        download_percent: None,
        installing: false,
        needs_relaunch: false,
        recommended: Some(CURATED[0].into()),
        warned_models: vec![],
        hardware_summary: None,
    }
}
pub struct FixtureHost {
    pub state: RefCell<Snapshot>,
    pub actions: RefCell<Vec<Action>>,
    pub calls: RefCell<Vec<&'static str>>,
    pub stops: Rc<Cell<usize>>,
    pub fail: Cell<bool>,
    pub hold: Cell<bool>,
    pub hold_cancel: Cell<bool>,
    pub held: RefCell<Vec<(Request, Inbox)>>,
    pub root: Option<std::path::PathBuf>,
}
impl Default for FixtureHost {
    fn default() -> Self {
        Self {
            state: RefCell::new(snapshot()),
            actions: RefCell::new(vec![]),
            calls: RefCell::new(vec![]),
            stops: Default::default(),
            fail: Cell::new(false),
            hold: Cell::new(false),
            hold_cancel: Cell::new(false),
            held: RefCell::new(vec![]),
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
    fn detach(&self, _: u64) {
        self.calls.borrow_mut().push("detach");
    }
    fn request(&self, r: Request, inbox: Inbox) {
        if !r.lease.is_current() || inbox.is_closed() {
            return;
        }
        self.calls.borrow_mut().push("request");
        self.actions.borrow_mut().push(r.action.clone());
        if self.hold.get()
            || self.hold_cancel.get() && matches!(r.action, Action::CancelDownload(_))
        {
            self.held.borrow_mut().push((r, inbox));
            return;
        }
        let result = (|| {
            if self.fail.replace(false) {
                return Err(Problem::Unavailable);
            }
            let mut s = self.state.borrow().clone();
            if !matches!(r.action, Action::Read) && r.ticket.revision != s.revision {
                return Err(Problem::Conflict);
            }
            match &r.action {
                Action::Request(p) => s.permissions[p.index()] = Access::Granted,
                Action::Reset(p) => s.permissions[p.index()] = Access::NotDetermined,
                Action::Download(id) => {
                    if let Some(m) = s
                        .catalog
                        .as_mut()
                        .and_then(|c| c.iter_mut().find(|m| &m.id == id))
                    {
                        m.installed = true;
                    } else {
                        return Err(Problem::Invalid);
                    }
                }
                Action::Complete {
                    model,
                    recording_mode,
                    trigger_key,
                } => {
                    if !s.installed(model) {
                        return Err(Problem::Unavailable);
                    }
                    for (key, value) in [
                        ("model", model),
                        ("recordingMode", recording_mode),
                        ("doubleTapKey", trigger_key),
                    ] {
                        s.settings = murmur_core::state::settings_edit(
                            &s.settings,
                            key,
                            serde_json::json!(value),
                        )
                        .map_err(|_| Problem::Invalid)?;
                    }
                    s.settings["launchOnboardingComplete"] = serde_json::json!(true);
                    s.complete = true;
                }
                Action::ResumeExisting | Action::Grandfather => {
                    s.complete = true;
                    s.settings["launchOnboardingComplete"] = serde_json::json!(true);
                }
                _ => (),
            }
            if let Some(root) = &self.root
                && matches!(r.action, Action::Complete { .. })
            {
                if !r.lease.is_current() {
                    return Err(Problem::Conflict);
                }
                {
                    let _guard = murmur_core::completion::lifecycle::SOURCE_WRITE_LOCK
                        .lock()
                        .unwrap();
                    murmur_core::completion::save_source(
                        root,
                        "dictation-settings",
                        &s.settings.to_string(),
                    )
                    .map_err(|_| Problem::Unavailable)?;
                }
                let owner = murmur_core::completion::CompletionService::start(root.clone(), false)
                    .map_err(|_| Problem::Unavailable)?;
                for (key, value) in [
                    ("murmur_onboarding_complete", serde_json::json!(true)),
                    (
                        "murmur_discovery_v1",
                        crate::home::fixtures::snapshot().discovery,
                    ),
                ] {
                    owner
                        .lifecycle(murmur_core::completion::lifecycle::Operation::Save {
                            key: key.into(),
                            value,
                        })
                        .map_err(|_| Problem::Unavailable)?;
                }
            }
            if !matches!(r.action, Action::Read) {
                s.revision += 1;
            }
            *self.state.borrow_mut() = s.clone();
            Ok(s)
        })();
        let _ = inbox.publish(Event::Reply {
            ticket: r.ticket,
            result,
        });
    }
}
