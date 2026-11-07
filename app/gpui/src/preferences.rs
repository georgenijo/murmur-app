use gpui::{App, Global};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayPreferences {
    pub reduce_motion: bool,
    pub increase_contrast: bool,
    pub reduce_transparency: bool,
}
impl Global for DisplayPreferences {}

/// Inject preferences for a deterministic test or a future appearance owner.
pub fn apply(preferences: DisplayPreferences, cx: &mut App) {
    cx.set_reduce_motion(preferences.reduce_motion);
    cx.set_global(preferences);
}

/// Read-only AppKit query; skipped by GPUI's deterministic test scheduler.
/// Call before rendering a root. Live changes therefore apply on the next frame.
pub fn refresh(cx: &mut App) {
    if cx
        .background_executor()
        .scheduler_executor()
        .scheduler()
        .as_test()
        .is_some()
    {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        // SAFETY: render and init run on GPUI's main thread.
        let bits = unsafe { murmur_display_preferences() };
        let preferences = DisplayPreferences {
            reduce_motion: bits & 1 != 0,
            increase_contrast: bits & 2 != 0,
            reduce_transparency: bits & 4 != 0,
        };
        if preferences != current(cx) {
            apply(preferences, cx);
        }
    }
}

pub fn current(cx: &App) -> DisplayPreferences {
    cx.try_global::<DisplayPreferences>()
        .copied()
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn murmur_display_preferences() -> u32;
}

/// AppKit has no GPUI platform event for these preferences at the pinned rev.
/// Re-query on the main thread twice per second; only changes invalidate windows.
/// Tests and offscreen hosts use injected preferences and never start this task.
pub(crate) fn follow(cx: &mut App) {
    if cx
        .background_executor()
        .scheduler_executor()
        .scheduler()
        .as_test()
        .is_some()
    {
        return;
    }
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(500))
                .await;
            cx.update(|cx| {
                let previous = current(cx);
                refresh(cx);
                if previous != current(cx) {
                    cx.refresh_windows();
                }
            });
        }
    })
    .detach();
}
