//! Runtime validation for bundled helper code identities.
//!
//! The release path validates the exact nested executable before spawn using
//! Security.framework. Path pinning alone is insufficient: the helper must be
//! valid Developer ID code, carry its fixed identifier, share the main app's
//! Team ID, and have the hardened-runtime flag.

/// A macOS Team ID is exactly 10 uppercase-alphanumeric ASCII characters
/// (e.g. `"ABCDE12345"`). Pulled out of the macOS-only validation path so it
/// compiles and is unit-testable on every platform, even though it only ever
/// runs against real Security.framework output on macOS.
pub(crate) fn team_id_is_well_formed(team_id: &str) -> bool {
    team_id.len() == 10
        && team_id
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

/// Build the `SecRequirement` text pinning a helper to its expected bundle
/// identifier, an Apple-issued anchor, and the main app's own Team ID. Pulled
/// out of the macOS-only validation path so the exact requirement string is
/// unit-testable on every platform.
pub(crate) fn requirement_text(expected_identifier: &str, team_id: &str) -> String {
    format!(
        "identifier \"{expected_identifier}\" and anchor apple generic and certificate leaf[subject.OU] = \"{team_id}\""
    )
}

#[cfg(target_os = "macos")]
mod macos {
    use core_foundation::base::TCFType;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::{CFNumber, CFNumberRef};
    use core_foundation::string::{CFString, CFStringRef};
    use core_foundation::url::CFURL;
    use security_framework::os::macos::code_signing::{
        Flags, SecCode, SecRequirement, SecStaticCode,
    };
    use std::ffi::c_void;
    use std::path::Path;

    const SIGNING_INFORMATION: u32 = 1 << 1;
    const CS_RUNTIME: i64 = 0x0001_0000;

    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        static kSecCodeInfoFlags: CFStringRef;
        static kSecCodeInfoTeamIdentifier: CFStringRef;
        fn SecCodeCopySigningInformation(
            code: *const c_void,
            flags: u32,
            information: *mut core_foundation::dictionary::CFDictionaryRef,
        ) -> i32;
    }

    fn signing_information<T: TCFType>(
        code: &T,
    ) -> Result<CFDictionary<*const c_void, *const c_void>, ()> {
        let mut dictionary = std::ptr::null();
        let status = unsafe {
            SecCodeCopySigningInformation(code.as_CFTypeRef(), SIGNING_INFORMATION, &mut dictionary)
        };
        if status != 0 || dictionary.is_null() {
            return Err(());
        }
        Ok(unsafe { CFDictionary::wrap_under_create_rule(dictionary) })
    }

    fn dictionary_string(
        dictionary: &CFDictionary<*const c_void, *const c_void>,
        key: CFStringRef,
    ) -> Result<String, ()> {
        let value = dictionary.find(key.cast()).ok_or(())?;
        let string = unsafe { CFString::wrap_under_get_rule(*value as CFStringRef) };
        Ok(string.to_string())
    }

    fn dictionary_number(
        dictionary: &CFDictionary<*const c_void, *const c_void>,
        key: CFStringRef,
    ) -> Result<i64, ()> {
        let value = dictionary.find(key.cast()).ok_or(())?;
        let number = unsafe { CFNumber::wrap_under_get_rule(*value as CFNumberRef) };
        number.to_i64().ok_or(())
    }

    pub fn validate(path: &Path, expected_identifier: &str) -> Result<(), ()> {
        let self_code = SecCode::for_self(Flags::NONE).map_err(|_| ())?;
        let self_information = signing_information(&self_code)?;
        let team_id = dictionary_string(&self_information, unsafe { kSecCodeInfoTeamIdentifier })?;
        if !super::team_id_is_well_formed(&team_id) {
            return Err(());
        }

        let url = CFURL::from_path(path, false).ok_or(())?;
        let helper = SecStaticCode::from_path(&url, Flags::NONE).map_err(|_| ())?;
        let requirement: SecRequirement = super::requirement_text(expected_identifier, &team_id)
            .parse()
            .map_err(|_| ())?;
        helper
            .check_validity(
                Flags::CHECK_ALL_ARCHITECTURES | Flags::STRICT_VALIDATE | Flags::NO_NETWORK_ACCESS,
                &requirement,
            )
            .map_err(|_| ())?;

        let helper_information = signing_information(&helper)?;
        let helper_team =
            dictionary_string(&helper_information, unsafe { kSecCodeInfoTeamIdentifier })?;
        let flags = dictionary_number(&helper_information, unsafe { kSecCodeInfoFlags })?;
        if helper_team != team_id || flags & CS_RUNTIME == 0 {
            return Err(());
        }
        Ok(())
    }
}

pub fn validate_bundled_helper(
    path: &std::path::Path,
    expected_identifier: &str,
) -> Result<(), ()> {
    #[cfg(target_os = "macos")]
    {
        macos::validate(path, expected_identifier)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, expected_identifier);
        Err(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_id_is_well_formed_accepts_ten_uppercase_alphanumeric_chars() {
        assert!(team_id_is_well_formed("ABCDE12345"));
        assert!(team_id_is_well_formed("0000000000"));
        assert!(team_id_is_well_formed("AAAAAAAAAA"));
    }

    #[test]
    fn team_id_is_well_formed_rejects_wrong_length() {
        assert!(!team_id_is_well_formed(""));
        assert!(!team_id_is_well_formed("ABCDE1234"));
        assert!(!team_id_is_well_formed("ABCDE123456"));
    }

    #[test]
    fn team_id_is_well_formed_rejects_lowercase_or_non_alphanumeric() {
        assert!(!team_id_is_well_formed("abcde12345"));
        assert!(!team_id_is_well_formed("ABCDE-2345"));
        assert!(!team_id_is_well_formed("ABCDE 2345"));
        assert!(!team_id_is_well_formed("ABCDE12345 "));
    }

    #[test]
    fn requirement_text_embeds_identifier_and_team_id() {
        let text = requirement_text("com.murmur.helper", "ABCDE12345");
        assert_eq!(
            text,
            "identifier \"com.murmur.helper\" and anchor apple generic and certificate leaf[subject.OU] = \"ABCDE12345\""
        );
    }
}
