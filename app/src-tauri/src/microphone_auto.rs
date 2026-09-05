//! Deterministic, cached-only policy for opt-in Smart Auto dictation input.
//!
//! This module never enumerates or opens a microphone. Its caller supplies the
//! shared inventory snapshot taken while capture is idle, then freezes the
//! returned stable Core Audio UID for the complete recording generation.

use crate::audio::AudioDeviceDescriptor;
use murmur_capture_helper_protocol::{ProductionDeviceKind, ProductionLidState};
use serde::Deserialize;
use std::collections::HashSet;

const MAX_APPROVED_DEVICES: usize = 32;
const MAX_STABLE_ID_BYTES: usize = 4_096;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SmartAutoRequest {
    pub(crate) approved_device_ids: Vec<String>,
    pub(crate) preferred_device_ids: Vec<String>,
    pub(crate) allow_continuity: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SmartAutoReason {
    PreferredApproved,
    ApprovedMacosDefault,
    ApprovedExternalFallback,
    ApprovedContinuityFallback,
}

impl SmartAutoReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::PreferredApproved => "preferred_approved",
            Self::ApprovedMacosDefault => "approved_macos_default",
            Self::ApprovedExternalFallback => "approved_external_fallback",
            Self::ApprovedContinuityFallback => "approved_continuity_fallback",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SmartAutoSelection {
    pub(crate) device_id: String,
    pub(crate) reason: SmartAutoReason,
}

fn valid_stable_id(value: &str) -> bool {
    !value.is_empty() && !value.contains('\0') && value.len() <= MAX_STABLE_ID_BYTES
}

fn validate(request: &SmartAutoRequest) -> Result<HashSet<&str>, &'static str> {
    if request.approved_device_ids.is_empty()
        || request.approved_device_ids.len() > MAX_APPROVED_DEVICES
        || request.preferred_device_ids.len() > MAX_APPROVED_DEVICES
    {
        return Err("Smart Auto needs one to 32 approved microphones.");
    }
    let approved: HashSet<&str> = request
        .approved_device_ids
        .iter()
        .map(String::as_str)
        .collect();
    if approved.len() != request.approved_device_ids.len()
        || !approved.iter().all(|id| valid_stable_id(id))
        || request
            .preferred_device_ids
            .iter()
            .any(|id| !valid_stable_id(id) || !approved.contains(id.as_str()))
    {
        return Err("Smart Auto microphone approval is invalid.");
    }
    Ok(approved)
}

fn is_eligible(
    device: &AudioDeviceDescriptor,
    approved: &HashSet<&str>,
    lid_state: ProductionLidState,
    allow_continuity: bool,
) -> bool {
    if !approved.contains(device.id.as_str()) || !device.connected || !device.has_input {
        return false;
    }
    match device.kind {
        ProductionDeviceKind::External => true,
        ProductionDeviceKind::Continuity => allow_continuity,
        // An uncertain clamshell state cannot authorize a built-in microphone.
        ProductionDeviceKind::BuiltIn => lid_state == ProductionLidState::Open,
        ProductionDeviceKind::Unknown => false,
    }
}

pub(crate) fn select(
    request: &SmartAutoRequest,
    devices: &[AudioDeviceDescriptor],
    default_input_id: Option<&str>,
    lid_state: ProductionLidState,
) -> Result<SmartAutoSelection, &'static str> {
    let approved = validate(request)?;
    let eligible = |device: &AudioDeviceDescriptor| {
        is_eligible(device, &approved, lid_state, request.allow_continuity)
    };

    for preferred_id in &request.preferred_device_ids {
        if let Some(device) = devices
            .iter()
            .find(|device| device.id == *preferred_id && eligible(device))
        {
            return Ok(SmartAutoSelection {
                device_id: device.id.clone(),
                reason: SmartAutoReason::PreferredApproved,
            });
        }
    }
    if let Some(default_id) = default_input_id {
        if let Some(device) = devices
            .iter()
            .find(|device| device.id == default_id && eligible(device))
        {
            return Ok(SmartAutoSelection {
                device_id: device.id.clone(),
                reason: SmartAutoReason::ApprovedMacosDefault,
            });
        }
    }
    let external = devices
        .iter()
        .filter(|device| device.kind == ProductionDeviceKind::External && eligible(device))
        .min_by(|left, right| left.id.cmp(&right.id));
    if let Some(device) = external {
        return Ok(SmartAutoSelection {
            device_id: device.id.clone(),
            reason: SmartAutoReason::ApprovedExternalFallback,
        });
    }
    let continuity = devices
        .iter()
        .filter(|device| device.kind == ProductionDeviceKind::Continuity && eligible(device))
        .min_by(|left, right| left.id.cmp(&right.id));
    if let Some(device) = continuity {
        return Ok(SmartAutoSelection {
            device_id: device.id.clone(),
            reason: SmartAutoReason::ApprovedContinuityFallback,
        });
    }
    Err("No approved, usable microphone is available for Smart Auto.")
}

