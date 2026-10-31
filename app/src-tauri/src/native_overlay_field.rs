//! Native decorative field. The overlay webview remains the state/interaction
//! owner; only bounded state and scalar RMS cross this presentation seam.
use tauri::Manager;

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldStage {
    Idle,
    Recording,
    Processing,
    Done,
}
#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldStyle {
    Sweep,
    Flow,
    Pulse,
}

#[cfg(target_os = "macos")]
extern "C" {
    fn murmur_overlay_dot_state(
        window: *mut std::ffi::c_void,
        stage: i32,
        style: i32,
        reduced: bool,
        visible: bool,
        top: f64,
        idle_w: f64,
        recording_w: f64,
        processing_w: f64,
        height: f64,
        radius: f64,
    ) -> bool;
    fn murmur_overlay_dot_level(level: f64);
}

/// Called by the existing bounded level publisher, never the real-time audio
/// callback. The Swift function takes a short lock and does no UI work.
pub fn level(rms: f32) {
    #[cfg(target_os = "macos")]
    unsafe {
        murmur_overlay_dot_level(f64::from(rms));
    }
}

#[tauri::command]
pub async fn set_overlay_field_state(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    stage: FieldStage,
    style: FieldStyle,
    reduced: bool,
    visible: bool,
) -> Result<bool, String> {
    if window.label() != "overlay" {
        return Err("overlay field is owned by the overlay window".into());
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, stage, style, reduced, visible);
        Ok(false)
    }
    #[cfg(target_os = "macos")]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let dispatch = app.clone();
        app.run_on_main_thread(move || {
            let result = (|| {
                if tx.is_closed() {
                    return Err("field state request expired".to_string());
                }
                let geometry =
                    crate::commands::overlay::get_overlay_geometry(dispatch.state(), None);
                let pointer = window.ns_window().map_err(|e| e.to_string())?;
                // All NSWindow/view work happens on this main-thread dispatch.
                Ok(unsafe {
                    murmur_overlay_dot_state(
                        pointer,
                        match stage {
                            FieldStage::Idle => 0,
                            FieldStage::Recording => 1,
                            FieldStage::Processing => 2,
                            FieldStage::Done => 3,
                        },
                        match style {
                            FieldStyle::Sweep => 0,
                            FieldStyle::Flow => 1,
                            FieldStyle::Pulse => 2,
                        },
                        reduced,
                        visible,
                        geometry.island.notch_h + geometry.island.pad,
                        geometry.island.plate_idle_w,
                        geometry.island.plate_recording_w,
                        geometry.island.plate_processing_w,
                        geometry.island.plate_h,
                        geometry.island.inner_radius,
                    )
                })
            })();
            let _ = tx.send(result);
        })
        .map_err(|e| e.to_string())?;
        rx.await.map_err(|e| e.to_string())?
    }
}
