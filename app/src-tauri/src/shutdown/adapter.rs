//! Host routing seam shared by the production deadline driver and synthetic
//! adapter tests. Owners supply receipts; no owner wait occurs under this lock.
use murmur_core::shutdown::{Coordinator, Effect, Event, Request, State};
use std::sync::{mpsc::RecvTimeoutError, Mutex};
pub(crate) enum Message {
    Accepted(Vec<Effect>),
    Finished(Event),
}
pub(crate) fn reduce(
    core: &Mutex<Coordinator>,
    message: Result<Message, RecvTimeoutError>,
    now: u64,
) -> Option<Vec<Effect>> {
    reduce_observed(core, message, now, |_, _, _| {})
}
/// The optional host observer receives only policy state, under the reducer
/// mutex. It must not perform I/O or wait for an owner.
pub(crate) fn reduce_observed(
    core: &Mutex<Coordinator>,
    message: Result<Message, RecvTimeoutError>,
    now: u64,
    observe: impl FnOnce(State, &Coordinator, Option<Event>),
) -> Option<Vec<Effect>> {
    let mut core = core.lock().unwrap_or_else(|p| p.into_inner());
    let before = core.state;
    let event = match &message {
        Ok(Message::Finished(event)) => Some(*event),
        _ => None,
    };
    let result = match message {
        Ok(Message::Accepted(effects)) => Some(effects),
        Ok(Message::Finished(event)) => Some(core.advance(event, now)),
        Err(RecvTimeoutError::Timeout) => Some(core.tick(now)),
        Err(RecvTimeoutError::Disconnected) => None,
    };
    observe(before, &core, event);
    result
}
pub(crate) fn intercept_exit(permitted: bool, prevent: impl FnOnce(), request: impl FnOnce()) {
    if !permitted {
        prevent();
        request();
    }
}

pub(crate) fn accept(
    core: &Mutex<Coordinator>,
    send: &std::sync::mpsc::Sender<Message>,
    request: Request,
    now: u64,
) {
    accept_observed(core, send, request, now, |_, _| {});
}
pub(crate) fn accept_observed(
    core: &Mutex<Coordinator>,
    send: &std::sync::mpsc::Sender<Message>,
    request: Request,
    now: u64,
    observe: impl FnOnce(State, &Coordinator),
) {
    let mut core = core.lock().unwrap_or_else(|p| p.into_inner());
    let before = core.state;
    let effects = core.request(request, now);
    observe(before, &core);
    let _ = send.send(Message::Accepted(effects));
}
