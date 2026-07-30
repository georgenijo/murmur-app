//! Runtime validation for bundled helper code identities.
//!
//! The release path validates the exact nested executable before spawn using
//! Security.framework. Path pinning alone is insufficient: the helper must be
//! valid Developer ID code, carry its fixed identifier, share the main app's
//! Team ID, and have the hardened-runtime flag.

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
        if team_id.len() != 10
            || !team_id
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        {
            return Err(());
        }

        let url = CFURL::from_path(path, false).ok_or(())?;
        let helper = SecStaticCode::from_path(&url, Flags::NONE).map_err(|_| ())?;
        let requirement_text = format!(
            "identifier \"{expected_identifier}\" and anchor apple generic and certificate leaf[subject.OU] = \"{team_id}\""
        );
        let requirement: SecRequirement = requirement_text.parse().map_err(|_| ())?;
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
