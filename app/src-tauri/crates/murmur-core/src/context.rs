//! Explicit host capabilities. Core services own typed state; there is no
//! AppHandle-style service locator and no runtime creation here.
use std::{future::Future, path::PathBuf, pin::Pin, sync::Arc};

pub type Task = Box<dyn FnOnce() + Send + 'static>;
pub type BackgroundFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextError {
    PathUnavailable,
    SchedulingFailed,
    WorkerStopped,
}

impl std::fmt::Display for ContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::PathUnavailable => "Host path unavailable",
            Self::SchedulingFailed => "Host scheduling failed",
            Self::WorkerStopped => "Host worker stopped before returning a result",
        })
    }
}
impl std::error::Error for ContextError {}

pub trait Paths: Send + Sync {
    /// Durable app data. Distinct from the existing shared model directory.
    fn app_data_dir(&self) -> Result<PathBuf, ContextError>;
    fn models_dir(&self) -> Option<PathBuf>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionStatus {
    Granted,
    Denied,
    NotDetermined,
    Unknown,
}

pub trait Permissions: Send + Sync {
    /// Query only: never request permission or open a capture device.
    fn microphone_status(&self) -> PermissionStatus;
}

pub trait BackgroundWork: Send + Sync {
    /// Scheduling success does not mean completion. Services must own their
    /// cancellation/generation checks and drain pending work before host exit.
    fn spawn(&self, future: BackgroundFuture) -> Result<(), ContextError>;
    fn spawn_blocking(&self, task: Task) -> Result<(), ContextError>;
}

pub trait MainThread: Send + Sync {
    /// Enqueue without blocking the caller. NSWindow mutations belong here.
    fn dispatch(&self, task: Task) -> Result<(), ContextError>;
}

#[derive(Clone)]
pub struct AppContext {
    pub paths: Arc<dyn Paths>,
    pub permissions: Arc<dyn Permissions>,
    pub background: Arc<dyn BackgroundWork>,
    pub main_thread: Arc<dyn MainThread>,
}

impl AppContext {
    /// Await a result from the host's blocking pool without owning its runtime.
    /// A panic or shutdown that drops the worker closes the result channel.
    pub async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, ContextError> {
        let (send, receive) = futures_channel::oneshot::channel();
        self.background.spawn_blocking(Box::new(move || {
            let _ = send.send(work());
        }))?;
        receive.await.map_err(|_| ContextError::WorkerStopped)
    }
}
