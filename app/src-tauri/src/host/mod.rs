//! Typed host boundary for shared service owners. The native context has concrete
//! slots, never an AppHandle-style erased service registry. Tauri keeps its
//! managed-state adapter and command ABI until Phase 4.
#[cfg(feature = "tauri-host")]
pub mod runtime {
    pub fn spawn<F: std::future::Future + Send + 'static>(
        future: F,
    ) -> tauri::async_runtime::JoinHandle<F::Output>
    where
        F::Output: Send + 'static,
    {
        tauri::async_runtime::spawn(future)
    }
    pub fn spawn_blocking<F: FnOnce() -> T + Send + 'static, T: Send + 'static>(
        work: F,
    ) -> tauri::async_runtime::JoinHandle<T> {
        tauri::async_runtime::spawn_blocking(work)
    }
    pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tauri::async_runtime::block_on(future)
    }
}
#[cfg(feature = "tauri-host")]
pub trait Runtime: tauri::Runtime {}
#[cfg(feature = "tauri-host")]
impl<R: tauri::Runtime> Runtime for R {}
#[cfg(feature = "tauri-host")]
pub type Context<R = DefaultRuntime> = tauri::AppHandle<R>;
#[cfg(feature = "tauri-host")]
pub type StateRef<'a, T> = tauri::State<'a, T>;
#[cfg(feature = "tauri-host")]
pub type Window<R = DefaultRuntime> = tauri::WebviewWindow<R>;
#[cfg(feature = "tauri-host")]
pub type DefaultRuntime = tauri::Wry;

pub(crate) trait ServiceAccess<R: Runtime> {
    fn services(&self) -> StateRef<'_, crate::State>;
    fn try_services(&self) -> Option<StateRef<'_, crate::State>>;
    fn context(&self) -> StateRef<'_, murmur_core::context::AppContext>;
    #[cfg(feature = "tauri-host")]
    fn try_context(&self) -> Option<StateRef<'_, murmur_core::context::AppContext>>;
    fn control(&self) -> StateRef<'_, crate::dictation_control::Control>;
    fn try_control(&self) -> Option<StateRef<'_, crate::dictation_control::Control>>;
    fn event_owner(&self) -> StateRef<'_, crate::events::Events>;
    fn try_event_owner(&self) -> Option<StateRef<'_, crate::events::Events>>;
    fn completion_owner(&self) -> StateRef<'_, crate::completion::Completion>;
    fn try_completion_owner(&self) -> Option<StateRef<'_, crate::completion::Completion>>;
    fn completion_initialization(
        &self,
    ) -> StateRef<'_, crate::completion::CompletionInitialization>;
    fn try_completion_initialization(
        &self,
    ) -> Option<StateRef<'_, crate::completion::CompletionInitialization>>;
    fn window_state_owner(&self) -> StateRef<'_, crate::window_state::WindowState>;
    fn try_window_state_owner(&self) -> Option<StateRef<'_, crate::window_state::WindowState>>;
    fn try_shutdown_owner(&self) -> Option<StateRef<'_, crate::shutdown::Shutdown>>;
    fn try_updater_owner(
        &self,
    ) -> Option<StateRef<'_, std::sync::Arc<murmur_core::updater::Updater>>>;
    fn install_control(&self, owner: crate::dictation_control::Control) -> bool;
    fn install_event_owner(&self, owner: crate::events::Events) -> bool;
    fn install_completion_owner(&self, owner: crate::completion::Completion) -> bool;
    fn install_completion_initialization(
        &self,
        owner: crate::completion::CompletionInitialization,
    ) -> bool;
    fn install_window_state_owner(&self, owner: crate::window_state::WindowState) -> bool;
    fn install_shutdown_owner(&self, owner: crate::shutdown::Shutdown) -> bool;
    fn install_updater_owner(&self, owner: std::sync::Arc<murmur_core::updater::Updater>) -> bool;
}
#[cfg(feature = "tauri-host")]
impl<R: Runtime, T: tauri::Manager<R>> ServiceAccess<R> for T {
    fn services(&self) -> StateRef<'_, crate::State> {
        tauri::Manager::state::<crate::State>(self)
    }
    fn try_services(&self) -> Option<StateRef<'_, crate::State>> {
        tauri::Manager::try_state::<crate::State>(self)
    }
    fn context(&self) -> StateRef<'_, murmur_core::context::AppContext> {
        tauri::Manager::state::<murmur_core::context::AppContext>(self)
    }
    #[cfg(feature = "tauri-host")]
    fn try_context(&self) -> Option<StateRef<'_, murmur_core::context::AppContext>> {
        tauri::Manager::try_state::<murmur_core::context::AppContext>(self)
    }
    fn control(&self) -> StateRef<'_, crate::dictation_control::Control> {
        tauri::Manager::state::<crate::dictation_control::Control>(self)
    }
    fn try_control(&self) -> Option<StateRef<'_, crate::dictation_control::Control>> {
        tauri::Manager::try_state::<crate::dictation_control::Control>(self)
    }
    fn event_owner(&self) -> StateRef<'_, crate::events::Events> {
        tauri::Manager::state::<crate::events::Events>(self)
    }
    fn try_event_owner(&self) -> Option<StateRef<'_, crate::events::Events>> {
        tauri::Manager::try_state::<crate::events::Events>(self)
    }
    fn completion_owner(&self) -> StateRef<'_, crate::completion::Completion> {
        tauri::Manager::state::<crate::completion::Completion>(self)
    }
    fn try_completion_owner(&self) -> Option<StateRef<'_, crate::completion::Completion>> {
        tauri::Manager::try_state::<crate::completion::Completion>(self)
    }
    fn completion_initialization(
        &self,
    ) -> StateRef<'_, crate::completion::CompletionInitialization> {
        tauri::Manager::state::<crate::completion::CompletionInitialization>(self)
    }
    fn try_completion_initialization(
        &self,
    ) -> Option<StateRef<'_, crate::completion::CompletionInitialization>> {
        tauri::Manager::try_state::<crate::completion::CompletionInitialization>(self)
    }
    fn window_state_owner(&self) -> StateRef<'_, crate::window_state::WindowState> {
        tauri::Manager::state::<crate::window_state::WindowState>(self)
    }
    fn try_window_state_owner(&self) -> Option<StateRef<'_, crate::window_state::WindowState>> {
        tauri::Manager::try_state::<crate::window_state::WindowState>(self)
    }
    fn try_shutdown_owner(&self) -> Option<StateRef<'_, crate::shutdown::Shutdown>> {
        tauri::Manager::try_state::<crate::shutdown::Shutdown>(self)
    }
    fn try_updater_owner(
        &self,
    ) -> Option<StateRef<'_, std::sync::Arc<murmur_core::updater::Updater>>> {
        tauri::Manager::try_state::<std::sync::Arc<murmur_core::updater::Updater>>(self)
    }
    fn install_control(&self, owner: crate::dictation_control::Control) -> bool {
        tauri::Manager::manage(self, owner)
    }
    fn install_event_owner(&self, owner: crate::events::Events) -> bool {
        tauri::Manager::manage(self, owner)
    }
    fn install_completion_owner(&self, owner: crate::completion::Completion) -> bool {
        tauri::Manager::manage(self, owner)
    }
    fn install_completion_initialization(
        &self,
        owner: crate::completion::CompletionInitialization,
    ) -> bool {
        tauri::Manager::manage(self, owner)
    }
    fn install_window_state_owner(&self, owner: crate::window_state::WindowState) -> bool {
        tauri::Manager::manage(self, owner)
    }
    fn install_shutdown_owner(&self, owner: crate::shutdown::Shutdown) -> bool {
        tauri::Manager::manage(self, owner)
    }
    fn install_updater_owner(&self, owner: std::sync::Arc<murmur_core::updater::Updater>) -> bool {
        tauri::Manager::manage(self, owner)
    }
}

