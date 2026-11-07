use gpui::{App, Global};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayPreferences {
    pub reduce_motion: bool,
    pub increase_contrast: bool,
    pub reduce_transparency: bool,
}
impl Global for DisplayPreferences {}

#[derive(Clone, Copy)]
struct SystemAppearance(bool);
impl Global for SystemAppearance {}

/// Apply one native observation. Appearance and accessibility preferences share
/// the same main-thread watcher and window invalidation decision.
pub(crate) fn apply_snapshot(bits: u32, cx: &mut App) -> bool {
    let preferences = DisplayPreferences {
        reduce_motion: bits & 1 != 0,
        increase_contrast: bits & 2 != 0,
        reduce_transparency: bits & 4 != 0,
    };
    let dark = bits & 8 != 0;
    let appearance_changed = cx.try_global::<SystemAppearance>().map(|a| a.0) != Some(dark);
    let preferences_changed = preferences != current(cx);
    if appearance_changed {
        crate::themes::appearance::system_changed(dark, cx);
        cx.set_global(SystemAppearance(dark));
    }
    if preferences_changed {
        apply(preferences, cx);
    }
    appearance_changed || preferences_changed
}

/// Inject preferences for a deterministic test or a future appearance owner.
pub fn apply(preferences: DisplayPreferences, cx: &mut App) {
    cx.set_reduce_motion(preferences.reduce_motion);
    cx.set_global(preferences);
}

/// Read-only AppKit query; skipped by GPUI's deterministic test scheduler.
/// Call before rendering a root. Live changes therefore apply on the next frame.
pub fn refresh(cx: &mut App) -> bool {
    if cx
        .background_executor()
        .scheduler_executor()
        .scheduler()
        .as_test()
        .is_some()
    {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        // SAFETY: render and init run on GPUI's main thread.
        let bits = unsafe { murmur_display_preferences() };
        apply_snapshot(bits, cx)
    }
    #[cfg(not(target_os = "macos"))]
    false
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
                if refresh(cx) {
                    cx.refresh_windows();
                }
            });
        }
    })
    .detach();
}
