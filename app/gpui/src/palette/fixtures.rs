use super::{host::*, model::*};
use crate::section;
use std::{cell::RefCell, rc::Rc};
pub fn snapshot() -> Snapshot {
    let state = super::registry::State {
        has_history: true,
        enabled_modes: crate::modes::model::builtins()
            .iter()
            .map(|m| {
                (
                    m["id"].as_str().unwrap().into(),
                    m["name"].as_str().unwrap().into(),
                )
            })
            .collect(),
        ..Default::default()
    };
    Snapshot {
        revision: 1,
        commands: super::registry::commands(&state),
    }
}
#[derive(Default)]
pub struct FixtureHost {
    pub actions: RefCell<Vec<Action>>,
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
    fn request(&self, r: Request, inbox: crate::workspace::Inbox<Snapshot>) {
        if !r.lease.is_current() {
            return;
        }
        self.actions.borrow_mut().push(r.action);
        section::reply(&inbox, r.ticket, Ok(snapshot()));
    }
}
pub fn host() -> Rc<Host> {
    Rc::new(FixtureHost::default())
}
impl super::CommandPalette {
    pub fn fixture_state(
        &mut self,
        state: &str,
        w: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.show(w, cx);
        if state == "search" {
            self.search
                .update(cx, |s, cx| s.set_value("delivery", w, cx));
        }
        if state == "empty" {
            self.search
                .update(cx, |s, cx| s.set_value("no matching command", w, cx));
        }
        if state == "loading" {
            self.controller.model.snapshot = None;
        }
        if state == "error" {
            self.controller.model.problem = Some(Problem::Unavailable);
        }
    }
    pub fn fixture_type(&mut self, w: &mut gpui::Window, cx: &mut gpui::Context<Self>) {
        use gpui::{EntityInputHandler, Focusable};
        self.show(w, cx);
        let e = self.search.clone();
        w.focus(&e.read(cx).focus_handle(cx), cx);
        e.update(cx, |s, cx| s.replace_text_in_range(None, "delivery", w, cx));
    }
}
