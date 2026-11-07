//! Native tray presentation is injected; Mode/disable/update ownership is shared.
use crate::host::NativePorts;
use std::sync::{Arc, OnceLock};
static PORTS: OnceLock<Arc<dyn NativePorts>> = OnceLock::new();
pub(crate) fn install(ports: Arc<dyn NativePorts>) {
    let _ = PORTS.set(ports);
}
pub(crate) fn set_mode_menu_status(status: &crate::commands::mode_runtime::ModeRuntimeStatus) {
    if let Some(ports) = PORTS.get() {
        if let Ok(value) = serde_json::to_value(status) {
            ports.mode_status(value);
        }
    }
}
pub fn set_tray_update_available(version: Option<String>) -> Result<(), String> {
    PORTS
        .get()
        .ok_or("Native tray unavailable")?
        .tray_update(version)
        .map_err(String::from)
}
pub fn update_tray_icon(_app: crate::host::Context, _icon_state: String) -> Result<(), String> {
    Ok(())
}
