//! Per-window destruction is independent of website-data-store retirement.
//! The native-parity host must release every strong owner, not just hide/remove
//! the view. Keep this WebKit code out of the native UI dependency graph.
use objc2::{
    rc::{Retained, Weak},
    MainThreadMarker,
};
use objc2_web_kit::WKWebView;

pub struct OverlayRetirement {
    view: Weak<WKWebView>,
}
impl OverlayRetirement {
    /// Consume the host's owned overlay view on main. Another retained owner
    /// keeps the barrier closed until it too is released. No store is purged:
    /// unported windows and the standalone migration importer still need it.
    pub fn destroy(_main: MainThreadMarker, view: Retained<WKWebView>) -> Self {
        let weak = Weak::from_retained(&view);
        // SAFETY: the retained view and the main-thread token prove lifetime
        // and AppKit affinity; no user content is read or evaluated.
        unsafe {
            view.stopLoading();
            view.setNavigationDelegate(None);
            view.setUIDelegate(None);
            view.removeFromSuperview();
        }
        drop(view);
        Self { view: weak }
    }
    /// Inventory only the retired overlay, never unrelated WebKit processes.
    /// A zero count proves ObjC destruction rather than visibility or detachment.
    pub fn remaining_webviews(&self, _main: MainThreadMarker) -> usize {
        usize::from(self.view.load().is_some())
    }
}
