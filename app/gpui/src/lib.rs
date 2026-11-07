//! Murmur's shared native presentation layer. Core never depends on this crate.
pub mod button;
pub mod choice;
pub mod data_components;
pub mod diagnostics;
mod events;
pub mod field;
pub mod gallery;
pub mod history;
pub mod home;
pub mod meetings;
pub mod motion;
pub mod onboarding;
pub mod overlay;
pub mod preferences;
pub mod preview;
pub mod settings;
pub mod toggle;
pub mod tokens;
mod workspace;
mod section;
pub mod palette;

pub use button::{Button, ButtonIntent, ButtonSize};
pub use choice::{Choice, ChoiceKind, Choices, SelectionChanged};
pub use field::TextField;
pub use toggle::Toggle;

/// Call once from GPUI's application callback, on the main thread. Installs
/// gpui-component's keyboard/editor infrastructure and the Phase 0 AX fix.
pub fn init(cx: &mut gpui::App) {
    init_with_preferences(cx, None);
}

/// Deterministic hosts can inject display preferences instead of following the OS.
pub fn init_with_preferences(
    cx: &mut gpui::App,
    preferences: Option<preferences::DisplayPreferences>,
) {
    gpui_component::init(cx);
    tokens::apply_appearance(false, cx);
    if let Some(preferences) = preferences {
        self::preferences::apply(preferences, cx);
    } else {
        self::preferences::refresh(cx);
        self::preferences::follow(cx);
    }
    #[cfg(target_os = "macos")]
    // SAFETY: GPUI has registered its AppKit classes before this callback;
    // this idempotent hook only forwards AX queries to AccessKit's content view.
    unsafe {
        murmur_install_focus_hook()
    };
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn murmur_install_focus_hook();
}

#[cfg(test)]
mod tests;
pub mod insights;
