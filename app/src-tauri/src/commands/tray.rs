use std::sync::OnceLock;
use tauri::menu::MenuItem;

static UPDATE_MENU_ITEM: OnceLock<MenuItem<tauri::Wry>> = OnceLock::new();

pub(crate) fn register_tray_update_item(item: MenuItem<tauri::Wry>) {
    let _ = UPDATE_MENU_ITEM.set(item);
}

fn update_menu_label(version: Option<&str>) -> Result<String, String> {
    let Some(version) = version else {
        return Ok("Check for Updates…".to_string());
    };
    let version = version.trim().trim_start_matches('v');
    if version.is_empty()
        || version.len() > 64
        || !version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
    {
        return Err("Invalid update version for menu-bar item".to_string());
    }
    Ok(format!("Update Murmur to v{version}…"))
}

#[tauri::command]
pub fn set_tray_update_available(version: Option<String>) -> Result<(), String> {
    let label = update_menu_label(version.as_deref())?;
    if let Some(item) = UPDATE_MENU_ITEM.get() {
        item.set_text(label).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn register_update_wake_observer(app_handle: tauri::AppHandle) {
    use objc2_app_kit::{NSWorkspace, NSWorkspaceDidWakeNotification};
    use objc2_foundation::{NSNotification, NSOperationQueue};
    use tauri::Emitter;

    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    let block = block2::RcBlock::new(move |_notification: std::ptr::NonNull<NSNotification>| {
        let _ = app_handle.emit("updater-background-check-requested", ());
    });
    unsafe {
        let observer = center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidWakeNotification),
            None,
            Some(&NSOperationQueue::mainQueue()),
            &block,
        );
        std::mem::forget(observer);
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn register_update_wake_observer(_app_handle: tauri::AppHandle) {}

/// Generate 66×66 RGBA pixel data for an audio-bar tray icon (static white).
/// 66px = 3× resolution for a 22pt menu-bar icon (crisp on Retina).
/// Draws 5 vertical capsule bars at varying heights (waveform / equalizer style).
pub(crate) fn make_tray_icon_data() -> Vec<u8> {
    let (r, g, b): (u8, u8, u8) = (255, 255, 255);
    const SIZE: u32 = 66;
    let mut data = vec![0u8; (SIZE * SIZE * 4) as usize];

    // 5 vertical bars: (x-center, height) — all coords in 3× pixel space
    let bars: [(f64, f64); 5] = [
        (9.0, 18.0),
        (21.0, 36.0),
        (33.0, 48.0),
        (45.0, 30.0),
        (57.0, 18.0),
    ];
    let half_w: f64 = 3.0; // 6px wide bars (2pt at 3×)
    let cy: f64 = 33.0; // vertical center of canvas
    let rr: f64 = 3.0; // corner rounding (= half_w → capsule ends)
    let aa: f64 = 1.0; // anti-alias transition width

    for y in 0..SIZE {
        for x in 0..SIZE {
            let px = x as f64 + 0.5;
            let py = y as f64 + 0.5;
            let mut alpha: f64 = 0.0;

            for &(cx, h) in &bars {
                let half_h = h / 2.0;
                // Rounded-rect signed distance (capsule when rr == half_w)
                let qx = (px - cx).abs() - half_w + rr;
                let qy = (py - cy).abs() - half_h + rr;
                let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
                let inside = qx.max(qy).min(0.0);
                let sdf = outside + inside - rr;
                let a = (1.0 - sdf.max(0.0) / aa).max(0.0);
                alpha = alpha.max(a);
            }

            if alpha > 0.0 {
                let idx = ((y * SIZE + x) * 4) as usize;
                data[idx] = r;
                data[idx + 1] = g;
                data[idx + 2] = b;
                data[idx + 3] = (alpha * 255.0).round() as u8;
            }
        }
    }
    data
}

/// No-op — tray icon is static white. Kept so the registered command doesn't break.
#[tauri::command]
pub fn update_tray_icon(_app: tauri::AppHandle, _icon_state: String) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: usize = 66;

    #[test]
    fn tray_icon_data_correct_size() {
        let data = make_tray_icon_data();
        assert_eq!(data.len(), SIZE * SIZE * 4);
    }

    #[test]
    fn tray_icon_center_pixel_is_opaque_white() {
        let data = make_tray_icon_data();
        let idx = (33 * SIZE + 33) * 4;
        assert_eq!(data[idx], 255, "R");
        assert_eq!(data[idx + 1], 255, "G");
        assert_eq!(data[idx + 2], 255, "B");
        assert_eq!(data[idx + 3], 255, "A should be opaque");
    }

    #[test]
    fn tray_icon_corner_pixel_is_transparent() {
        let data = make_tray_icon_data();
        for &(row, col) in &[(0, 0), (0, 65), (65, 0), (65, 65)] {
            let idx = (row * SIZE + col) * 4;
            assert_eq!(
                data[idx + 3],
                0,
                "corner ({row},{col}) alpha should be 0 (transparent)"
            );
        }
    }

    #[test]
    fn update_menu_label_tracks_available_version() {
        assert_eq!(
            update_menu_label(Some("v0.23.0")).unwrap(),
            "Update Murmur to v0.23.0…"
        );
        assert_eq!(update_menu_label(None).unwrap(), "Check for Updates…");
    }

    #[test]
    fn update_menu_label_rejects_unbounded_or_unsafe_versions() {
        assert!(update_menu_label(Some("")).is_err());
        assert!(update_menu_label(Some("0.23.0\nQuit Murmur")).is_err());
        assert!(update_menu_label(Some(&"1".repeat(65))).is_err());
    }
}
