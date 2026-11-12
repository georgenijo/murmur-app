import type { AccessibilityRequestOutcome } from './dictation';

/**
 * What to tell the user after an Accessibility request or reset. Returns null
 * when the outcome needs no message (nothing was shown, or access is granted).
 * `owner` names the app macOS lists ("Murmur", or the terminal app in dev).
 */
export function accessibilityOutcomeMessage(
  outcome: AccessibilityRequestOutcome | null,
  owner = 'Murmur',
): string | null {
  switch (outcome) {
    case 'prompted':
      return `Choose Open System Settings in the macOS dialog, then turn on ${owner}. This screen updates automatically.`;
    case 'opened_settings':
      return `Turn on ${owner} in System Settings, then come back. This screen updates automatically.`;
    case 'restarting':
      return 'Murmur is restarting to finish the reset. Turn it on when macOS asks.';
    case 'restart_deferred':
      return `Murmur is busy, so it did not restart. Turn on ${owner} in System Settings. If it stays off, finish your recording or meeting and press the button again to refresh the old entry.`;
    default:
      return null;
  }
}

/** Failure text for a rejected permission command (Rust sends plain strings). */
export function permissionErrorMessage(error: unknown, fallback: string): string {
  return typeof error === 'string' && error.length > 0 ? error : fallback;
}

const RESUME_KEY = 'murmur.onboarding.resume';
/** How long after a permission relaunch the wizard still resumes where it was. */
const RESUME_WINDOW_MS = 3 * 60 * 1000;

/**
 * Remember which wizard step to return to after Murmur restarts for a permission
 * reset or grant. Best effort: storage may be unavailable.
 */
export function rememberOnboardingResume(step: string): void {
  try {
    localStorage.setItem(RESUME_KEY, JSON.stringify({ step, at: Date.now() }));
  } catch {
    // Without storage the wizard simply starts at Welcome again.
  }
}

/** The remembered step if it is recent and one of `validSteps`. Does not clear it. */
export function readOnboardingResume<T extends string>(validSteps: readonly T[]): T | null {
  try {
    const raw = localStorage.getItem(RESUME_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as { step?: unknown; at?: unknown };
    if (typeof parsed.step !== 'string' || typeof parsed.at !== 'number') return null;
    if (Date.now() - parsed.at > RESUME_WINDOW_MS || parsed.at > Date.now() + 1000) return null;
    return validSteps.find((step) => step === parsed.step) ?? null;
  } catch {
    return null;
  }
}

export function clearOnboardingResume(): void {
  try {
    localStorage.removeItem(RESUME_KEY);
  } catch {
    // Nothing to clear.
  }
}
