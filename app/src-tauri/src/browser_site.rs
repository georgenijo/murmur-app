use crate::frontmost::FrontmostAppIdentity;
use crate::state::BrowserSiteRule;
use crate::MutexExt;
use serde::Serialize;

pub const ALLOWED_BROWSER_BUNDLE_IDS: [&str; 6] = [
    "com.apple.Safari",
    "com.google.Chrome",
    "com.microsoft.edgemac",
    "com.brave.Browser",
    "company.thebrowser.Browser",
    "org.chromium.Chromium",
];

#[derive(Clone, PartialEq, Eq)]
pub struct BrowserSiteIdentity {
    pub browser_bundle_id: String,
    pub host: String,
}

pub fn allowed_browser(bundle_id: &str) -> bool {
    ALLOWED_BROWSER_BUNDLE_IDS.contains(&bundle_id)
}

pub fn normalize_host(value: &str) -> Option<String> {
    let host = value.trim().to_ascii_lowercase();
    let host = host.strip_suffix('.').unwrap_or(&host);
    if host.is_empty() || host.len() > 253 || host.contains(':') {
        return None;
    }
    if host == "localhost" {
        return Some(host.to_string());
    }
    let labels = host.split('.').collect::<Vec<_>>();
    if labels.len() < 2
        || labels.iter().any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return None;
    }
    Some(host.to_string())
}

pub fn host_from_document_url(value: &str) -> Option<String> {
    let value = value.trim();
    let remainder = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))?;
    let authority = remainder.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    normalize_host(authority.split(':').next()?)
}

pub fn matching_rule<'a>(
    rules: &'a [BrowserSiteRule],
    identity: &BrowserSiteIdentity,
) -> Option<&'a BrowserSiteRule> {
    rules.iter().find(|rule| {
        rule.enabled
            && rule.browser_bundle_id == identity.browser_bundle_id
            && rule.host == identity.host
    })
}

#[cfg(target_os = "macos")]
fn ax_document_url(process_id: i32) -> Option<String> {
    use core_foundation::base::{CFGetTypeID, TCFType};
    use core_foundation::string::{CFString, CFStringGetTypeID, CFStringRef};
    use core_foundation::url::{CFURLGetTypeID, CFURLRef, CFURL};
    use std::ffi::{c_char, c_void, CString};

    type AXUIElementRef = *const c_void;
    type CFTypeRef = *const c_void;
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFTypeRef,
            value: *mut CFTypeRef,
        ) -> i32;
        fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> i32;
        fn CFStringCreateWithCString(
            allocator: CFTypeRef,
            string: *const c_char,
            encoding: u32,
        ) -> CFTypeRef;
        fn CFRelease(value: CFTypeRef);
    }
    struct Guard(CFTypeRef);
    impl Drop for Guard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }
    fn attribute(name: &str) -> Option<Guard> {
        let name = CString::new(name).ok()?;
        let value =
            unsafe { CFStringCreateWithCString(std::ptr::null(), name.as_ptr(), 0x0800_0100) };
        (!value.is_null()).then_some(Guard(value))
    }
    fn copy(element: AXUIElementRef, name: &str) -> Option<Guard> {
        let attribute = attribute(name)?;
        let mut value = std::ptr::null();
        let status = unsafe { AXUIElementCopyAttributeValue(element, attribute.0, &mut value) };
        (status == 0 && !value.is_null()).then_some(Guard(value))
    }

    let application = unsafe { AXUIElementCreateApplication(process_id) };
    if application.is_null() {
        return None;
    }
    let application = Guard(application);
    if unsafe { AXUIElementSetMessagingTimeout(application.0, 0.025) } != 0 {
        return None;
    }
    let window = copy(application.0, "AXFocusedWindow")?;
    let document = copy(window.0, "AXDocument")?;
    let type_id = unsafe { CFGetTypeID(document.0) };
    if type_id == unsafe { CFStringGetTypeID() } {
        return Some(
            unsafe { CFString::wrap_under_get_rule(document.0 as CFStringRef) }.to_string(),
        );
    }
    if type_id == unsafe { CFURLGetTypeID() } {
        return Some(
            unsafe { CFURL::wrap_under_get_rule(document.0 as CFURLRef) }
                .get_string()
                .to_string(),
        );
    }
    None
}

