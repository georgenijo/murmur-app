//! Native QA faults for microphone states a Mac cannot reach on demand.
//!
//! Every selector is compiled to a no-op unless the crate is built with
//! `native-host-qa`, and even then only the isolated QA identity honours it, so
//! neither a production build nor a developer build of another identity can be
//! affected. A fault replaces an outcome at the service boundary; the host
//! adapters, projections and native views that follow it are the real ones.

/// The only identity that honours a fault, matching the other QA fixtures.
#[cfg(feature = "native-host-qa")]
const QA_IDENTITY: &str = "com.localdictation.gpui-host-qa110";
/// Names exactly one fault per launch.
#[cfg(feature = "native-host-qa")]
const SELECTOR: &str = "MURMUR_QA_MICROPHONE_FAULT";

#[cfg_attr(not(feature = "native-host-qa"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fault {
    /// Voice detection rejects a sensitivity update.
    VadUnavailable,
    /// The Smart Auto status service cannot answer.
    SmartAutoUnavailable,
    /// The startup diagnostic stalls before it claims the run.
    StartupDelay,
    /// The startup diagnostic reports a cycle that fell back to CPAL.
    StartupFallback,
    /// The startup diagnostic reports a cycle in which every backend failed.
    StartupFailure,
}

#[cfg_attr(not(feature = "native-host-qa"), allow(dead_code))]
pub(crate) fn parse(value: &str) -> Option<Fault> {
    Some(match value {
        "vad-unavailable" => Fault::VadUnavailable,
        "smart-auto-unavailable" => Fault::SmartAutoUnavailable,
        "startup-delay" => Fault::StartupDelay,
        "startup-fallback" => Fault::StartupFallback,
        "startup-failure" => Fault::StartupFailure,
        _ => return None,
    })
}

#[cfg(feature = "native-host-qa")]
pub(crate) fn selected() -> Option<Fault> {
    if crate::identity::identifier() != QA_IDENTITY {
        return None;
    }
    parse(std::env::var(SELECTOR).ok()?.as_str())
}
#[cfg(not(feature = "native-host-qa"))]
pub(crate) const fn selected() -> Option<Fault> {
    None
}

/// Fails the command a fault stands in for.
pub(crate) fn refuse(fault: Fault) -> Result<(), String> {
    if selected() == Some(fault) {
        Err("Unavailable (native QA fault)".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faults_are_named_and_unknown_names_select_nothing() {
        assert_eq!(parse("vad-unavailable"), Some(Fault::VadUnavailable));
        assert_eq!(
            parse("smart-auto-unavailable"),
            Some(Fault::SmartAutoUnavailable)
        );
        assert_eq!(parse("startup-delay"), Some(Fault::StartupDelay));
        assert_eq!(parse("startup-fallback"), Some(Fault::StartupFallback));
        assert_eq!(parse("startup-failure"), Some(Fault::StartupFailure));
        assert_eq!(parse("anything-else"), None);
    }

    #[cfg(not(feature = "native-host-qa"))]
    #[test]
    fn production_builds_never_fault() {
        assert_eq!(selected(), None);
        assert!(refuse(Fault::VadUnavailable).is_ok());
    }

    /// Outside the QA identity the selector is ignored even when set.
    #[cfg(feature = "native-host-qa")]
    #[test]
    fn only_the_qa_identity_honours_the_selector() {
        assert_ne!(crate::identity::identifier(), QA_IDENTITY);
        assert_eq!(selected(), None);
    }
}
