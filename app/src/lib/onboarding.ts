/**
 * First-launch onboarding completion flag.
 *
 * The setup assistant (OnboardingFlow) runs when this flag is absent. Existing
 * installs are grandfathered at mount: if the mic + accessibility permissions
 * and any model are already in place, App sets the flag silently so upgrades
 * never see the wizard. Stored in localStorage alongside settings,
 * stats, and history.
 */

const ONBOARDING_KEY = 'murmur_onboarding_complete';

export function isOnboardingComplete(): boolean {
  try {
    return localStorage.getItem(ONBOARDING_KEY) === 'true';
  } catch {
    // Storage unavailable: treat as complete so the app never hard-blocks on
    // the wizard; the permissions banner still catches missing grants.
    return true;
  }
}

export function markOnboardingComplete(): void {
  try {
    localStorage.setItem(ONBOARDING_KEY, 'true');
  } catch {
    // Non-fatal: the wizard will show again next launch.
  }
}

/** Clear the flag so the setup assistant runs again (Settings → Setup Assistant). */
export function resetOnboarding(): void {
  try {
    localStorage.removeItem(ONBOARDING_KEY);
  } catch {
    // Non-fatal.
  }
}
