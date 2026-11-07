use super::{host::*, model::*};
use crate::section;
use std::{cell::RefCell, rc::Rc};
pub fn snapshot() -> Snapshot {
    let catalog = crate::settings::schema::ROWS
        .iter()
        .find_map(|r| {
            if r.key == "model" {
                if let crate::settings::schema::Kind::Choice(o) = r.kind {
                    Some(
                        o.iter()
                            .map(|(id, label)| ModelInfo {
                                id: (*id).into(),
                                label: (*label).into(),
                                size: "Synthetic size".into(),
                                backend: "Local transcription".into(),
                                accelerator: "Mac".into(),
                                generation: 1,
                                install: if *id == "base.en" {
                                    Install::Installed
                                } else {
                                    Install::Missing
                                },
                                lifecycle: Lifecycle::Unloaded,
                                supported: true,
                                failure: false,
                                capabilities: vec!["Offline".into()],
                            })
                            .collect(),
                    )
                } else {
                    None
                }
            } else {
                None
            }
        })
        .unwrap();
    Snapshot {
        settings_revision: 1,
        revision: 1,
        selected: "parakeet-tdt-0.6b-v3-coreml".into(),
        catalog,
        busy: false,
        attempt: None,
        attempt_model: None,
        percent: None,
        phase: None,
        recommended: Some("parakeet-tdt-0.6b-v3-coreml".into()),
        warned: vec![],
        hardware: Some("Synthetic Apple Silicon · 16 GB".into()),
    }
}
pub struct FixtureHost {
    pub state: RefCell<Snapshot>,
    pub actions: RefCell<Vec<Action>>,
}
impl Default for FixtureHost {
    fn default() -> Self {
        Self {
            state: RefCell::new(snapshot()),
            actions: RefCell::new(vec![]),
        }
    }
}
struct Stop;
impl Subscription for Stop {
    fn stop(&mut self) {}
}
impl SectionHost<Snapshot, Action> for FixtureHost {
    fn subscribe(
        &self,
        _: murmur_core::events::Consumer,
        _: u64,
        _: crate::workspace::Inbox<Snapshot>,
    ) -> Box<dyn Subscription> {
        Box::new(Stop)
    }
    fn request(&self, r: Request, i: crate::workspace::Inbox<Snapshot>) {
        if !r.lease.is_current() {
            return;
        }
        let mut s = self.state.borrow_mut();
        match &r.action {
            Action::Select { id, .. } => s.selected = id.clone(),
            Action::Download(id) => {
                if let Some(m) = s.catalog.iter_mut().find(|m| &m.id == id) {
                    m.install = Install::Installed;
                    m.generation += 1
                }
            }
            Action::Cancel { .. } => {
                s.attempt = None;
                s.attempt_model = None;
                s.percent = None;
            }
            Action::Remove(id) => {
                if let Some(m) = s.catalog.iter_mut().find(|m| &m.id == id) {
                    m.install = Install::Missing;
                    m.generation += 1
                }
            }
            _ => (),
        };
        self.actions.borrow_mut().push(r.action);
        s.revision += 1;
        section::reply(&i, r.ticket, Ok(s.clone()));
    }
}
pub fn host() -> Rc<Host> {
    Rc::new(FixtureHost::default())
}
impl super::Models {
    pub fn fixture_state(
        &mut self,
        state: &str,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) {
        if state == "loading" {
            self.controller.model.snapshot = None;
            return;
        }
        if state == "error" {
            self.controller.model.problem = Some(Problem::Unavailable);
        }
        let s = self.controller.model.snapshot.as_mut().unwrap();
        if state == "busy" {
            s.busy = true
        }
        if state == "download" {
            s.attempt = Some("synthetic-attempt".into());
            s.attempt_model = Some(s.selected.clone());
            s.percent = Some(42);
            s.phase = Some("Downloading…".into());
        }
        if state == "confirm" {
            self.confirm = Some(("base.en".into(), s.revision, std::time::Instant::now()));
        }
    }
}
