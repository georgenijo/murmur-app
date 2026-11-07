//! Typed host boundary for shared service owners. The native context has concrete
//! slots, never an AppHandle-style erased service registry. The GPUI host supplies the presentation and OS ports.

pub(crate) trait ServiceAccess<R: Runtime> {
    fn services(&self) -> StateRef<'_, crate::State>;
    fn try_services(&self) -> Option<StateRef<'_, crate::State>>;
    fn context(&self) -> StateRef<'_, murmur_core::context::AppContext>;

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

mod native;

pub use native::*;

pub use dpi::{LogicalPosition, LogicalSize};

pub(crate) fn send<R: Runtime, P: serde::Serialize>(
    app: &Context<R>,
    label: &str,
    event: &str,
    payload: &P,
) -> Result<()> {
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
    {
        app.send(label, event, payload)
    }
}
