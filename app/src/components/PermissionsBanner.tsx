import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  checkMicrophonePermissionStatus,
  resetAccessibilityPermission,
  resetMicrophonePermission,
  type MicPermissionStatus,
} from '../lib/dictation';

interface PermissionStatus {
  microphone: MicPermissionStatus;
  accessibility: 'unknown' | 'granted' | 'denied';
}

/**
 * Whether a microphone status should render as a hard "denied" banner. Only a
 * genuine TCC denial (or restriction) blocks recording; "notDetermined" (no TCC
 * entry yet, common after a rebuild/move) and "unknown" (a transient probe
 * glitch) must NOT false-negative as denied (issue #190).
 */
function isMicHardDenied(status: MicPermissionStatus): boolean {
  return status === 'denied';
}

export function PermissionsBanner() {
  const [permissions, setPermissions] = useState<PermissionStatus>({
    microphone: 'unknown',
    accessibility: 'unknown',
  });
  const [dismissed, setDismissed] = useState(false);
  const [checking, setChecking] = useState(true);
  const [resetError, setResetError] = useState<string | null>(null);
  const [micResetError, setMicResetError] = useState<string | null>(null);

  const checkPermissions = useCallback(async () => {
    setChecking(true);
    try {
      // Check accessibility permission via Tauri command
      const hasAccessibility = await invoke<boolean>('check_accessibility_permission');

      // Check microphone via native TCC status query (issue #177).
      // Must NOT use getUserMedia here: opening the mic spins up voice-processing
      // I/O, which ducks all other system audio on every window focus.
      //
      // Use the 4-state status (not the bool probe) so a transient
      // "notDetermined"/"unknown" never collapses to a hard "denied" banner
      // after a dev rebuild or app move (issue #190).
      let micStatus: MicPermissionStatus = 'unknown';
      try {
        micStatus = await checkMicrophonePermissionStatus();
      } catch {
        micStatus = 'unknown';
      }

      setPermissions({
        microphone: micStatus,
        accessibility: hasAccessibility ? 'granted' : 'denied',
      });
    } catch (error) {
      console.error('Failed to check permissions:', error);
    } finally {
      setChecking(false);
    }
  }, []);

  useEffect(() => {
    checkPermissions();

    // Re-check when window gains focus (user might have granted permission)
    window.addEventListener('focus', checkPermissions);
    return () => window.removeEventListener('focus', checkPermissions);
  }, [checkPermissions]);

  const handleOpenAccessibility = async () => {
    await invoke('request_accessibility_permission');
  };

  const handleOpenMicrophone = async () => {
    await invoke('request_microphone_permission');
  };

  const handleResetAccessibility = async () => {
    setResetError(null);
    try {
      await resetAccessibilityPermission();
    } catch (error) {
      console.error('Failed to reset accessibility permission:', error);
      setResetError(
        typeof error === 'string'
          ? error
          : "Couldn't reset the Accessibility entry. Check the logs for details.",
      );
    } finally {
      checkPermissions();
    }
  };

  const handleResetMicrophone = async () => {
    setMicResetError(null);
    try {
      await resetMicrophonePermission();
    } catch (error) {
      console.error('Failed to reset microphone permission:', error);
      setMicResetError(
        typeof error === 'string'
          ? error
          : "Couldn't reset the Microphone entry. Check the logs for details.",
      );
    } finally {
      checkPermissions();
    }
  };

  const micDenied = isMicHardDenied(permissions.microphone);
  // Only a genuine denial blocks recording; treat notDetermined/unknown as "fine
  // for now" so the banner doesn't surface a false-negative (issue #190).
  const micOk = !micDenied;
  const allGranted = micOk && permissions.accessibility === 'granted';

  if (dismissed || allGranted || checking) {
    return null;
  }

  const needsAccessibility = permissions.accessibility !== 'granted';
  const message = micDenied && needsAccessibility
    ? 'Microphone and Accessibility access are needed.'
    : micDenied
      ? 'Microphone access was revoked.'
      : 'Accessibility access is needed for auto-paste.';

  return (
    <div
      role="region"
      aria-label="Permission warning"
      className="relative mx-3.5 mb-1.5 flex shrink-0 items-center gap-2 rounded-lg bg-warning/10 px-3 py-2 text-xs text-warning"
    >
      <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-warning" />
      <span className="min-w-0 flex-1 truncate">{message}</span>
      <button
        type="button"
        onClick={micDenied ? handleOpenMicrophone : handleOpenAccessibility}
        className="shrink-0 font-semibold underline underline-offset-2 hover:no-underline"
      >
        Open System Settings
      </button>
      <details className="relative shrink-0">
        <summary className="cursor-pointer list-none rounded px-1 py-0.5 font-semibold hover:bg-warning/10 focus:outline-none focus-visible:ring-2 focus-visible:ring-warning">
          More
        </summary>
        <div className="absolute right-0 top-6 z-40 w-72 space-y-2 rounded-xl border border-outline-variant/25 bg-surface-container-lowest p-3 text-on-surface shadow-2xl">
          <button
            type="button"
            onClick={checkPermissions}
            className="text-xs font-semibold hover:underline"
          >
            Re-check permissions
          </button>
          {micDenied && (
            <div className="space-y-1 border-t border-outline-variant/20 pt-2">
              <button
                type="button"
                onClick={handleResetMicrophone}
                className="text-xs font-semibold underline hover:no-underline"
              >
                Reset Microphone permission
              </button>
              <p className="text-[10px] leading-relaxed text-on-surface-variant">
                Clears Murmur's stale Microphone entry, then opens System Settings.
                macOS will re-prompt the next time you record.
              </p>
              {micResetError && (
                <p
                  role="alert"
                  className="rounded-md border border-error bg-surface-container-lowest px-2 py-1 text-xs text-error"
                >
                  {micResetError}
                </p>
              )}
            </div>
          )}
          {needsAccessibility && (
            <div className="space-y-1 border-t border-outline-variant/20 pt-2">
              <button
                type="button"
                onClick={handleResetAccessibility}
                className="text-xs font-semibold underline hover:no-underline"
              >
                Reset Accessibility permission
              </button>
              <p className="text-[10px] leading-relaxed text-on-surface-variant">
                Clears Murmur's stale Accessibility entry, then opens System Settings.
                You'll still need to turn Murmur back on manually.
              </p>
              {resetError && (
                <p
                  role="alert"
                  className="rounded-md border border-error bg-surface-container-lowest px-2 py-1 text-xs text-error"
                >
                  {resetError}
                </p>
              )}
            </div>
          )}
        </div>
      </details>
      <button
        type="button"
        onClick={() => setDismissed(true)}
        className="grid h-5 w-5 shrink-0 place-items-center rounded text-warning transition-colors hover:bg-warning/10"
        aria-label="Dismiss"
      >
        <svg className="h-3.5 w-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>
    </div>
  );
}
