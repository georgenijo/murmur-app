//! Host routing seam shared by the production deadline driver and synthetic
//! adapter tests. Owners supply receipts; no owner wait occurs under this lock.
use murmur_core::shutdown::{Coordinator, Effect, Event, Request};
use std::sync::{mpsc::RecvTimeoutError, Mutex};
pub(crate) enum Message {
    Request(Request, u64),
    Finished(Event),
}
pub(crate) fn reduce(
    core: &Mutex<Coordinator>,
    message: Result<Message, RecvTimeoutError>,
    now: u64,
) -> Option<Vec<Effect>> {
    let mut core = core.lock().unwrap_or_else(|p| p.into_inner());
    match message {
        Ok(Message::Request(request, requested_at)) => {
            let mut effects = core.request(request, requested_at);
            effects.extend(core.tick(now));
            Some(effects)
        }
        Ok(Message::Finished(event)) => Some(core.advance(event, now)),
        Err(RecvTimeoutError::Timeout) => Some(core.tick(now)),
        Err(RecvTimeoutError::Disconnected) => None,
    }
}
pub(crate) fn intercept_exit(permitted: bool, prevent: impl FnOnce(), request: impl FnOnce()) {
    if !permitted {
        prevent();
        request();
    }
}
