use super::{host::*, model::*};
use crate::remaining::Scenario;
use crate::section;
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};
pub fn entry(i: usize) -> Value {
    json!({"id":format!("synthetic-{i}"),"revision":1,"enabled":true,"payload":{"kind":"replacement_rule","source":format!("Synthetic phrase {i}"),"replacement":format!("Synthetic replacement {i}")},"scope":{"kind":"global"},"provenance":"manual","createdAtMs":0,"updatedAtMs":0,"voiceCommand":null})
}
pub fn snapshot() -> Snapshot {
    Snapshot {
        revision: 1,
        availability: "ready".into(),
        store_revision: 1,
        records: (0..3).map(entry).collect(),
        total: 3,
        offset: 0,
        next: None,
        import_preview: None,
        bulk: None,
        notice: None,
    }
}
pub struct FixtureHost {
    pub state: RefCell<Snapshot>,
    pub actions: RefCell<Vec<Action>>,
    scenario: Scenario,
    import_baseline: RefCell<Option<u64>>,
}
impl Default for FixtureHost {
    fn default() -> Self {
        Self::with_scenario(Scenario::Ready)
    }
}
impl FixtureHost {
    pub fn with_scenario(scenario: Scenario) -> Self {
        let mut state = snapshot();
        if scenario == Scenario::PartialBulk {
            state.availability = "recovered".into();
        }
        Self {
            state: RefCell::new(state),
            actions: RefCell::new(vec![]),
            scenario,
            import_baseline: RefCell::new(None),
        }
    }
    fn apply_bulk(&self, s: &mut Snapshot, bulk: &Bulk, action: BulkAction) -> Result<(), Problem> {
        if bulk.revision != s.store_revision {
            return Err(Problem::Conflict);
        }
        let commands = bulk.commands(action)?;
        let unchanged = bulk.entries.len() - commands.len();
        let mut succeeded = 0;
        let mut failures = vec![];
        for (_, command) in commands {
            if self.scenario == Scenario::PartialBulk && command["id"] == "synthetic-1" {
                failures.push("synthetic-1");
                continue;
            }
            if action == BulkAction::Delete {
                s.records.retain(|v| v["id"] != command["id"]);
            } else if let Some(record) = s.records.iter_mut().find(|v| v["id"] == command["id"]) {
                record["enabled"] = json!(action == BulkAction::Enable);
                record["revision"] = json!(record["revision"].as_u64().unwrap() + 1);
            }
            succeeded += 1;
        }
        s.total = s.records.len();
        if succeeded > 0 {
            s.store_revision += 1;
        }
        s.notice = Some(format!(
            "{succeeded} succeeded · {unchanged} unchanged · {} failed{}",
            failures.len(),
            if failures.is_empty() {
                String::new()
            } else {
                format!(
                    " · Failed IDs: {}. Review these records before retrying.",
                    failures.join(", ")
                )
            }
        ));
        Ok(())
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
        s.bulk = None;
        s.import_preview = None;
        let mut problem = None;
        match &r.action {
            Action::List { filter, offset } => {
                s.offset = *offset;
                s.records = (0..3)
                    .map(entry)
                    .filter(|v| {
                        title(v)
                            .to_lowercase()
                            .contains(&filter.query.to_lowercase())
                            && filter.enabled.is_none_or(|e| v["enabled"] == e)
                            && filter
                                .kind
                                .as_ref()
                                .is_none_or(|k| v["payload"]["kind"] == *k)
                    })
                    .collect();
                s.total = s.records.len();
            }
            Action::Upsert(v) => {
                if valid_draft(v).is_err() {
                    problem = Some(Problem::Invalid)
                } else {
                    let mut v = v.clone();
                    v["id"] = v.get("id").cloned().unwrap_or(json!("synthetic-new"));
                    v["revision"] = json!(v["expectedRevision"].as_u64().unwrap_or(0) + 1);
                    let id = v["id"].clone();
                    s.records.retain(|p| p["id"] != id);
                    s.records.push(v);
                    s.total = s.records.len();
                    s.store_revision += 1;
                }
            }
            Action::Enable { entry, enabled } => {
                if let Some(v) = s
                    .records
                    .iter_mut()
                    .find(|v| v["id"] == entry["id"] && v["revision"] == entry["revision"])
                {
                    v["enabled"] = json!(enabled);
                    v["revision"] = json!(v["revision"].as_u64().unwrap() + 1);
                    s.store_revision += 1
                } else {
                    problem = Some(Problem::Conflict)
                }
            }
            Action::Delete(v) => {
                s.records.retain(|p| p["id"] != v["id"]);
                s.total = s.records.len();
                s.store_revision += 1;
            }
            Action::DeleteAll(revision) => {
                if *revision == s.store_revision {
                    s.records.clear();
                    s.total = 0;
                    s.store_revision += 1
                } else {
                    problem = Some(Problem::Conflict)
                }
            }
            Action::Collect { filter, action } => {
                let mut b = Bulk::new(filter.clone(), s.store_revision, s.records.len()).unwrap();
                b.append(s.store_revision, s.records.len(), s.records.clone(), None)
                    .unwrap();
                if *action == BulkAction::Delete {
                    s.bulk = Some(b)
                } else {
                    problem = self.apply_bulk(&mut s, &b, *action).err();
                }
            }
            Action::Apply { bulk, action } => {
                problem = self.apply_bulk(&mut s, bulk, *action).err();
            }
            Action::InspectImport => {
                *self.import_baseline.borrow_mut() = Some(s.store_revision);
                s.import_preview = Some(
                    json!({"token":"synthetic-import","new":2,"duplicates":1,"conflicts":usize::from(self.scenario == Scenario::ConflictingImport)}),
                );
                if self.scenario == Scenario::StaleImport {
                    // Owner changes after inspection, before the user confirms.
                    s.store_revision += 1;
                }
            }
            Action::Import { preview } => {
                if preview["token"] != "synthetic-import"
                    || preview["conflicts"] != 0
                    || *self.import_baseline.borrow() != Some(s.store_revision)
                {
                    problem = Some(Problem::Conflict);
                } else {
                    s.notice = Some("Knowledge imported.".into());
                    *self.import_baseline.borrow_mut() = None;
                }
            }
            Action::Retry => s.availability = "ready".into(),
            Action::Export => s.notice = Some("Knowledge exported.".into()),
            _ => (),
        };
        s.revision += 1;
        self.actions.borrow_mut().push(r.action);
        section::reply(&i, r.ticket, problem.map_or_else(|| Ok(s.clone()), Err));
    }
}
pub fn host() -> Rc<Host> {
    Rc::new(FixtureHost::default())
}
impl super::Knowledge {
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
                .availability = "unavailable".into();
        }
        if state == "empty" {
            let s = self.controller.model.snapshot.as_mut().unwrap();
            s.records.clear();
            s.total = 0;
        }
        if state == "edit" {
            self.begin_dialog(super::Dialog::Edit(Some(entry(0))), w, cx);
        }
        if state == "delete" {
            self.begin_dialog(super::Dialog::DeleteAll(1), w, cx);
        }
    }
}
