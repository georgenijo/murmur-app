import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  checkAccessibilityRestartNeeded,
  checkMicrophonePermissionStatus,
  openMicrophoneSettings,
  requestAccessibilityPermission,
  resetAccessibilityPermission,
  resetMicrophonePermission,
  restartMurmurForPermission,
  type AccessibilityRequestOutcome,
  type MicPermissionStatus,
} from '../lib/dictation';
import {
  ACCESSIBILITY_RESTART_NOTICE,
  accessibilityOutcomeMessage,
  microphoneResetMessage,
  permissionErrorMessage,
} from '../lib/permissionOutcome';
import { openSystemAudioPreferences, type SystemAudioAccess, type SystemAudioPermissionState } from '../lib/meetings';

interface PermissionStatus {
  microphone: MicPermissionStatus;
  accessibility: 'unknown' | 'granted' | 'denied';
  /** Accessibility is on, but this running copy cannot use it until it restarts. */
  accessibilityRestartNeeded: boolean;
  systemAudio: SystemAudioPermissionState;
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

export function PermissionsBanner({ microphoneInHome = false }: { microphoneInHome?: boolean }) {
  const [permissions, setPermissions] = useState<PermissionStatus>({
    microphone: 'unknown',
    accessibility: 'unknown',
    accessibilityRestartNeeded: false,
    systemAudio: 'unknown',
  });
  const [dismissed, setDismissed] = useState(false);
  const [checking, setChecking] = useState(true);
  const [resetError, setResetError] = useState<string | null>(null);
  const [micResetError, setMicResetError] = useState<string | null>(null);
  // What the last Accessibility or Microphone action did, shown in the banner.
  const [actionNote, setActionNote] = useState<string | null>(null);
  // A restart is on its way. Never cleared or dismissed (the process is about to
  // end), so Murmur cannot restart without a visible reason.
  const [restartPending, setRestartPending] = useState<string | null>(null);
  const [moreOpen, setMoreOpen] = useState(false);

  const systemAudioRevision = useRef(0);
  // Permission state the last action note was about; the note is dropped when it changes.
  const stateSignature = useRef<string | null>(null);

  const checkPermissions = useCallback(async () => {
    const revision = systemAudioRevision.current;
    setChecking(true);
    try {
      // Check accessibility permission via Tauri command
      const [hasAccessibility, restartNeeded, systemAudio] = await Promise.all([
        invoke<boolean>('check_accessibility_permission'),
        checkAccessibilityRestartNeeded().catch(() => false),
        invoke<SystemAudioPermissionState>('get_system_audio_permission_status').catch(() => 'unknown' as const),
      ]);

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

      const signature = `${micStatus === 'denied'}|${hasAccessibility}|${restartNeeded}`;
      if (stateSignature.current !== null && stateSignature.current !== signature) setActionNote(null);
      stateSignature.current = signature;
      setPermissions((current) => ({
        microphone: micStatus,
        accessibility: hasAccessibility ? 'granted' : 'denied',
        accessibilityRestartNeeded: hasAccessibility && restartNeeded,
        systemAudio: revision === systemAudioRevision.current ? systemAudio : current.systemAudio,
      }));
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

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    listen<SystemAudioAccess>('system-audio-permission-changed', (event) => {
      ++systemAudioRevision.current;
      setActionNote(null);
      setPermissions((current) => ({ ...current, systemAudio: event.payload.permission }));
      if (event.payload.permission === 'denied') setDismissed(false);
    }).then((fn) => {
      if (disposed) fn(); else unlisten = fn;
    }).catch(() => {
      // Unit tests and non-Tauri previews do not expose the native event bridge.
    });
    return () => { disposed = true; unlisten?.(); };
  }, []);

  const noteAccessibilityOutcome = (outcome: AccessibilityRequestOutcome) => {
    const message = accessibilityOutcomeMessage(outcome);
    setActionNote(message);
    if (message && (outcome === 'restarting' || outcome === 'restart_pending')) setRestartPending(message);
  };

  const handleOpenAccessibility = async () => {
    setActionNote(null);
    try {
      noteAccessibilityOutcome(await requestAccessibilityPermission());
    } catch (error) {
      setActionNote(permissionErrorMessage(error, "Couldn't open the Accessibility request."));
    }
  };

  const handleRestartMurmur = async () => {
    setActionNote(null);
    try {
      await restartMurmurForPermission();
      const message = 'Murmur is restarting to finish turning on Accessibility.';
      setActionNote(message);
      setRestartPending(message);
    } catch (error) {
      setActionNote(permissionErrorMessage(error, "Couldn't restart Murmur."));
    }
  };

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const subscribe = (name: string, handler: () => void) => {
      listen(name, handler).then((fn) => {
        if (disposed) fn(); else unlisteners.push(fn);
      }).catch(() => {
        // Unit tests and non-Tauri previews do not expose the native event bridge.
      });
    };
    // A real Accessibility query was refused while trusted: show the notice now,
    // not at the next window focus.
    subscribe('accessibility-restart-needed', () => { void checkPermissions(); });
    // Whoever asked for it, a restart is scheduled: say so.
    subscribe('permission-relaunch-scheduled', () => {
      setRestartPending((current) => current ?? 'Murmur is restarting to finish a permission change.');
    });
    return () => { disposed = true; unlisteners.forEach((fn) => fn()); };
  }, [checkPermissions]);

