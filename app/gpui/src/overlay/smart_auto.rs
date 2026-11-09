//! Structured Smart Auto presentation for the overlay. The host projects the
//! production `get_smart_auto_microphone_status` result into one of the shipped
//! overlay's three states plus the deadline at which that claim stops being
//! true, so the cue expires on its own instead of outliving the service's
//! `validForMs` / `remainingMs` / `retryAfterMs`. Device names come from the
//! user's own inventory; ids never reach this type.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SmartAutoState {
    Ready {
        device: String,
        reason: &'static str,
    },
    Probing {
        device: String,
        phase: &'static str,
    },
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmartAuto {
    pub state: SmartAutoState,
    /// Overlay-clock millisecond at which this state stops being current, or
    /// `None` when the service gave no expiry (a fixed pin, no cooldown).
    pub until_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    Ready,
    Probing,
    Blocked,
}

const BLOCKED: &str = "Open settings to retry, verify signal, or pin an input.";

impl SmartAuto {
    /// The shipped "unavailable" state: blocked, no cooldown.
    pub fn unavailable() -> Self {
        Self {
            state: SmartAutoState::Blocked,
            until_ms: None,
        }
    }
    /// Whether the service's claim still holds at `now`.
    pub fn current(&self, now: u64) -> bool {
        self.until_ms.is_none_or(|until| now < until)
    }
    pub fn cue(&self, now: u64) -> Option<Cue> {
        self.current(now).then_some(match self.state {
            SmartAutoState::Ready { .. } => Cue::Ready,
            SmartAutoState::Probing { .. } => Cue::Probing,
            SmartAutoState::Blocked => Cue::Blocked,
        })
    }
    /// Whole seconds of cooldown left, never below one while it is running.
    fn cooldown_seconds(&self, now: u64) -> Option<u64> {
        let until = self
            .until_ms
            .filter(|_| self.state == SmartAutoState::Blocked)?;
        Some((until.saturating_sub(now).div_ceil(1000)).max(1))
    }
    /// The short text of the shipped tooltip and the expanded detail line.
    pub fn title(&self, now: u64) -> Option<String> {
        self.current(now).then(|| match &self.state {
            SmartAutoState::Ready { device, reason } => format!("Smart Auto: {device}. {reason}."),
            SmartAutoState::Probing { device, phase } => {
                format!("Smart Auto checking {device}: {phase}")
            }
            SmartAutoState::Blocked if self.until_ms.is_some() => {
                "Smart Auto blocked. Waiting for cooldown.".into()
            }
            SmartAutoState::Blocked => format!("Smart Auto blocked. {BLOCKED}"),
        })
    }
    /// The shipped accessible explanation of the cue.
    pub fn description(&self, now: u64) -> Option<String> {
        self.current(now).then(|| match &self.state {
            SmartAutoState::Ready { device, reason } => {
                format!("Smart Auto next capture ready with {device}. {reason}.")
            }
            SmartAutoState::Probing { device, phase } => format!(
                "Smart Auto background check for {device}: {phase}. Audio is not transcribed or saved."
            ),
            SmartAutoState::Blocked => {
                let base = format!("Smart Auto next capture blocked. {BLOCKED}");
                match self.cooldown_seconds(now) {
                    Some(seconds) => format!("{base} Cooldown: about {seconds} seconds remaining."),
                    None => base,
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(until_ms: Option<u64>) -> SmartAuto {
        SmartAuto {
            state: SmartAutoState::Ready {
                device: "Synthetic Microphone".into(),
                reason: "your preferred included microphone",
            },
            until_ms,
        }
    }

    #[test]
    fn each_state_has_its_own_cue_title_and_accessible_description() {
        let probing = SmartAuto {
            state: SmartAutoState::Probing {
                device: "Synthetic Microphone".into(),
                phase: "checking signal",
            },
            until_ms: Some(9_000),
        };
        let blocked = SmartAuto::unavailable();
        assert_eq!(ready(None).cue(0), Some(Cue::Ready));
        assert_eq!(probing.cue(0), Some(Cue::Probing));
        assert_eq!(blocked.cue(0), Some(Cue::Blocked));
        assert_eq!(
            ready(None).title(0).unwrap(),
            "Smart Auto: Synthetic Microphone. your preferred included microphone."
        );
        assert_eq!(
            ready(None).description(0).unwrap(),
            "Smart Auto next capture ready with Synthetic Microphone. your preferred included microphone."
        );
        assert_eq!(
            probing.title(0).unwrap(),
            "Smart Auto checking Synthetic Microphone: checking signal"
        );
        assert_eq!(
            probing.description(0).unwrap(),
            "Smart Auto background check for Synthetic Microphone: checking signal. Audio is not transcribed or saved."
        );
        assert_eq!(
            blocked.title(0).unwrap(),
            "Smart Auto blocked. Open settings to retry, verify signal, or pin an input."
        );
        assert_eq!(
            blocked.description(0).unwrap(),
            "Smart Auto next capture blocked. Open settings to retry, verify signal, or pin an input."
        );
    }

    #[test]
    fn a_cooldown_counts_down_in_whole_seconds_and_never_reads_zero() {
        let cooling = SmartAuto {
            state: SmartAutoState::Blocked,
            until_ms: Some(45_000),
        };
        assert_eq!(
            cooling.title(0).unwrap(),
            "Smart Auto blocked. Waiting for cooldown."
        );
        for (now, seconds) in [(0, 45), (1, 45), (30_000, 15), (44_001, 1), (44_999, 1)] {
            assert_eq!(
                cooling.description(now).unwrap(),
                format!(
                    "Smart Auto next capture blocked. Open settings to retry, verify signal, or pin an input. Cooldown: about {seconds} seconds remaining."
                ),
                "{now}"
            );
        }
    }

    #[test]
    fn a_state_with_a_deadline_stops_claiming_anything_when_it_passes() {
        let state = ready(Some(30_000));
        assert!(state.current(29_999) && state.cue(29_999).is_some());
        for now in [30_000, 90_000] {
            assert!(!state.current(now));
            assert_eq!(state.cue(now), None);
            assert_eq!(state.title(now), None);
            assert_eq!(state.description(now), None);
        }
        assert!(ready(None).current(u64::MAX), "no deadline never expires");
    }
}
