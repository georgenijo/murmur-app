//! Legacy global stores retain production locations only for the production ID.
//! All other bundle IDs get an injective, path-safe namespace, including future IDs.
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};
pub(crate) const PRODUCTION: &str = "com.localdictation";
static IDENTIFIER: OnceLock<String> = OnceLock::new();
static PREVIEW_REAL_DELIVERY: OnceLock<bool> = OnceLock::new();
const PREVIEW_REAL_DELIVERY_KEY: &str = "MurmurPreviewRealDelivery";

static NATIVE_SUPPORT: OnceLock<PathBuf> = OnceLock::new();

pub(crate) fn initialize_native(identifier: &str, home: &Path) -> Result<(), String> {
    if !home.is_absolute() {
        return Err("Native home path unavailable".into());
    }
    let support = home.join("Library/Application Support");
    if NATIVE_SUPPORT.get().is_some_and(|old| *old != support)
        || initialized_identifier().is_some_and(|old| old != identifier)
    {
        return Err("Native identity already initialized".into());
    }
    let _ = NATIVE_SUPPORT.set(support);
    // Main-bundle metadata only, frozen before worker dispatch. No environment
    // or saved preference can opt an automated test into external delivery.
    PREVIEW_REAL_DELIVERY.get_or_init(preview_real_delivery_from_bundle);
    initialize(identifier);
    Ok(())
}

/// Failure to inspect legacy storage is not evidence that it is absent.
/// Never commit an empty migration/retirement on a permission or I/O error.
pub(crate) fn legacy_store_present(directory: &Path) -> Result<bool, String> {
    directory
        .try_exists()
        .map_err(|_| "Legacy store availability is unknown".to_owned())
}

pub(crate) fn native_support() -> Option<&'static Path> {
    NATIVE_SUPPORT.get().map(PathBuf::as_path)
}
pub(crate) fn initialize(identifier: &str) {
    let _ = IDENTIFIER.set(identifier.to_owned());
}
pub(crate) fn initialized_identifier() -> Option<&'static str> {
    IDENTIFIER.get().map(String::as_str)
}
/// Production, or a deliberately opted-in manual preview, may reach OS input.
pub(crate) fn external_input_allowed() -> bool {
    input_allowed_for(
        initialized_identifier(),
        PREVIEW_REAL_DELIVERY.get().copied().unwrap_or(false),
    )
}
fn input_allowed_for(identifier: Option<&str>, preview_opt_in: bool) -> bool {
    if preview_opt_in
        && identifier.is_some_and(|id| {
            !id.is_empty()
                && !matches!(
                    id,
                    "com.localdictation.bench" | "com.localdictation.shutdown-test"
                )
        })
    {
        return true;
    }
    identifier == Some(PRODUCTION)
}
pub(crate) fn private_pasteboard_name(identifier: Option<&str>) -> Option<String> {
    pasteboard_name_for(
        identifier,
        PREVIEW_REAL_DELIVERY.get().copied().unwrap_or(false),
    )
}
fn pasteboard_name_for(identifier: Option<&str>, preview_opt_in: bool) -> Option<String> {
    if input_allowed_for(identifier, preview_opt_in) {
        return None;
    }
    let encoded: String = identifier
        .unwrap_or("uninitialized")
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    // The option discriminator also separates None from Some("uninitialized").
    Some(format!(
        "com.localdictation.private-pasteboard.{}.{encoded}",
        if identifier.is_some() { "id" } else { "absent" }
    ))
}