  const handleOpenMicrophone = async () => {
    await openMicrophoneSettings();
  };

  const handleResetAccessibility = async () => {
    setResetError(null);
    try {
      noteAccessibilityOutcome(await resetAccessibilityPermission());
    } catch (error) {
      console.error('Failed to reset accessibility permission:', error);
      setResetError(
        permissionErrorMessage(error, "Couldn't reset the Accessibility entry. Check the logs for details."),
      );
    } finally {
      checkPermissions();
    }
  };

  const handleResetMicrophone = async () => {
    setMicResetError(null);
    try {
      const message = microphoneResetMessage(await resetMicrophonePermission());
      setActionNote(message);
      setRestartPending(message);
    } catch (error) {
      console.error('Failed to reset microphone permission:', error);
      setMicResetError(
        permissionErrorMessage(error, "Couldn't reset the Microphone entry. Check the logs for details."),
      );
    } finally {
      checkPermissions();
    }
  };

  const micDenied = isMicHardDenied(permissions.microphone);
  const systemAudioDenied = permissions.systemAudio === 'denied';
  // Only a genuine denial blocks recording; treat notDetermined/unknown as "fine
  // for now" so the banner doesn't surface a false-negative (issue #190).
  const micOk = !micDenied;
  const allGranted = micOk && permissions.accessibility === 'granted' && !systemAudioDenied;

  // Accessibility is on but this copy cannot use it yet: a notice, never an automatic restart.
  const restartNotice = permissions.accessibilityRestartNeeded && micOk && !systemAudioDenied;

  if (restartPending) {
    return (
      <div
        role="status"
        aria-label="Restart pending"
        className="dialog-banner relative mx-3.5 mb-1.5 flex shrink-0 items-center gap-2 border-warning/25 bg-[color-mix(in_srgb,var(--murmur-warning)_10%,transparent)] px-3 py-2 text-[length:var(--ui-font-caption)] text-warning"
      >
        <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-warning" />
        <span className="min-w-0 flex-1 truncate" title={restartPending}>{restartPending}</span>
      </div>
    );
  }

  if (dismissed || (allGranted && !restartNotice) || checking || (microphoneInHome && micDenied && permissions.accessibility === 'granted' && !systemAudioDenied)) {
    return null;
  }

