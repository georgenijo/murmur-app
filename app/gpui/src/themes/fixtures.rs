use super::{host::*, library, model::*};
use crate::section;
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};
pub fn midnight() -> Value {
    json!({"version":1,"id":"midnight","label":"Midnight","modes":["dark"],"theme":{"version":1,"presetId":"custom","dark":{"background":"#000000","surface":"#000000","surface-container":"#10121a","on-surface":"#f5f5f5","on-surface-variant":"#b7b8d0","outline-variant":"#53566e","primary":"#74a9d8","on-primary":"#140d2a"}},"source":{"kind":"local"}})
}
pub fn imported() -> Value {
    let(_,dark,theme)=super::import::vscode(&json!({"name":"Synthetic Orchid","type":"dark","colors":{"editor.background":"#20152b","editor.foreground":"#f7efff","focusBorder":"#d3a3ff","sideBar.background":"#251c35"}})).unwrap();
    json!({"version":1,"id":"synthetic-orchid","label":"Synthetic Orchid","modes":[if dark{"dark"}else{"light"}],"theme":theme,"source":{"kind":"local"}})
}
pub fn snapshot() -> Snapshot {
    let library = json!({"version":1,"revision":1,"themes":[midnight(),imported()]});
    Snapshot {
        revision: 1,
        appearance: json!({"version":1,"revision":1,"mode":"system","theme":{"version":1,"presetId":"sonic"},"selection":{"light":"sonic","dark":"sonic"}}),
        library,
        library_available: true,
        results: vec![],
        import_preview: None,
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
            Action::Appearance(doc) => s.appearance = doc,
            Action::Install(entries) | Action::ConfirmImport(entries) => {
                match library::install(&s.library, &entries, None) {
                    Ok(v) => {
                        s.library = v;
                        s.import_preview = None
                    }
                    Err(p) => error = Some(p),
                }
            }
            Action::Remove(ids) => match library::remove(&s.library, &ids) {
                Ok(v) => {
                    s.library = v;
                    for mode in ["light", "dark"] {
                        if ids.iter().any(|id| s.appearance["selection"][mode] == *id) {
                            s.appearance["selection"][mode] = json!("custom");
                        }
                    }
                }
                Err(p) => error = Some(p),
            },
            Action::Duplicate(id) => {
                if let Some(mut entry) = library::entries(&s.library)
                    .into_iter()
                    .find(|v| v["id"] == id)
                {
                    entry["id"] = json!(format!("copy-{}", s.revision));
                    entry["label"] = json!(format!("{} copy", entry["label"].as_str().unwrap()));
                    entry["source"] = json!({"kind":"local"});
                    entry.as_object_mut().unwrap().remove("collection");
                    match library::install(&s.library, &[entry], None) {
                        Ok(v) => s.library = v,
                        Err(p) => error = Some(p),
                    }
                }
            }
            Action::InspectLocal | Action::Add(_) => {
                s.import_preview = Some(vec![
                    json!({"version":1,"id":"imported-fixture","label":"Imported fixture","modes":["dark"],"theme":{"version":1,"presetId":"custom","accent":"#aa77ee"},"source":{"kind":"local"}}),
                ])
            }
            Action::ExportCurrent => {
                s.notice = Some("Fixture exported current appearance. No file was written.".into())
            }
            Action::Cancel => s.import_preview = None,
            Action::Search { .. } => {
                s.notice = Some("Fixture search completed. No network request was made.".into())
            }
            _ => (),
        };
        s.revision += 1;
        section::reply(&i, r.ticket, error.map_or_else(|| Ok(s.clone()), Err));
    }
}
pub fn host() -> Rc<Host> {
    Rc::new(FixtureHost::default())
}
impl super::Themes {
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
        if state == "unavailable" {
            self.controller
                .model
                .snapshot
                .as_mut()
                .unwrap()
                .library_available = false;
        }
        match state {
            "edit" => self.dialog(super::Dialog::Edit { create: true }, w, cx),
            "community" => self.dialog(super::Dialog::Community, w, cx),
            "import" => self.dialog(super::Dialog::Preview(vec![imported()]), w, cx),
            _ => (),
        }
    }
}
