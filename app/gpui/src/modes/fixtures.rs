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
    pub exports: RefCell<Vec<String>>,
    files: bool,
}
impl Default for FixtureHost {
    fn default() -> Self {
        Self::with_files(false)
    }
}
impl FixtureHost {
    pub fn seed_sites(&mut self) {
        let state = self.state.get_mut();
        state.settings["siteModeLookupEnabled"] = serde_json::json!(true);
        state.settings["browserSiteRules"] = serde_json::json!([{"id":"site.synthetic","browserBundleId":"com.apple.Safari","host":"github.com","modeId":"builtin.technical","enabled":true}]);
    }
    pub fn with_files(files: bool) -> Self {
        let mut state = snapshot();
        if files {
            state.settings["appProfiles"] = serde_json::json!([crate::customize::model::profile(
                "example.synthetic",
                "Synthetic editor"
            )]);
        }
        Self {
            state: RefCell::new(state),
            exports: RefCell::new(vec![]),
            files,
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
                    s.notice = Some(super::exchange::reason::APPLIED.into());
                }
                Err(p) => error = Some(p),
            },
            Action::InspectImport if self.files => {
                let mut source = snapshot().settings;
                source["modes"] =
                    serde_json::json!([blank("mode.fixture-file", "Synthetic file mode")]);
                source["appProfiles"] = s.settings["appProfiles"].clone();
                source["appProfiles"][0]["modeId"] = serde_json::json!("mode.fixture-file");
                source["browserSiteRules"] = serde_json::json!([{
                    "id":"site.fixture-file", "browserBundleId": BROWSERS[0].0,
                    "host":"example.com", "modeId":"mode.fixture-file", "enabled":true
                }]);
                let bytes = super::exchange::export(&source).unwrap();
                let file = super::exchange::parse(&bytes).unwrap();
                s.import_preview = Some(super::exchange::preview(&file, &s.settings).unwrap());
            }
            Action::Export if self.files => match super::exchange::export(&s.settings) {
                Ok(bytes) => {
                    self.exports.borrow_mut().push(bytes);
                    s.notice = Some(super::exchange::reason::EXPORTED.into());
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
        if state == "sites" {
            self.selected = "builtin.technical".into();
        }
        if state == "custom" {
            let s = self.controller.model.snapshot.as_mut().unwrap();
            s.settings["modes"] = serde_json::json!([blank("mode.synthetic", "Synthetic mode")]);
            self.selected = "mode.synthetic".into();
        }
        let _ = (w, cx);
    }
}