  const needsAccessibility = permissions.accessibility !== 'granted';
  const message = restartNotice
    ? ACCESSIBILITY_RESTART_NOTICE
    : systemAudioDenied
    ? 'System Audio access was denied; Meeting capture cannot start.'
    : micDenied && needsAccessibility
    ? 'Microphone and Accessibility access are needed.'
    : micDenied
      ? 'Microphone access was revoked.'
      : 'Accessibility access is needed for auto-paste.';

  const shownMessage = actionNote ?? message;

  return (
    <div
      role="region"
      aria-label="Permission warning"
      className="dialog-banner relative mx-3.5 mb-1.5 flex shrink-0 items-center gap-2 border-warning/25 bg-[color-mix(in_srgb,var(--murmur-warning)_10%,transparent)] px-3 py-2 text-xs text-warning"
    >
      <span aria-hidden="true" className="h-1.5 w-1.5 shrink-0 rounded-full bg-warning" />
      <span className="min-w-0 flex-1 truncate" title={shownMessage}>{shownMessage}</span>
      <button
        type="button"
        onClick={() => void (restartNotice
          ? handleRestartMurmur()
          : systemAudioDenied
            ? openSystemAudioPreferences()
            : micDenied
              ? handleOpenMicrophone()
              : handleOpenAccessibility())}
        className="shrink-0 rounded-[var(--ui-radius-pill)] px-2.5 py-1 font-semibold transition-colors hover:bg-warning/10"
      >
        {restartNotice ? 'Restart Murmur' : 'Open System Settings…'}
      </button>
      <details
        open={moreOpen}
        onToggle={(event) => setMoreOpen(event.currentTarget.open)}
        className="relative shrink-0"
      >
        <summary className="cursor-pointer list-none rounded-[var(--ui-radius-control)] px-1.5 py-0.5 font-semibold hover:bg-warning/10 focus:outline-none focus-visible:ring-2 focus-visible:ring-warning">
          More
        </summary>
        <div className="dialog-popover absolute right-0 top-6 z-40 w-72 space-y-2 p-3 text-on-surface">
          <button
            type="button"
            onClick={checkPermissions}
            className="text-xs font-semibold hover:underline"
          >
            Re-check permissions
          </button>
          {micDenied && (
            <div className="space-y-1 border-t border-[var(--ui-hairline)] pt-2">
              <button
                type="button"
                onClick={handleResetMicrophone}
                className="text-xs font-semibold underline hover:no-underline"
              >
                Reset microphone permission…
              </button>
              <p className="text-[10px] leading-relaxed text-on-surface-variant">
                Clears Murmur's stale Microphone entry and restarts Murmur, so it
                is unavailable while you record or are in a meeting. macOS asks
                again the next time you record.
              </p>
              {micResetError && (
                <p
                  role="alert"
                  className="rounded-[var(--ui-radius-control)] border border-error bg-surface-container-lowest px-2 py-1 text-xs text-error"
                >
                  {micResetError}
                </p>
              )}
            </div>
          )}
          {needsAccessibility && (
            <div className="space-y-1 border-t border-[var(--ui-hairline)] pt-2">
              <button
                type="button"
                onClick={handleResetAccessibility}
                className="text-xs font-semibold underline hover:no-underline"
              >
                Reset Accessibility permission…
              </button>
              <p className="text-[10px] leading-relaxed text-on-surface-variant">
                Clears Murmur's stale Accessibility entry and restarts Murmur, so
                it is unavailable while you record or are in a meeting. When it is
                back, macOS asks again; turn it on then.
              </p>
              {resetError && (
                <p
                  role="alert"
                  className="rounded-[var(--ui-radius-control)] border border-error bg-surface-container-lowest px-2 py-1 text-xs text-error"
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
        className="grid h-5 w-5 shrink-0 place-items-center rounded-[var(--ui-radius-control)] text-warning transition-colors hover:bg-warning/10"
        aria-label="Dismiss"
      >
        <svg className="h-3.5 w-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>
    </div>
  );
}