#[cfg(not(feature = "tauri-host"))]
mod native;
#[cfg(not(feature = "tauri-host"))]
pub use native::*;

#[cfg(not(feature = "tauri-host"))]
pub use dpi::{LogicalPosition, LogicalSize};
#[cfg(feature = "tauri-host")]
pub mod window {
    pub type Color = tauri::window::Color;
}
#[cfg(feature = "tauri-host")]
pub type LogicalPosition<T> = tauri::LogicalPosition<T>;
#[cfg(feature = "tauri-host")]
pub type LogicalSize<T> = tauri::LogicalSize<T>;
#[cfg(feature = "tauri-host")]
pub type Monitor = tauri::Monitor;
#[cfg(feature = "tauri-host")]
pub type Result<T> = tauri::Result<T>;
#[cfg(feature = "tauri-host")]
pub type HostWindow<R = DefaultRuntime> = tauri::Window<R>;
#[cfg(feature = "tauri-host")]
pub type WindowEvent = tauri::WindowEvent;

pub(crate) fn send<R: Runtime, P: serde::Serialize>(
    app: &Context<R>,
    label: &str,
    event: &str,
    payload: &P,
) -> Result<()> {
    #[cfg(feature = "tauri-host")]
    {
        crate::events::transport_send(app, label, event, payload)
    }
    #[cfg(not(feature = "tauri-host"))]
    {
        let encoded = serde_json::to_string(payload).map_err(|_| "Event serialization failed")?;
        app.send(label, event, &encoded)
    }
}
pub(crate) fn send_str<R: Runtime>(
    app: &Context<R>,
    label: &str,
    event: &str,
    payload: &str,
) -> Result<()> {
    #[cfg(feature = "tauri-host")]
    {
        crate::events::transport_send_str(app, label, event, payload)
    }
    #[cfg(not(feature = "tauri-host"))]
    {
        app.send(label, event, payload)
    }
}
#[cfg(feature = "tauri-host")]
pub type BinaryResponse = tauri::ipc::Response;
