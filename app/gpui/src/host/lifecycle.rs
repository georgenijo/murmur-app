//! The native host runs the same #992 reducer before permitting AppKit quit.
//! The deadline driver never waits for an owner or blocks the AppKit thread.
use murmur_core::{context::BackgroundFuture, shutdown::*};
use std::{
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
pub trait DrainOwners: Send + Sync + 'static {
    /// Synchronous, memory-only admission fence, before returning to AppKit.
    fn close_admission(&self);
    /// Owner futures must keep the original recording generation/configuration.
    /// These futures report evidence; cancellation is never evidence of a drain.
    fn drain(
        &self,
        effect: Effect,
        deadline: Instant,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Outcome> + Send>>;
}
enum Message {
    Accepted(Vec<Effect>),
    Finished(Event),
}
#[derive(Clone)]
pub struct Lifecycle {
    core: Arc<Mutex<Coordinator>>,
    send: mpsc::Sender<Message>,
    clock: Instant,
    owners: Arc<dyn DrainOwners>,
}
impl Lifecycle {
    pub fn start(
        owners: Arc<dyn DrainOwners>,
        spawn: impl Fn(BackgroundFuture) + Send + 'static,
    ) -> std::io::Result<(Self, mpsc::Receiver<Action>)> {
        let core = Arc::new(Mutex::new(Coordinator::default()));
        let (send, receive) = mpsc::channel();
        let (terminal, ready) = mpsc::channel();
        let this = Self {
            core: core.clone(),
            send: send.clone(),
            clock: Instant::now(),
            owners: owners.clone(),
        };
        let clock = this.clock;
        std::thread::Builder::new()
            .name("gpui-shutdown-deadlines".into())
            .spawn(move || {
                loop {
                    let message = receive.recv_timeout(Duration::from_millis(10));
                    let effects = {
                        let mut core = core.lock().unwrap_or_else(|p| p.into_inner());
                        let now = clock.elapsed().as_millis() as u64;
                        match message {
                            Ok(Message::Accepted(e)) => e,
                            Ok(Message::Finished(e)) => core.advance(e, now),
                            Err(mpsc::RecvTimeoutError::Timeout) => core.tick(now),
                            Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    };
                    for effect in effects {
                        if effect.step == Step::Terminate {
                            if let Some(action) =
                                core.lock().unwrap_or_else(|p| p.into_inner()).issue()
                            {
                                let _ = terminal.send(action);
                            }
                            return;
                        }
                        let owners = owners.clone();
                        let send = send.clone();
                        spawn(Box::pin(async move {
                            let outcome = owners
                                .drain(effect, clock + Duration::from_millis(effect.deadline_ms))
                                .await;
                            let _ = send.send(Message::Finished(Event { effect, outcome }));
                        }));
                    }
                }
            })?;
        Ok((this, ready))
    }
    pub fn request(&self, request: Request) {
        self.owners.close_admission();
        let mut core = self.core.lock().unwrap_or_else(|p| p.into_inner());
        let effects = core.request(request, self.clock.elapsed().as_millis() as u64);
        let _ = self.send.send(Message::Accepted(effects));
    }
    pub fn permitted(&self) -> bool {
        self.core
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .termination_permitted()
    }
    pub fn restart_ticket(&self) -> Option<RestartTicket> {
        self.core
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .restart_ticket()
    }
    pub fn owns_restart_ticket(&self, ticket: RestartTicket) -> bool {
        self.core
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .owns_restart_ticket(ticket)
    }
    pub fn exited(&self) {
        self.core.lock().unwrap_or_else(|p| p.into_inner()).exited();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Owners {
        fenced: AtomicBool,
        writes: WriteOutcome,
        steps: Mutex<Vec<Step>>,
    }
    impl DrainOwners for Owners {
        fn close_admission(&self) {
            self.fenced.store(true, Ordering::Release);
        }
        fn drain(
            &self,
            effect: Effect,
            _: Instant,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Outcome> + Send>> {
            assert!(self.fenced.load(Ordering::Acquire));
            self.steps.lock().unwrap().push(effect.step);
            let outcome = match effect.step {
                Step::Admission => Outcome::Admission(true),
                Step::Capture => Outcome::Capture(true),
                Step::Helpers => Outcome::Helpers(true),
                Step::Writers => Outcome::Writers(self.writes),
                Step::Terminate => panic!("termination is not an owner effect"),
            };
            Box::pin(async move { outcome })
        }
    }
    #[test]
    fn owners_drain_before_terminal_permission_and_failed_writes_suppress_restart() {
        for writes in [WriteOutcome::Committed, WriteOutcome::UnpreservedFailure] {
            let owners = Arc::new(Owners {
                fenced: AtomicBool::new(false),
                writes,
                steps: Mutex::default(),
            });
            let (lifecycle, terminal) = Lifecycle::start(owners.clone(), |future| {
                std::thread::spawn(move || futures::executor::block_on(future));
            })
            .unwrap();
            assert!(!lifecycle.permitted());
            lifecycle.request(Request::Restart);
            assert!(owners.fenced.load(Ordering::Acquire));
            let action = terminal.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(
                action,
                if writes == WriteOutcome::Committed {
                    Action::Restart
                } else {
                    Action::Quit
                }
            );
            assert!(lifecycle.permitted());
            assert_eq!(
                *owners.steps.lock().unwrap(),
                [Step::Admission, Step::Capture, Step::Helpers, Step::Writers]
            );
            assert!(terminal.try_recv().is_err());
        }
    }
}
