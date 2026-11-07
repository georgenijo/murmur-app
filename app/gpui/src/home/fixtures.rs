//! Synthetic projection; real data and native recording services are never accessed.
use super::{host::*, inbox::Inbox, model::*};
use crate::onboarding::model::Access;
use serde_json::json;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
pub fn snapshot() -> Snapshot {
    Snapshot {
        revision: 1,
        settings: murmur_core::state::settings(murmur_core::state::settings_defaults()).unwrap(),
        discovery: json!({"version":1,"checklistDismissed":false,"completed":[],"baseline":{"transforms":0,"meetings":0,"correctionsTaught":0,"modeBindings":[]},"pendingHints":[],"dismissedHints":[]}),
        status: Status::Idle,
        initialized: true,
        recording_generation: None,
        recording_seconds: 0,
        audio_level: 0.,
        meeting_busy: false,
        words_this_month: 420,
        recordings_this_month: 7,
        permissions: [Access::Granted; 4],
        permissions_checked: true,
        destination: None,
    }
}
pub struct FixtureHost {
    pub state: RefCell<Snapshot>,
    pub actions: RefCell<Vec<Action>>,
    pub calls: RefCell<Vec<&'static str>>,
    pub stops: Rc<Cell<usize>>,
    pub fail: Cell<bool>,
}
impl Default for FixtureHost {
    fn default() -> Self {
        Self {
            state: RefCell::new(snapshot()),
            actions: RefCell::new(vec![]),
            calls: RefCell::new(vec![]),
            stops: Default::default(),
            fail: Cell::new(false),
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
    fn request(&self, r: Request, inbox: Inbox) {
        if !r.lease.is_current() || inbox.is_closed() {
            return;
        }
        self.calls.borrow_mut().push("request");
        self.actions.borrow_mut().push(r.action.clone());
        let result = (|| {
            if self.fail.replace(false) {
                return Err(Problem::Unavailable);
            }
            let mut s = self.state.borrow().clone();
            if !matches!(r.action, Action::Read) && r.ticket.revision != s.revision {
                return Err(Problem::Conflict);
            }
            match r.action {
                Action::Start if !s.waiting() && !s.capturing() => {
                    s.status = Status::Recording;
                    s.recording_generation = Some(s.revision + 1);
                }
                Action::Stop { generation }
                    if s.capturing() && s.recording_generation == Some(generation) =>
                {
                    s.status = Status::Processing;
                }
                Action::Start | Action::Stop { .. } => return Err(Problem::Conflict),
                Action::Hint { id, .. } => {
                    let pending = s.discovery["pendingHints"].as_array_mut().unwrap();
                    pending.retain(|v| v != id);
                    if !s.discovery["dismissedHints"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|v| v == id)
                    {
                        s.discovery["dismissedHints"]
                            .as_array_mut()
                            .unwrap()
                            .push(json!(id));
                    }
                }
                Action::DismissChecklist => s.discovery["checklistDismissed"] = json!(true),
                Action::Checklist(id)
                    if matches!(id, "command_palette" | "shortcuts") && !s.completed(id) =>
                {
                    s.discovery["completed"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!(id));
                }
                _ => (),
            }
            s.revision += 1;
            *self.state.borrow_mut() = s.clone();
            Ok(s)
        })();
        let _ = inbox.publish(Event::Reply {
            ticket: r.ticket,
            result,
        });
    }
}
