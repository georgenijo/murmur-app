//! Crash-recoverable authority transfer. Persist each checkpoint before its
//! following effect. A failed effect never advances the checkpoint.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Checkpoint {
    #[default]
    Unseen,
    /// A verified, enabled old agent AND an enabled saved choice were observed.
    MigrateEnabled,
    /// Old authority was disabled and its verified plist retired.
    OldRetired,
    /// Persisted before the one automatic registration attempt. On restart a
    /// disabled service may represent a later user choice, so never re-enable.
    RegistrationAttempted,
    Complete,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Registration {
    Disabled,
    Enabled,
    RequiresApproval,
    Unavailable,
}
#[derive(Clone, Copy, Debug)]
pub struct Observation {
    pub preference: Option<bool>,
    pub legacy_verified: bool,
    pub legacy_enabled: bool,
    pub service: Registration,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Persist(Checkpoint),
    RetireLegacy,
    Register,
    Unregister,
    Done,
    Unavailable,
}
/// Reads never register an app. Automatic migration requires proof that the
/// old authority and saved preference were both on. Missing/ambiguous evidence
/// fails closed. The saved checkpoint is the only proof after old retirement.
pub fn next(checkpoint: Checkpoint, o: Observation) -> Effect {
    if o.service == Registration::Unavailable {
        return Effect::Unavailable;
    }
    if o.preference == Some(false) {
        if o.legacy_verified && o.legacy_enabled {
            return Effect::RetireLegacy;
        }
        if matches!(
            o.service,
            Registration::Enabled | Registration::RequiresApproval
        ) {
            return Effect::Unregister;
        }
        return if checkpoint == Checkpoint::Complete {
            Effect::Done
        } else {
            Effect::Persist(Checkpoint::Complete)
        };
    }
    if checkpoint == Checkpoint::Complete
        && o.legacy_verified
        && o.legacy_enabled
        && matches!(
            o.service,
            Registration::Enabled | Registration::RequiresApproval
        )
    {
        // A downgrade can recreate the legacy plist after migration. Preserve
        // the existing SM choice while removing the duplicate old authority.
        return Effect::RetireLegacy;
    }
    match checkpoint {
        Checkpoint::Unseen => {
            if o.preference == Some(true) && o.legacy_verified && o.legacy_enabled {
                Effect::Persist(Checkpoint::MigrateEnabled)
            } else {
                Effect::Persist(Checkpoint::Complete)
            }
        }
        Checkpoint::MigrateEnabled => {
            if o.legacy_verified {
                Effect::RetireLegacy
            } else if o.legacy_enabled {
                // A changed/unverifiable old job is not ours to retire.
                Effect::Unavailable
            } else {
                Effect::Persist(Checkpoint::OldRetired)
            }
        }
        Checkpoint::OldRetired => {
            if o.legacy_verified || o.legacy_enabled {
                Effect::RetireLegacy
            } else if o.service == Registration::Disabled {
                Effect::Register
            } else {
                // Approval is a user decision. Do not repeatedly re-register.
                Effect::Persist(Checkpoint::Complete)
            }
        }
        Checkpoint::RegistrationAttempted => Effect::Persist(Checkpoint::Complete),
        Checkpoint::Complete => Effect::Done,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn obs(preference: Option<bool>, legacy: bool, service: Registration) -> Observation {
        Observation {
            preference,
            legacy_verified: legacy,
            legacy_enabled: legacy,
            service,
        }
    }
    #[test]
    fn off_missing_or_disabled_legacy_never_enables() {
        for preference in [None, Some(false), Some(true)] {
            for legacy in [false, true] {
                let o = obs(preference, legacy, Registration::Disabled);
                if preference != Some(true) || !legacy {
                    assert_ne!(next(Checkpoint::Unseen, o), Effect::Register);
                    assert_ne!(
                        next(Checkpoint::Unseen, o),
                        Effect::Persist(Checkpoint::MigrateEnabled)
                    );
                }
            }
        }
    }
    #[test]
    fn enabled_migration_orders_durable_intent_retirement_and_registration() {
        let on = obs(Some(true), true, Registration::Disabled);
        assert_eq!(
            next(Checkpoint::Unseen, on),
            Effect::Persist(Checkpoint::MigrateEnabled)
        );
        assert_eq!(next(Checkpoint::MigrateEnabled, on), Effect::RetireLegacy);
        let retired = obs(Some(true), false, Registration::Disabled);
        assert_eq!(
            next(Checkpoint::MigrateEnabled, retired),
            Effect::Persist(Checkpoint::OldRetired)
        );
        assert_eq!(next(Checkpoint::OldRetired, retired), Effect::Register);
        let approval = obs(Some(true), false, Registration::RequiresApproval);
        assert_eq!(
            next(Checkpoint::OldRetired, approval),
            Effect::Persist(Checkpoint::Complete)
        );
        assert_eq!(next(Checkpoint::Complete, approval), Effect::Done);
    }
    #[test]
    fn user_off_wins_at_every_crash_checkpoint() {
        for checkpoint in [
            Checkpoint::Unseen,
            Checkpoint::MigrateEnabled,
            Checkpoint::OldRetired,
            Checkpoint::RegistrationAttempted,
            Checkpoint::Complete,
        ] {
            assert_eq!(
                next(checkpoint, obs(Some(false), true, Registration::Enabled)),
                Effect::RetireLegacy
            );
            assert_eq!(
                next(checkpoint, obs(Some(false), false, Registration::Enabled)),
                Effect::Unregister
            );
            assert_ne!(
                next(checkpoint, obs(Some(false), false, Registration::Disabled)),
                Effect::Register
            );
        }
    }
    #[test]
    fn unknown_registration_outcome_never_overrides_a_later_os_off_choice() {
        let off = obs(Some(true), false, Registration::Disabled);
        assert_eq!(
            next(Checkpoint::RegistrationAttempted, off),
            Effect::Persist(Checkpoint::Complete)
        );
        assert_eq!(next(Checkpoint::Complete, off), Effect::Done);
    }
    #[test]
    fn downgrade_cannot_leave_duplicate_enabled_authorities() {
        assert_eq!(
            next(
                Checkpoint::Complete,
                obs(Some(true), true, Registration::Enabled)
            ),
            Effect::RetireLegacy
        );
        assert_eq!(
            next(
                Checkpoint::Complete,
                obs(Some(true), false, Registration::Enabled)
            ),
            Effect::Done
        );
    }
    #[test]
    fn no_registration_while_old_authority_remains_or_status_unknown() {
        for service in [
            Registration::Disabled,
            Registration::Enabled,
            Registration::RequiresApproval,
            Registration::Unavailable,
        ] {
            assert_ne!(
                next(Checkpoint::OldRetired, obs(Some(true), true, service)),
                Effect::Register
            );
        }
        let changed = Observation {
            preference: Some(true),
            legacy_verified: false,
            legacy_enabled: true,
            service: Registration::Disabled,
        };
        assert_eq!(
            next(Checkpoint::MigrateEnabled, changed),
            Effect::Unavailable
        );
    }
}
