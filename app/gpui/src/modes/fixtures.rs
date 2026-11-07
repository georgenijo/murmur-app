use super::{host::*, model::*};
use crate::section;
use std::{cell::RefCell, rc::Rc};
pub fn snapshot() -> Snapshot {
    Snapshot {
        revision: 1,
        settings: murmur_core::state::settings(murmur_core::state::settings_defaults()).unwrap(),
        import_preview: None,
        probe: None,
        notice: None,
    }
}
pub struct FixtureHost {
    pub state: RefCell<Snapshot>,
}
impl Default for FixtureHost {
    fn default() -> Self {
        Self {
            state: RefCell::new(snapshot()),
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
        let mut error = None;
        match r.action {
            Action::Patch(patch) => {
                let mut next = s.settings.clone();
                for (k, v) in patch.as_object().unwrap() {
                    next[k] = v.clone()
                }
                match murmur_core::state::normalize("dictation-settings", next) {
                    Ok(next) => s.settings = next,
                    Err(_) => error = Some(Problem::Invalid),
                }
            }
            Action::Probe => {
                s.probe = Some("No supported frontmost browser site is available.".into())
            }
            Action::Import(p) => match super::exchange::commit(&p, &s.settings) {
                Ok(patch) => {
                    for (k, v) in patch.as_object().unwrap() {
                        s.settings[k] = v.clone()
                    }
                    s.import_preview = None;
                }
                Err(p) => error = Some(p),
            },
            _ => (),
        };
        s.revision += 1;
        section::reply(&i, r.ticket, error.map_or_else(|| Ok(s.clone()), Err));
    }
}
pub fn host() -> Rc<Host> {
    Rc::new(FixtureHost::default())
}
impl super::Modes {
    pub fn fixture_state(
        &mut self,
        state: &str,
        w: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if state == "loading" {
            self.controller.model.snapshot = None;
            return;
        }
        if state == "error" {
            self.controller.model.problem = Some(Problem::Unavailable);
        }
        if state == "custom" || state == "sites" {
            let s = self.controller.model.snapshot.as_mut().unwrap();
            s.settings["modes"] = serde_json::json!([blank("mode.synthetic", "Synthetic mode")]);
            self.selected = "mode.synthetic".into();
            if state == "sites" {
                s.settings["siteModeLookupEnabled"] = serde_json::json!(true);
            }
        }
        let _ = (w, cx);
    }
}
