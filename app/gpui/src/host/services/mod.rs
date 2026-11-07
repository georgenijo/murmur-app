//! Production composition only. QA/gallery entry points keep their injected
//! fixtures and never create these service owners or register OS capabilities.
pub mod autostart;
pub mod os;
pub mod process;

impl super::lifecycle::ProcessLifecycle for murmur_services::native_services::Services {
    fn request(&self, request: murmur_core::shutdown::Request) {
        self.shutdown(request)
    }
    fn permitted(&self) -> bool {
        self.termination_permitted()
    }
    fn exited(&self) {
        self.terminal_receipt()
    }
}

pub mod windows;

pub mod login;

/// Production text editing shares the same identity-aware pasteboard owner as
/// recording delivery. An unavailable private board never falls back to general.
pub fn install_clipboard(services: murmur_services::native_services::Services, cx: &mut gpui::App) {
    struct Clipboard(murmur_services::native_services::Services);
    impl gpui::ClipboardProvider for Clipboard {
        fn read(&self) -> Option<gpui::ClipboardItem> {
            self.0
                .clipboard_text()
                .ok()
                .filter(|s| !s.is_empty())
                .map(gpui::ClipboardItem::new_string)
        }
        fn write(&self, item: gpui::ClipboardItem) {
            if let Some(text) = item.text() {
                let _ = self.0.copy_text(&text);
            }
        }
    }
    cx.set_clipboard_provider(std::rc::Rc::new(Clipboard(services)));
}