#[cfg(target_os = "macos")]
fn preview_real_delivery_from_bundle() -> bool {
    objc2::rc::autoreleasepool(|_| {
        use objc2_foundation::{NSBundle, NSString};
        let value = NSBundle::mainBundle()
            .objectForInfoDictionaryKey(&NSString::from_str(PREVIEW_REAL_DELIVERY_KEY));
        preview_real_delivery_value(value.as_deref())
    })
}
#[cfg(target_os = "macos")]
fn preview_real_delivery_value(value: Option<&objc2::runtime::AnyObject>) -> bool {
    use core_foundation::{base::CFGetTypeID, boolean::CFBooleanGetTypeID};
    value.is_some_and(|value| {
        // CFBoolean is toll-free bridged to NSNumber. Strings and integers (even
        // 1) must fail closed rather than accepting NSNumber's coercions.
        (unsafe { CFGetTypeID(std::ptr::from_ref(value).cast()) == CFBooleanGetTypeID() })
            && value
                .downcast_ref::<objc2_foundation::NSNumber>()
                .is_some_and(|number| number.as_bool())
    })
}
#[cfg(not(target_os = "macos"))]
fn preview_real_delivery_from_bundle() -> bool {
    let _ = PREVIEW_REAL_DELIVERY_KEY;
    false
}
pub(crate) fn identifier() -> &'static str {
    IDENTIFIER.get().map(String::as_str).unwrap_or(PRODUCTION)
}
pub(crate) fn internal() -> bool {
    identifier() != PRODUCTION
}
pub(crate) fn namespace(root: &Path, identifier: &str) -> PathBuf {
    if identifier == PRODUCTION {
        return root.to_owned();
    }
    let encoded: String = identifier
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    root.join("murmur-isolated").join(encoded)
}
pub(crate) fn data_dir() -> Option<PathBuf> {
    native_support().map(|root| namespace(root, identifier()))
}
pub(crate) fn temp_dir() -> PathBuf {
    namespace(&std::env::temp_dir(), identifier())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manual_preview_delivery_requires_explicit_opt_in_and_an_allowed_identity() {
        for id in [
            None,
            Some(""),
            Some("com.localdictation.bench"),
            Some("com.localdictation.shutdown-test"),
            Some("com.localdictation.gpui-preview.app"),
            Some("future.id"),
        ] {
            assert!(!input_allowed_for(id, false));
            let encoded: String = id
                .unwrap_or("uninitialized")
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            let original = Some(format!(
                "com.localdictation.private-pasteboard.{}.{encoded}",
                if id.is_some() { "id" } else { "absent" }
            ));
            assert_eq!(pasteboard_name_for(id, false), original);
            let allowed = matches!(
                id,
                Some("com.localdictation.gpui-preview.app" | "future.id")
            );
            assert_eq!(input_allowed_for(id, true), allowed);
            assert_eq!(
                pasteboard_name_for(id, true),
                if allowed { None } else { original }
            );
        }
        for opt_in in [false, true] {
            assert!(input_allowed_for(Some(PRODUCTION), opt_in));
            assert_eq!(pasteboard_name_for(Some(PRODUCTION), opt_in), None);
        }
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn preview_bundle_opt_in_accepts_only_a_true_plist_boolean() {
        use objc2_foundation::{NSNumber, NSString};
        objc2::rc::autoreleasepool(|_| {
            assert_eq!(PREVIEW_REAL_DELIVERY_KEY, "MurmurPreviewRealDelivery");
            assert!(!preview_real_delivery_value(None));
            assert!(preview_real_delivery_value(Some(&NSNumber::new_bool(true))));
            assert!(!preview_real_delivery_value(Some(&NSNumber::new_bool(
                false
            ))));
            assert!(!preview_real_delivery_value(Some(&NSNumber::new_i32(1))));
            assert!(!preview_real_delivery_value(Some(&NSString::from_str(
                "true"
            ))));
        });
    }
    #[test]
    fn clipboard_and_input_routing_fails_closed_without_touching_pasteboards() {
        assert!(input_allowed_for(Some(PRODUCTION), false));
        assert_eq!(private_pasteboard_name(Some(PRODUCTION)), None);
        let mut names = std::collections::HashSet::new();
        for id in [
            None,
            Some("uninitialized"),
            Some(""),
            Some("com.localdictation.shutdown-test"),
            Some("com.localdictation.bench"),
            Some("com.localdictation.refactor-test"),
            Some("future.id"),
            Some("../com.localdictation"),
        ] {
            assert!(!input_allowed_for(id, false));
            let name = private_pasteboard_name(id).unwrap();
            assert!(!name.contains('/'));
            assert!(names.insert(name));
        }
    }
    #[test]
    fn every_nonproduction_identity_is_disjoint_from_production_and_each_other() {
        let root = Path::new("/synthetic");
        let ids = [
            "com.localdictation.bench",
            "com.localdictation.refactor-test",
            "com.localdictation.shutdown-test",
            "future.id",
            "../com.localdictation",
            "",
        ];
        let mut paths = std::collections::HashSet::new();
        for id in ids {
            let isolated = namespace(root, id);
            for suffix in [
                "local-dictation/logs",
                "local-dictation/models",
                "FluidAudio/Models",
                "Murmur Benchmark Corpus/v1",
                "pending",
                "cache",
                "socket",
            ] {
                assert_ne!(isolated.join(suffix), root.join(suffix));
                assert!(!isolated
                    .join(suffix)
                    .starts_with(root.join("local-dictation")));
            }
            assert!(paths.insert(isolated));
        }
    }
}

#[cfg(test)]
mod legacy_probe_tests {
    use super::*;
    #[test]
    fn missing_present_and_unreadable_legacy_roots_are_distinct() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().join("private");
        std::fs::create_dir(&parent).unwrap();
        let store = parent.join("synthetic-store");
        assert_eq!(legacy_store_present(&store), Ok(false));
        std::fs::create_dir(&store).unwrap();
        assert_eq!(legacy_store_present(&store), Ok(true));
        struct Restore(PathBuf);
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
            }
        }
        let _restore = Restore(parent.clone());
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o000)).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            assert!(legacy_store_present(&store).is_err());
        }
    }
}
