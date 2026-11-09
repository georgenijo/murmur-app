use super::{host::*, model::*, vocabulary};
use crate::remaining::Scenario;
use crate::section;
use serde_json::json;
use std::{cell::RefCell, rc::Rc};
pub fn snapshot() -> Snapshot {
    let mut settings =
        murmur_core::state::settings(murmur_core::state::settings_defaults()).unwrap();
    settings["vocabularyEntries"] = json!([{"id":"synthetic-spelling","written":"SyntheticTerm","aliases":["synthetic term"],"enabled":true,"scope":{"kind":"global"}}]);
    settings["codeVocabFolder"] = json!("/synthetic/project");
    settings = murmur_core::state::settings(settings).unwrap();
    Snapshot {
        revision: 1,
        settings,
        commands: vec![
            json!({"id":"synthetic-command","revision":1,"payload":{"kind":"snippet","trigger":"synthetic greeting","body":"Hello {{date}}"},"voiceCommand":{"commandType":"snippet","allowClipboardRead":false},"enabled":true,"scope":{"kind":"global"}}),
        ],
        command_total: 1,
        next_offset: None,
        running_apps: vec![],
        ide: json!({}),
        scan: json!({"state":"idle","filesRead":0,"terms":0,"rankedTerms":[]}),
        preview: None,
        preview_note: None,
        import_preview: None,
    }
}
pub struct FixtureHost {
    pub state: RefCell<Snapshot>,
    pub actions: RefCell<Vec<Action>>,
    scenario: Scenario,
}
impl Default for FixtureHost {
    fn default() -> Self {
        Self::with_scenario(Scenario::Ready)
    }
}
impl FixtureHost {
    pub fn with_scenario(scenario: Scenario) -> Self {
        Self {
            state: RefCell::new(snapshot()),
            actions: RefCell::new(vec![]),
            scenario,
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
        if self.scenario == Scenario::HeldScan && matches!(r.action, Action::Scan { .. }) {
            return;
        }
        let mut s = self.state.borrow_mut();
        let mut problem = None;
        match &r.action {
            Action::Patch(patch) => {
                let mut next = s.settings.clone();
                for (k, v) in patch.as_object().unwrap() {
                    next[k] = v.clone()
                }
                match murmur_core::state::normalize("dictation-settings", next) {
                    Ok(v) => s.settings = v,
                    Err(_) => problem = Some(Problem::Invalid),
                }
            }
            Action::Upsert(draft) => {
                let mut entry = draft.clone();
                entry["id"] = draft
                    .get("id")
                    .cloned()
                    .unwrap_or(json!("new-synthetic-command"));
                entry["revision"] = json!(draft["expectedRevision"].as_u64().unwrap_or(0) + 1);
                s.commands.retain(|v| v["id"] != entry["id"]);
                s.commands.push(entry);
                s.command_total = s.commands.len();
            }
            Action::Delete(entry) => {
                s.commands.retain(|v| v["id"] != entry["id"]);
                s.command_total = s.commands.len();
            }
            Action::Enable { entry, enabled } => {
                if let Some(v) = s
                    .commands
                    .iter_mut()
                    .find(|v| v["id"] == entry["id"] && v["revision"] == entry["revision"])
                {
                    v["enabled"] = json!(enabled);
                    v["revision"] = json!(v["revision"].as_u64().unwrap() + 1)
                } else {
                    problem = Some(Problem::Conflict)
                }
            }
            Action::PreviewCommand { read_clipboard, .. } => {
                s.preview = Some(
                    if *read_clipboard {
                        "Synthetic preview; fixture never reads the pasteboard."
                    } else {
                        "Synthetic command preview (no clipboard read)."
                    }
                    .into(),
                );
            }
            Action::PreviewAliases { .. } => s.preview = Some("npm run SyntheticTerm dev".into()),
            Action::RunningApps => {
                s.running_apps =
                    vec![json!({"bundleId":"example.synthetic","name":"Synthetic editor"})]
            }
            Action::ChooseRoot(bundle) => {
                for p in s.settings["appProfiles"].as_array_mut().unwrap() {
                    if p["bundleId"] == *bundle {
                        p["ideProjectRoots"] = json!(["/synthetic/project"])
                    }
                }
            }
            Action::RefreshIde(bundle) => {
                s.ide[bundle] = json!({"state":"ready","files":12,"symbols":25})
            }
            Action::ClearIde(bundle) => {
                s.ide[bundle] = json!({"state":"cleared","files":0,"symbols":0})
            }
            Action::Scan { id, .. } | Action::ChooseProject { id } => {
                s.scan = json!({"id":id,"state":"done","filesRead":12,"terms":2,"rankedTerms":[{"term":"SyntheticTerm","freq":4},{"term":"FixtureSymbol","freq":2}]})
            }
            Action::ClearProject => {
                s.settings["codeVocabFolder"] = json!("");
                s.settings["codeVocabLastScan"] = json!(null);
                s.scan = json!(null);
            }
            Action::CancelScan(_) => s.scan = json!(null),
            Action::InspectSpellings => {
                let alias = if self.scenario == Scenario::ConflictingImport {
                    "synthetic term"
                } else {
                    "fixture name"
                };
                let v = json!({"format":"murmur-vocabulary","version":1,"entries":[{"id":"import-spelling","written":"FixtureName","aliases":[alias],"enabled":true,"scope":{"kind":"global"}}]});
                // Exercise the same strict parser as a real import without opening a panel.
                let v = vocabulary::parse(&v.to_string()).unwrap();
                s.import_preview = Some(vocabulary::preview(&v, &s.settings).unwrap());
                if self.scenario == Scenario::StaleImport {
                    s.settings["vocabularyEntries"][0]["written"] = json!("SyntheticTermUpdated");
                }
            }
            Action::ImportSpellings(p) => match vocabulary::commit(p, &s.settings) {
                Ok(v) => {
                    s.settings["vocabularyEntries"] = v["vocabularyEntries"].clone();
                    s.import_preview = None
                }
                Err(p) => problem = Some(p),
            },
            Action::Cancel => {
                s.import_preview = None;
                s.preview = None
            }
            _ => (),
        };
        s.settings = murmur_core::state::settings(s.settings.clone()).unwrap();
        s.revision += 1;
        self.actions.borrow_mut().push(r.action);
        section::reply(&i, r.ticket, problem.map_or_else(|| Ok(s.clone()), Err));
    }
}
pub fn host() -> Rc<Host> {
    Rc::new(FixtureHost::default())
}
impl super::Customize {
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
        if state == "edit" {
            self.open(serde_json::Value::Null, w, cx);
        }
        if state == "scanning" {
            let s = self.controller.model.snapshot.as_mut().unwrap();
            s.scan = json!({"state":"scanning","id":"synthetic-scan","filesRead":32,"terms":12,"rankedTerms":[]});
        }
        if state == "preview" {
            self.controller.model.snapshot.as_mut().unwrap().preview =
                Some("Synthetic local preview".into());
        }
    }
}