#[cfg(target_os = "macos")]
pub fn query_for_identity(expected: &FrontmostAppIdentity) -> Option<BrowserSiteIdentity> {
    use objc2_app_kit::NSWorkspace;
    let expected_bundle = expected.bundle_id.as_deref()?;
    if !allowed_browser(expected_bundle) || expected.process_id.is_none() {
        return None;
    }
    let application = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let actual = FrontmostAppIdentity {
        bundle_id: application
            .bundleIdentifier()
            .map(|value| value.to_string()),
        process_id: Some(application.processIdentifier()),
    };
    if !crate::frontmost::query_identity_matches(expected, &actual) {
        return None;
    }
    let host = host_from_document_url(&ax_document_url(application.processIdentifier())?)?;
    Some(BrowserSiteIdentity {
        browser_bundle_id: expected_bundle.to_string(),
        host,
    })
}

#[cfg(not(target_os = "macos"))]
pub fn query_for_identity(_expected: &FrontmostAppIdentity) -> Option<BrowserSiteIdentity> {
    None
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSiteProbe {
    status: &'static str,
    browser_bundle_id: Option<String>,
    host: Option<String>,
}

#[tauri::command]
pub async fn probe_browser_site(
    state: tauri::State<'_, crate::State>,
) -> Result<BrowserSiteProbe, String> {
    if !state
        .app_state
        .dictation
        .lock_or_recover()
        .site_mode_lookup_enabled
    {
        return Ok(BrowserSiteProbe {
            status: "disabled",
            browser_bundle_id: None,
            host: None,
        });
    }
    let mut supported_browser = None;
    for _ in 0..50 {
        let expected = crate::frontmost::query_frontmost_app_identity();
        if expected.bundle_id.as_deref().is_some_and(allowed_browser) {
            supported_browser = expected.bundle_id.clone();
            if let Some(identity) = query_for_identity(&expected) {
                return Ok(BrowserSiteProbe {
                    status: "available",
                    browser_bundle_id: Some(identity.browser_bundle_id),
                    host: Some(identity.host),
                });
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    Ok(BrowserSiteProbe {
        status: if supported_browser.is_some() {
            "unavailable"
        } else {
            "unsupported_browser"
        },
        browser_bundle_id: supported_browser,
        host: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rule(browser: &str, host: &str, mode: &str, enabled: bool) -> BrowserSiteRule {
        BrowserSiteRule {
            id: format!("{browser}:{host}"),
            browser_bundle_id: browser.into(),
            host: host.into(),
            mode_id: mode.into(),
            enabled,
        }
    }
    #[test]
    fn document_urls_reduce_to_strict_exact_hosts() {
        assert_eq!(
            host_from_document_url("https://GitHub.com/org/repo?q=1"),
            Some("github.com".into())
        );
        assert_eq!(
            host_from_document_url("http://localhost:1420/path"),
            Some("localhost".into())
        );
        assert_eq!(host_from_document_url("file:///tmp/private"), None);
        assert_eq!(host_from_document_url("https://user@example.com"), None);
    }
    #[test]
    fn rules_match_browser_and_host_exactly_in_saved_order() {
        let rules = vec![
            rule("com.apple.Safari", "github.com", "disabled", false),
            rule("com.apple.Safari", "github.com", "technical", true),
            rule("com.google.Chrome", "github.com", "chrome", true),
        ];
        let site = BrowserSiteIdentity {
            browser_bundle_id: "com.apple.Safari".into(),
            host: "github.com".into(),
        };
        assert_eq!(
            matching_rule(&rules, &site).map(|rule| rule.mode_id.as_str()),
            Some("technical")
        );
        assert!(matching_rule(
            &rules,
            &BrowserSiteIdentity {
                host: "www.github.com".into(),
                ..site
            }
        )
        .is_none());
    }
}
