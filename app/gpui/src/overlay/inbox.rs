//! Bounded lifecycle queue plus one replaceable level sample. Publishers never
//! touch GPUI/NSWindow, wait for main, or serialize payloads into diagnostic text.
use super::model::Event;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
#[derive(Default, Clone)]
pub struct Inbox(Arc<Mutex<Pending>>);
#[derive(Default)]
struct Pending {
    events: VecDeque<Event>,
    level: Option<(u64, f32)>,
    waker: Option<std::task::Waker>,
}
impl Inbox {
    /// Backpressure is explicit. A full lifecycle queue must be retried or
    /// followed by a fresh authoritative snapshot, never silently dropped.
    pub fn publish(&self, event: Event) -> Result<(), Box<Event>> {
        let mut pending = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if pending.events.len() >= 256 {
            return Err(Box::new(event));
        }
        // Periodic host snapshots must not manufacture idle frame demand.
        if let Event::Dock(limit) = &event
            && matches!(pending.events.back(), Some(Event::Dock(previous)) if previous == limit)
        {
            return Ok(());
        }
        pending.events.push_back(event);
        let waker = pending.waker.take();
        drop(pending);
        if let Some(waker) = waker {
            waker.wake();
        }
        Ok(())
    }
    pub fn level(&self, generation: u64, level: f32) {
        let mut pending = self.0.lock().unwrap_or_else(|p| p.into_inner());
        pending.level = Some((generation, level));
        let waker = pending.waker.take();
        drop(pending);
        if let Some(waker) = waker {
            waker.wake();
        }
    }
    /// Suspends with no timer or frame source. Register under the publisher's
    /// lock so an event racing the drain cannot be lost.
    pub async fn ready(&self) {
        std::future::poll_fn(|cx| {
            let mut pending = self.0.lock().unwrap_or_else(|p| p.into_inner());
            if !pending.events.is_empty() || pending.level.is_some() {
                std::task::Poll::Ready(())
            } else {
                pending.waker = Some(cx.waker().clone());
                std::task::Poll::Pending
            }
        })
        .await
    }
    pub(crate) fn drain(&self) -> (VecDeque<Event>, Option<(u64, f32)>) {
        let mut p = self.0.lock().unwrap_or_else(|p| p.into_inner());
        (std::mem::take(&mut p.events), p.level.take())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audio_coalesces_without_displacing_lifecycle_and_backpressure_is_explicit() {
        let inbox = Inbox::default();
        for _ in 0..256 {
            assert!(inbox.publish(Event::Visible(true)).is_ok());
        }
        assert!(inbox.publish(Event::Visible(false)).is_err());
        for i in 0..1000 {
            inbox.level(7, i as f32);
        }
        let (events, level) = inbox.drain();
        assert_eq!(events.len(), 256);
        assert_eq!(level, Some((7, 999.)));
        assert!(inbox.drain().0.is_empty());
        assert!(inbox.drain().1.is_none());
    }
}

#[cfg(test)]
mod wake_tests {
    use super::*;
    use futures::task::{ArcWake, waker};
    use std::{
        future::Future,
        sync::atomic::{AtomicUsize, Ordering},
        task::{Context, Poll},
    };

    struct WakeCount(AtomicUsize);
    impl ArcWake for WakeCount {
        fn wake_by_ref(this: &Arc<Self>) {
            this.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    #[test]
    fn parked_consumer_wakes_once_for_coalesced_levels_and_never_loses_lifecycle() {
        let inbox = Inbox::default();
        let count = Arc::new(WakeCount(AtomicUsize::new(0)));
        let waker = waker(count.clone());
        let mut cx = Context::from_waker(&waker);
        let mut ready = Box::pin(inbox.ready());
        assert_eq!(ready.as_mut().poll(&mut cx), Poll::Pending);
        assert_eq!(count.0.load(Ordering::Relaxed), 0);
        inbox.level(1, 0.1);
        inbox.level(1, 0.8);
        inbox
            .publish(Event::Visible(false))
            .unwrap_or_else(|_| panic!("queue"));
        assert_eq!(count.0.load(Ordering::Relaxed), 1);
        assert_eq!(ready.as_mut().poll(&mut cx), Poll::Ready(()));
        drop(ready);
        let (events, level) = inbox.drain();
        assert!(matches!(events.front(), Some(Event::Visible(false))));
        assert_eq!(level, Some((1, 0.8)));
        assert_eq!(
            Box::pin(inbox.ready()).as_mut().poll(&mut cx),
            Poll::Pending
        );
    }
}
