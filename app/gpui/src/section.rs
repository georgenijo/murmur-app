//! Shared main-section lifecycle. No service ownership, storage or OS effects here.
use crate::workspace::{Authority, Event, Inbox, Projection, Session, Ticket};
pub use crate::workspace::{Lease, Problem};
use gpui::{Context, Render};
use std::{rc::Rc, time::Duration};
pub trait Subscription {
    fn stop(&mut self);
}
pub struct Request<A> {
    pub ticket: Ticket,
    pub lease: Lease,
    pub action: A,
}
pub trait Host<S: Projection, A> {
    /// Subscribe Main before reading. Subscription must not trigger network,
    /// capture, permissions or downloads. Never log payloads.
    fn subscribe(
        &self,
        consumer: murmur_core::events::Consumer,
        attachment: u64,
        inbox: Inbox<S>,
    ) -> Box<dyn Subscription>;
    /// Reads/panels/network are revocable. Recheck lease AND service ownership
    /// after each await and immediately before an effect. Durable preference
    /// intents admit synchronously, then remain owned by the existing service;
    /// merge only the requested keys under its write lock. Never rewrite stale
    /// documents. Replies/progress carry the original ticket.
    fn request(&self, request: Request<A>, inbox: Inbox<S>);
}
pub struct Controller<S: Projection, A> {
    pub model: Session<S>,
    pub inbox: Inbox<S>,
    host: Rc<dyn Host<S, A>>,
    authority: Authority,
    subscription: Option<Box<dyn Subscription>>,
}
impl<S: Projection, A> Controller<S, A> {
    pub fn new(host: Rc<dyn Host<S, A>>) -> Self {
        Self {
            model: Session::default(),
            inbox: Inbox::default(),
            host,
            authority: Authority::default(),
            subscription: None,
        }
    }
    pub fn attach(&mut self, read: A) {
        if self.model.attached {
            return;
        }
        self.model.attach();
        self.inbox = Inbox::default();
        self.authority.sync(&self.model);
        self.subscription = Some(self.host.subscribe(
            murmur_core::events::Consumer::Main,
            self.model.attachment,
            self.inbox.clone(),
        ));
        self.request(read);
    }
    pub fn request(&mut self, action: A) -> bool {
        let Some(ticket) = self.model.begin() else {
            return false;
        };
        self.authority.sync(&self.model);
        self.host.request(
            Request {
                ticket,
                lease: self.authority.lease(ticket),
                action,
            },
            self.inbox.clone(),
        );
        true
    }
    pub fn invalidate(&mut self) {
        self.model.invalidate();
        self.authority.sync(&self.model);
    }
    pub fn drain(&mut self) -> bool {
        let mut changed = false;
        for event in self.inbox.drain() {
            changed |= self.model.event(event);
        }
        self.authority.sync(&self.model);
        changed
    }
    pub fn detach(&mut self) {
        self.authority.revoke();
        self.inbox.close();
        if let Some(mut s) = self.subscription.take() {
            s.stop()
        };
        self.model.detach();
    }
}
impl<S: Projection, A> Drop for Controller<S, A> {
    fn drop(&mut self) {
        self.detach();
    }
}
pub fn follow<V: Render + 'static, S: Projection + 'static, A: 'static>(
    cx: &mut Context<V>,
    attachment: u64,
    get: fn(&mut V) -> &mut Controller<S, A>,
) {
    cx.spawn(async move |weak, cx| {
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(32))
                .await;
            if !weak
                .update(cx, |s, cx| {
                    let c = get(s);
                    if !c.model.attached || c.model.attachment != attachment || c.inbox.is_closed()
                    {
                        return false;
                    };
                    if c.drain() {
                        cx.notify()
                    };
                    true
                })
                .unwrap_or(false)
            {
                break;
            }
        }
    })
    .detach();
}
/// Bounded plain projection helpers; content is never formatted into errors.
pub fn bytes(value: &serde_json::Value) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |v| v.len())
}
pub fn text(value: &str, limit: usize) -> bool {
    value.len() <= limit && !value.contains('\0')
}
pub fn reply<S: Projection>(inbox: &Inbox<S>, ticket: Ticket, result: Result<S, Problem>) {
    let _ = inbox.publish(Event::Reply { ticket, result });
}
