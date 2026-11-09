//! One-shot synthetic outcomes for native QA of failure presentation.
//!
//! Some outcomes cannot be reached by a human on a healthy machine, such as a
//! delivery retry that finds nothing to paste right after a real dictation.
//! A `native-host-qa` build started with `--native-host-fault <name>[,<name>]`
//! under the isolated QA identity and test home arms each named fault once. The
//! next production command result for that spot is replaced; the adapter, model
//! and view that present it are the real ones. `Faults::arm` receives the
//! caller's permission decision, so every other build, identity and launch
//! arms nothing and `take` is always false.
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The overlay's delivery retry finds nothing to paste.
    OverlayRetryEmpty,
    /// The overlay's delivery retry command fails.
    OverlayRetryError,
    /// The overlay's Smart Auto status command fails.
    SmartAutoStatusError,
}

impl Fault {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "overlay-retry-empty" => Self::OverlayRetryEmpty,
            "overlay-retry-error" => Self::OverlayRetryError,
            "smart-auto-status-error" => Self::SmartAutoStatusError,
            _ => return None,
        })
    }
}

#[derive(Default)]
pub struct Faults(Mutex<Vec<Fault>>);

impl Faults {
    /// `requested` is the raw flag value. Nothing is armed unless `permitted`.
    /// Unknown names are ignored.
    pub fn arm(requested: Option<&str>, permitted: bool) -> Self {
        let armed = requested
            .filter(|_| permitted)
            .map(|names| {
                names
                    .split(',')
                    .filter_map(|n| Fault::parse(n.trim()))
                    .collect()
            })
            .unwrap_or_default();
        Self(Mutex::new(armed))
    }

    /// True once per armed fault: the first caller consumes it.
    pub fn take(&self, fault: Fault) -> bool {
        let mut armed = self.0.lock().unwrap_or_else(|p| p.into_inner());
        match armed.iter().position(|f| *f == fault) {
            Some(index) => {
                armed.remove(index);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_armed_fault_fires_once_and_only_for_its_own_spot() {
        let faults = Faults::arm(Some("overlay-retry-empty, smart-auto-status-error"), true);
        assert!(!faults.take(Fault::OverlayRetryError));
        assert!(faults.take(Fault::OverlayRetryEmpty));
        assert!(!faults.take(Fault::OverlayRetryEmpty), "one-shot");
        assert!(faults.take(Fault::SmartAutoStatusError));
        assert!(!faults.take(Fault::SmartAutoStatusError));
    }

    #[test]
    fn nothing_is_armed_without_permission_a_flag_or_a_known_name() {
        for faults in [
            Faults::arm(Some("overlay-retry-empty"), false),
            Faults::arm(None, true),
            Faults::arm(Some("mystery,,"), true),
            Faults::default(),
        ] {
            for fault in [
                Fault::OverlayRetryEmpty,
                Fault::OverlayRetryError,
                Fault::SmartAutoStatusError,
            ] {
                assert!(!faults.take(fault));
            }
        }
    }
}
