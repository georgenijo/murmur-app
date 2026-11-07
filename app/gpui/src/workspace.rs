//! Bounded, revocable presentation transport shared by the main-window ports.
//! Service admission and durable ownership stay with the injected host.
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, Weak},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub attachment: u64,
    pub generation: u64,
    pub revision: u64,
}
pub trait Projection: Clone {
    fn revision(&self) -> u64;
    fn valid(&self) -> bool;
    fn bytes(&self) -> usize;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    Unavailable,
    Conflict,
    Invalid,
}
impl Problem {
    pub fn message(self) -> &'static str {
        match self {
            Self::Unavailable => {
                "Something went wrong. Your choices are kept. Retry when the service is available."
            }
            Self::Conflict => "State changed elsewhere. Review the current state and try again.",
            Self::Invalid => {
                "Could not read the current state. Retry when the service is available."
            }
        }
    }
}
pub enum Event<S> {
    Changed {
        attachment: u64,
        snapshot: S,
    },
    /// Progress keeps the original ticket/revision; the host correlates its
    /// native request ID and attempt before forwarding it to this inbox.
    Progress {
        ticket: Ticket,
        snapshot: S,
    },
    Reply {
        ticket: Ticket,
        result: Result<S, Problem>,
    },
}
pub struct Inbox<S>(Arc<Mutex<Pending<S>>>);
impl<S> Clone for Inbox<S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
struct Pending<S> {
    queue: VecDeque<(Event<S>, usize)>,
    bytes: usize,
    closed: bool,
}
impl<S> Default for Inbox<S> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Pending {
            queue: VecDeque::new(),
            bytes: 0,
            closed: false,
        })))
    }
}
impl<S: Projection> Inbox<S> {
    pub fn publish(&self, event: Event<S>) -> Result<(), Box<Event<S>>> {
        let size = match &event {
            Event::Changed { snapshot, .. }
            | Event::Progress { snapshot, .. }
            | Event::Reply {
                result: Ok(snapshot),
                ..
            } => {
                if !snapshot.valid() {
                    return Err(Box::new(event));
                }
                snapshot.bytes()
            }
            _ => 128,
        };
        let mut p = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if p.closed || p.queue.len() >= 8 || p.bytes.saturating_add(size) > 4 * 1024 * 1024 {
            return Err(Box::new(event));
        }
        p.bytes += size;
        p.queue.push_back((event, size));
        Ok(())
    }
    pub fn is_closed(&self) -> bool {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).closed
    }
    pub(crate) fn close(&self) {
        let mut p = self.0.lock().unwrap_or_else(|p| p.into_inner());
        p.closed = true;
        p.queue.clear();
        p.bytes = 0;
    }
    pub(crate) fn drain(&self) -> VecDeque<Event<S>> {
        let mut p = self.0.lock().unwrap_or_else(|p| p.into_inner());
        p.bytes = 0;
        std::mem::take(&mut p.queue)
            .into_iter()
            .map(|(e, _)| e)
            .collect()
    }
}
pub struct Session<S> {
    pub snapshot: Option<S>,
    pub problem: Option<Problem>,
    pub pending: bool,
    pub attached: bool,
    pub attachment: u64,
    pub generation: u64,
    pub revision: u64,
}
impl<S> Default for Session<S> {
    fn default() -> Self {
        Self {
            snapshot: None,
            problem: None,
            pending: false,
            attached: false,
            attachment: 0,
            generation: 0,
            revision: 0,
        }
    }
}
impl<S: Projection> Session<S> {
    pub fn ticket(&self) -> Ticket {
        Ticket {
            attachment: self.attachment,
            generation: self.generation,
            revision: self.revision,
        }
    }
    pub fn attach(&mut self) {
        self.attachment += 1;
        self.generation += 1;
        self.attached = true;
        self.revision = 0;
    }
    pub fn detach(&mut self) {
        self.attached = false;
        self.generation += 1;
        self.snapshot = None;
        self.pending = false;
        self.problem = None;
    }
    pub fn invalidate(&mut self) {
        self.generation += 1;
        self.pending = false;
        self.problem = None;
    }
    pub fn begin(&mut self) -> Option<Ticket> {
        if !self.attached || self.pending {
            return None;
        }
        self.invalidate();
        self.pending = true;
        Some(self.ticket())
    }
    pub fn event(&mut self, e: Event<S>) -> bool {
        if !self.attached {
            return false;
        }
        let (snapshot, terminal, progress) = match e {
            Event::Changed {
                attachment,
                snapshot,
            } => {
                if attachment != self.attachment || snapshot.revision() <= self.revision {
                    return false;
                }
                (snapshot, false, false)
            }
            Event::Progress { ticket, snapshot } => {
                if !self.pending || ticket != self.ticket() || snapshot.revision() != self.revision
                {
                    return false;
                }
                (snapshot, false, true)
            }
            Event::Reply { ticket, result } => {
                if !self.pending || ticket != self.ticket() {
                    return false;
                }
                match result {
                    Ok(s) => (s, true, false),
                    Err(p) => {
                        self.pending = false;
                        self.problem = Some(p);
                        return true;
                    }
                }
            }
        };
        if snapshot.revision() < self.revision {
            return false;
        }
        if !snapshot.valid() {
            self.problem = Some(Problem::Invalid);
            if terminal {
                self.pending = false;
            }
            return true;
        }
        self.revision = snapshot.revision();
        self.snapshot = Some(snapshot);
        self.problem = None;
        if terminal {
            self.pending = false;
        } else if !progress && self.pending {
            self.invalidate();
            self.problem = Some(Problem::Conflict);
        }
        true
    }
}
#[derive(Default)]
pub(crate) struct Authority(Arc<Mutex<Option<Ticket>>>);
pub struct Lease {
    authority: Weak<Mutex<Option<Ticket>>>,
    ticket: Ticket,
}
impl Lease {
    pub fn is_current(&self) -> bool {
        self.authority
            .upgrade()
            .is_some_and(|a| *a.lock().unwrap_or_else(|p| p.into_inner()) == Some(self.ticket))
    }
}
impl Authority {
    pub fn sync<S: Projection>(&self, s: &Session<S>) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = s.attached.then_some(s.ticket());
    }
    pub fn revoke(&self) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
    pub fn lease(&self, ticket: Ticket) -> Lease {
        Lease {
            authority: Arc::downgrade(&self.0),
            ticket,
        }
    }
}