/// A lid transition does not necessarily change Core Audio topology or the
/// macOS default input. Read this small public display fact at the selection
/// boundary, while device membership still comes from the bounded cache.
pub(crate) fn current_lid_state() -> ProductionLidState {
    #[cfg(target_os = "macos")]
    {
        return macos_lid_state().unwrap_or(ProductionLidState::Unknown);
    }
    #[cfg(not(target_os = "macos"))]
    ProductionLidState::Unknown
}

/// Keep every ordinary capture entry point on the same contract: a manual
/// stable ID or System Default remains untouched, while Smart Auto resolves
/// once to a stable ID before that owner starts.
pub(crate) fn resolve_capture_device(
    device_name: Option<String>,
    smart_auto: Option<&SmartAutoRequest>,
) -> Result<Option<String>, String> {
    if smart_auto.is_some() && device_name.is_some() {
        return Err("Smart Auto cannot be combined with a fixed microphone.".to_string());
    }
    match smart_auto {
        Some(request) => crate::audio_inventory::resolve_smart_auto(request)
            .map(|selection| Some(selection.device_id)),
        None => Ok(device_name),
    }
}

#[cfg(target_os = "macos")]
fn macos_lid_state() -> Option<ProductionLidState> {
    use core_foundation::base::TCFType;
    use core_foundation::base::{CFAllocatorRef, CFGetTypeID, CFRelease, CFTypeRef};
    use core_foundation::boolean::{kCFBooleanTrue, CFBooleanGetTypeID, CFBooleanRef};
    use core_foundation::string::CFString;
    use core_foundation::string::CFStringRef;
    use std::ffi::{c_char, CStr};

    type IoRegistryEntry = u32;
    type KernReturn = i32;

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IORegistryEntryFromPath(main_port: u32, path: *const c_char) -> IoRegistryEntry;
        fn IORegistryEntryCreateCFProperty(
            entry: IoRegistryEntry,
            key: CFStringRef,
            allocator: CFAllocatorRef,
            options: u32,
        ) -> CFTypeRef;
        fn IOObjectRelease(object: IoRegistryEntry) -> KernReturn;
    }

    let path = CStr::from_bytes_with_nul(b"IOService:/\0").ok()?;
    let entry = unsafe { IORegistryEntryFromPath(0, path.as_ptr()) };
    if entry == 0 {
        return None;
    }
    let key = CFString::new("AppleClamshellState");
    let property = unsafe {
        IORegistryEntryCreateCFProperty(entry, key.as_concrete_TypeRef(), std::ptr::null(), 0)
    };
    unsafe {
        let _ = IOObjectRelease(entry);
    }
    if property.is_null() {
        return None;
    }
    let is_boolean = unsafe { CFGetTypeID(property) == CFBooleanGetTypeID() };
    let closed = if is_boolean {
        Some(property as CFBooleanRef == unsafe { kCFBooleanTrue })
    } else {
        None
    };
    unsafe { CFRelease(property) };
    closed.map(|is_closed| {
        if is_closed {
            ProductionLidState::Closed
        } else {
            ProductionLidState::Open
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, kind: ProductionDeviceKind) -> AudioDeviceDescriptor {
        AudioDeviceDescriptor {
            id: id.to_string(),
            name: id.to_string(),
            kind,
            connected: true,
            has_input: true,
        }
    }

    fn request(approved: &[&str], preferred: &[&str]) -> SmartAutoRequest {
        SmartAutoRequest {
            approved_device_ids: approved.iter().map(|value| (*value).to_string()).collect(),
            preferred_device_ids: preferred.iter().map(|value| (*value).to_string()).collect(),
            allow_continuity: false,
        }
    }

    #[test]
    fn uses_the_first_available_explicit_preference() {
        let selected = select(
            &request(&["anker", "studio"], &["anker", "studio"]),
            &[
                device("studio", ProductionDeviceKind::External),
                device("anker", ProductionDeviceKind::External),
            ],
            Some("studio"),
            ProductionLidState::Open,
        )
        .unwrap();
        assert_eq!(selected.device_id, "anker");
        assert_eq!(selected.reason, SmartAutoReason::PreferredApproved);
    }

    #[test]
    fn uses_an_approved_macos_default_before_the_deterministic_fallback() {
        let selected = select(
            &request(&["zebra", "anker"], &[]),
            &[
                device("zebra", ProductionDeviceKind::External),
                device("anker", ProductionDeviceKind::External),
            ],
            Some("zebra"),
            ProductionLidState::Open,
        )
        .unwrap();
        assert_eq!(selected.device_id, "zebra");
        assert_eq!(selected.reason, SmartAutoReason::ApprovedMacosDefault);
    }

    #[test]
    fn fallback_is_stable_by_id_and_never_auto_enrolls_a_new_device() {
        let selected = select(
            &request(&["zebra", "anker"], &[]),
            &[
                device("unapproved", ProductionDeviceKind::External),
                device("zebra", ProductionDeviceKind::External),
                device("anker", ProductionDeviceKind::External),
            ],
            None,
            ProductionLidState::Open,
        )
        .unwrap();
        assert_eq!(selected.device_id, "anker");
        assert_eq!(selected.reason, SmartAutoReason::ApprovedExternalFallback);
    }

    #[test]
    fn rejects_builtin_when_the_lid_is_closed_or_unknown() {
        let devices = [device("built-in", ProductionDeviceKind::BuiltIn)];
        for lid_state in [ProductionLidState::Closed, ProductionLidState::Unknown] {
            assert!(select(
                &request(&["built-in"], &[]),
                &devices,
                Some("built-in"),
                lid_state
            )
            .is_err());
        }
    }

    #[test]
    fn a_lid_only_change_excludes_a_cached_builtin_without_a_topology_change() {
        let devices = [device("built-in", ProductionDeviceKind::BuiltIn)];
        assert!(select(
            &request(&["built-in"], &[]),
            &devices,
            Some("built-in"),
            ProductionLidState::Open,
        )
        .is_ok());
        assert!(select(
            &request(&["built-in"], &[]),
            &devices,
            Some("built-in"),
            ProductionLidState::Closed,
        )
        .is_err());
    }

    #[test]
    fn rejects_disconnected_output_only_unknown_and_unapproved_devices() {
        let mut disconnected = device("disconnected", ProductionDeviceKind::External);
        disconnected.connected = false;
        let mut output_only = device("output-only", ProductionDeviceKind::External);
        output_only.has_input = false;
        assert!(select(
            &request(&["disconnected", "output-only", "unknown"], &[]),
            &[
                disconnected,
                output_only,
                device("unknown", ProductionDeviceKind::Unknown)
            ],
            None,
            ProductionLidState::Open,
        )
        .is_err());
    }

    #[test]
    fn continuity_requires_explicit_opt_in() {
        let devices = [device("iphone", ProductionDeviceKind::Continuity)];
        let mut allowed = request(&["iphone"], &[]);
        assert!(select(&allowed, &devices, Some("iphone"), ProductionLidState::Open).is_err());
        allowed.allow_continuity = true;
        assert_eq!(
            select(&allowed, &devices, Some("iphone"), ProductionLidState::Open)
                .unwrap()
                .reason,
            SmartAutoReason::ApprovedMacosDefault,
        );
    }
}
