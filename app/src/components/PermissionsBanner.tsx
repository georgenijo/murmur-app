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

  return (
    <div className="bg-primary/10 px-4 py-3">
      <div className="flex items-start justify-between gap-3">
        <div className="flex-1">
          <h3 className="text-sm font-medium text-on-surface">
            Permissions Required
          </h3>
          <div className="mt-2 space-y-2">
            {/* Microphone Permission */}
            <div className="flex items-center gap-2">
              <span className={`w-2 h-2 rounded-full ${
                micOk ? 'bg-success' : 'bg-error'
              }`} />
              <span className="text-sm text-on-surface">
                Microphone: {micOk ? 'Granted' : 'Required for recording'}
              </span>
              {micDenied && (
                <button
                  onClick={handleOpenMicrophone}
                  className="text-xs text-on-surface underline hover:no-underline"
                >
                  Open Settings
                </button>
              )}
            </div>

            {/* Microphone troubleshooting: reset a stale TCC entry */}
            {micDenied && (
              <div className="ml-4 space-y-1">
                <button
                  onClick={handleResetMicrophone}
                  className="text-xs text-on-surface underline hover:no-underline"
                >
                  Still not working? Reset &amp; Open Settings
                </button>
                <p className="text-xs text-on-surface">
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

            {/* Accessibility Permission */}
            <div className="flex items-center gap-2">
              <span className={`w-2 h-2 rounded-full ${
                permissions.accessibility === 'granted'
                  ? 'bg-success'
                  : 'bg-error'
              }`} />
              <span className="text-sm text-on-surface">
                Accessibility: {permissions.accessibility === 'granted' ? 'Granted' : 'Required for text pasting'}
              </span>
              {permissions.accessibility !== 'granted' && (
                <button
                  onClick={handleOpenAccessibility}
                  className="text-xs text-on-surface underline hover:no-underline"
                >
                  Open Settings
                </button>
              )}
            </div>

            {/* Accessibility troubleshooting: reset a stale TCC entry */}
            {permissions.accessibility !== 'granted' && (
              <div className="ml-4 space-y-1">
                <button
                  onClick={handleResetAccessibility}
                  className="text-xs text-on-surface underline hover:no-underline"
                >
                  Still not working? Reset &amp; Open Settings
                </button>
                <p className="text-xs text-on-surface">
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

          <button
            onClick={checkPermissions}
            className="mt-2 text-xs text-on-surface hover:underline"
          >
            Re-check permissions
          </button>
        </div>

        <button
          onClick={() => setDismissed(true)}
          className="rounded-md p-0.5 text-on-surface transition-colors hover:bg-surface-container-lowest"
          aria-label="Dismiss"
        >
          <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>
    </div>
  );
}
