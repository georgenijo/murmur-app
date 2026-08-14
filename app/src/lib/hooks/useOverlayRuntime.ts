import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { flog } from '../log';
import { loadSettings } from '../settings';
import type { DictationStatus } from '../types';
import {
  HOTKEY_MISS_FLASH_MS,
  isHotkeyTapRejectedPayload,
  shouldShowHotkeyMissFeedback,
} from '../hotkeyFeedback';

const CANCELLED_FLASH_MS = 800;
/** How long the secure-field refusal flash shows (issue #312 PR-C2). */
const SECURE_FIELD_FLASH_MS = 800;
/** How long the transform-busy refusal flash shows (issue #329). */
const TRANSFORM_BUSY_FLASH_MS = 800;
export const MICROPHONE_FAILURE_FLASH_MS = 5000;
export const CLIPBOARD_ONLY_FLASH_MS = 5000;

export type MicrophoneFailureCue = 'deviceUnavailable' | 'generic';

type DictationDeliveryOutcome =
  | 'clipboardOnly'
  | 'autoPastePosted'
  | 'clipboardWriteFailed'
  | 'unconfirmed';

interface DictationDeliveryOutcomePayload {
  recordingId: number;
  outcome: DictationDeliveryOutcome;
}

function recordingIdFromPayload(value: unknown): number | null {
  if (!value || typeof value !== 'object') return null;
  const recordingId = (value as Record<string, unknown>).recordingId;
  return Number.isSafeInteger(recordingId) && (recordingId as number) > 0
    ? recordingId as number
    : null;
}

function isDictationDeliveryOutcomePayload(
  value: unknown,
): value is DictationDeliveryOutcomePayload {
  const recordingId = recordingIdFromPayload(value);
  if (recordingId == null) return false;
  const payload = value as Record<string, unknown>;
  return [
      'clipboardOnly',
      'autoPastePosted',
      'clipboardWriteFailed',
      'unconfirmed',
    ].includes(payload.outcome as string);
}

function microphoneFailureCueFromPayload(value: unknown): MicrophoneFailureCue | null {
  if (recordingIdFromPayload(value) == null) return null;
  return (value as Record<string, unknown>).errorKind === 'device_unavailable'
    ? 'deviceUnavailable'
    : 'generic';
}

export interface UseOverlayRuntimeArgs {
  /** Current dictation status (reactive value, not just a ref). */
  status: DictationStatus;
  /** Ref mirror of `status`, read from the hotkey-tap-rejected listener. */
  statusRef: React.MutableRefObject<DictationStatus>;
  disabled: boolean;
  setDisabled: (value: boolean) => void;
  showHotkeyMiss: boolean;
  setShowHotkeyMiss: (value: boolean) => void;
  /**
   * Shared with useOverlaySettingsMirror, which writes into it whenever a
   * settings snapshot is applied. Created in the composition shell (not
   * inside this hook or the settings-mirror hook) because both hooks need to
   * read/write it synchronously and neither can be constructed from the
   * other's return value without an artificial call-order dependency.
   */
  hotkeyMissFeedbackRef: React.MutableRefObject<boolean>;
}

export interface OverlayRuntime {
  showCancelled: boolean;
  showHotkeyMiss: boolean;
  /** Brief flash when a secure/password field was refused (issue #312). */
  showSecureField: boolean;
  /**
   * Brief flash when a transform keypress was refused because dictation /
   * benchmark / file transcription / a mid-flight transform owns the pipeline
   * (issue #329).
   */
  showTransformBusy: boolean;
  /** Bounded, content-free microphone-initialization failure cue. */
  showMicrophoneFailure: MicrophoneFailureCue | null;
  /** Text reached the clipboard, but automatic paste did not complete. */
  showClipboardOnly: boolean;
  disabled: boolean;
  setDisabled: (value: boolean) => void;
  /** Ref mirror of `disabled`, read synchronously by useRecordingControls. */
  disabledRef: React.MutableRefObject<boolean>;
}

/**
 * Owns the overlay's Rust-driven runtime signals that sit outside the
 * expansion lifecycle: the cancelled/hotkey-miss transient flashes and the
 * global-disable mirror (`app-disabled-changed`). `disabled`/`setDisabled`/
 * `showHotkeyMiss`/`setShowHotkeyMiss`/`hotkeyMissFeedbackRef` are created by
 * the composition shell and passed in (see the module doc on
 * `hotkeyMissFeedbackRef` above) — this hook attaches behavior to them and
 * re-exposes them so callers can consume the runtime as one object.
 */
export function useOverlayRuntime({
  status,
  statusRef,
  disabled,
  setDisabled,
  showHotkeyMiss,
  setShowHotkeyMiss,
  hotkeyMissFeedbackRef,
}: UseOverlayRuntimeArgs): OverlayRuntime {
  const [showCancelled, setShowCancelled] = useState(false);
  const [showSecureField, setShowSecureField] = useState(false);
  const [showTransformBusy, setShowTransformBusy] = useState(false);
  const [showMicrophoneFailure, setShowMicrophoneFailure] =
    useState<MicrophoneFailureCue | null>(null);
  const [showClipboardOnly, setShowClipboardOnly] = useState(false);
  const disabledRef = useRef(disabled);
  const hotkeyMissTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const clipboardOnlyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const microphoneFailureTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const microphoneFailureRecordingIdRef = useRef(0);
  const lastDeliveryRecordingIdRef = useRef(0);
  const latestRecordingGenerationRef = useRef(0);
  const clipboardListenerFailedRef = useRef(false);

  useEffect(() => { disabledRef.current = disabled; }, [disabled]);

  // Mirrors the non-idle branch of the original combined "status changed"
  // effect: any non-idle status clears a pending hotkey-miss flash. (The idle
  // branch — resetting locked mode — lives in useRecordingControls, the hook
  // that owns locked mode.)
  useEffect(() => {
    if (status === 'idle') return;
    setShowHotkeyMiss(false);
    if (hotkeyMissTimerRef.current) {
      clearTimeout(hotkeyMissTimerRef.current);
      hotkeyMissTimerRef.current = null;
    }
  }, [status, setShowHotkeyMiss]);

  // A rejected-tap event is emitted only when the double-tap window expires.
  // The setting gate lives here because the overlay is a separate webview with
  // its own React context and reads the shared localStorage settings snapshot.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<unknown>('hotkey-tap-rejected', (event) => {
      if (!isHotkeyTapRejectedPayload(event.payload)) return;
      let feedbackEnabled = hotkeyMissFeedbackRef.current;
      try {
        feedbackEnabled = loadSettings().hotkeyMissFeedback;
        hotkeyMissFeedbackRef.current = feedbackEnabled;
      } catch { /* use the latest settings snapshot */ }
      if (!shouldShowHotkeyMissFeedback(
        feedbackEnabled,
        statusRef.current,
        event.payload,
      )) return;

      if (hotkeyMissTimerRef.current) clearTimeout(hotkeyMissTimerRef.current);
      setShowHotkeyMiss(true);
      hotkeyMissTimerRef.current = setTimeout(() => {
        if (!cancelled) setShowHotkeyMiss(false);
        hotkeyMissTimerRef.current = null;
      }, HOTKEY_MISS_FLASH_MS);
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => {
      cancelled = true;
      if (hotkeyMissTimerRef.current) clearTimeout(hotkeyMissTimerRef.current);
      unlisten?.();
    };
  }, [statusRef, hotkeyMissFeedbackRef, setShowHotkeyMiss]);

  // Subscribe to recording-cancelled for brief red X flash
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    let timeoutId: ReturnType<typeof setTimeout> | null = null;
    listen('recording-cancelled', () => {
      if (timeoutId) clearTimeout(timeoutId);
      setShowCancelled(true);
      timeoutId = setTimeout(() => {
        if (!cancelled) setShowCancelled(false);
        timeoutId = null;
      }, CANCELLED_FLASH_MS);
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => {
      cancelled = true;
      if (timeoutId) clearTimeout(timeoutId);
      unlisten?.();
    };
  }, []);

  // Every terminal delivery outcome carries a monotonic recording ID. A newer
  // non-clipboard outcome clears any older cue, while duplicate/stale events
  // cannot extend or resurrect it.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<unknown>('dictation-delivery-outcome', (event) => {
      if (cancelled) return;
      if (clipboardListenerFailedRef.current) return;
      if (!isDictationDeliveryOutcomePayload(event.payload)) return;
      if (event.payload.recordingId < latestRecordingGenerationRef.current) return;
      if (event.payload.recordingId <= lastDeliveryRecordingIdRef.current) return;
      if (event.payload.recordingId > microphoneFailureRecordingIdRef.current) {
        if (microphoneFailureTimerRef.current) clearTimeout(microphoneFailureTimerRef.current);
        microphoneFailureTimerRef.current = null;
        microphoneFailureRecordingIdRef.current = 0;
        setShowMicrophoneFailure(null);
      }
      latestRecordingGenerationRef.current = Math.max(
        latestRecordingGenerationRef.current,
        event.payload.recordingId,
      );
      lastDeliveryRecordingIdRef.current = event.payload.recordingId;

      if (clipboardOnlyTimerRef.current) {
        clearTimeout(clipboardOnlyTimerRef.current);
        clipboardOnlyTimerRef.current = null;
      }
      const clipboardOnly = event.payload.outcome === 'clipboardOnly';
      setShowClipboardOnly(clipboardOnly);
      if (!clipboardOnly) return;

      clipboardOnlyTimerRef.current = setTimeout(() => {
        if (!cancelled) setShowClipboardOnly(false);
        clipboardOnlyTimerRef.current = null;
      }, CLIPBOARD_ONLY_FLASH_MS);
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    }).catch((cause: unknown) => {
      if (cancelled) return;
      clipboardListenerFailedRef.current = true;
      if (clipboardOnlyTimerRef.current) clearTimeout(clipboardOnlyTimerRef.current);
      clipboardOnlyTimerRef.current = null;
      setShowClipboardOnly(false);
      flog.warn('overlay', 'dictation-delivery-outcome listener unavailable', {
        error: String(cause),
      });
    });
    return () => {
      cancelled = true;
      if (clipboardOnlyTimerRef.current) clearTimeout(clipboardOnlyTimerRef.current);
      clipboardOnlyTimerRef.current = null;
      unlisten?.();
    };
  }, []);

  // Track recording ownership independently from the string-only status event.
  // If webview delivery is delayed or reordered, an outcome from an older
  // generation cannot install a cue over newer work.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<unknown>('dictation-generation-started', (event) => {
      if (cancelled) return;
      const recordingId = recordingIdFromPayload(event.payload);
      if (recordingId == null || recordingId <= latestRecordingGenerationRef.current) return;
      latestRecordingGenerationRef.current = recordingId;
      if (recordingId > microphoneFailureRecordingIdRef.current) {
        if (microphoneFailureTimerRef.current) clearTimeout(microphoneFailureTimerRef.current);
        microphoneFailureTimerRef.current = null;
        microphoneFailureRecordingIdRef.current = 0;
        setShowMicrophoneFailure(null);
      }

      // Microphone ownership is independent of clipboard listener health.
      // Clipboard correlation still fails closed if either of its listeners
      // could not be installed.
      if (clipboardListenerFailedRef.current) return;
      if (recordingId <= lastDeliveryRecordingIdRef.current) return;
      if (clipboardOnlyTimerRef.current) clearTimeout(clipboardOnlyTimerRef.current);
      clipboardOnlyTimerRef.current = null;
      setShowClipboardOnly(false);
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    }).catch((cause: unknown) => {
      if (cancelled) return;
      clipboardListenerFailedRef.current = true;
      if (clipboardOnlyTimerRef.current) clearTimeout(clipboardOnlyTimerRef.current);
      clipboardOnlyTimerRef.current = null;
      setShowClipboardOnly(false);
      flog.warn('overlay', 'dictation-generation-started listener unavailable', {
        error: String(cause),
      });
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // A new active lifecycle supersedes an idle cue immediately. This prevents a
  // prior recording's timer from resurfacing after newer work returns to idle.
  useEffect(() => {
    if (status === 'idle') return;
    if (clipboardOnlyTimerRef.current) clearTimeout(clipboardOnlyTimerRef.current);
    clipboardOnlyTimerRef.current = null;
    setShowClipboardOnly(false);
  }, [status]);

  useEffect(() => {
    let cancelled = false;
    const flashFailure = (recordingId: number, cue: MicrophoneFailureCue) => {
      if (cancelled || recordingId < latestRecordingGenerationRef.current) return;
      latestRecordingGenerationRef.current = Math.max(
        latestRecordingGenerationRef.current,
        recordingId,
      );
      microphoneFailureRecordingIdRef.current = recordingId;
      if (microphoneFailureTimerRef.current) {
        clearTimeout(microphoneFailureTimerRef.current);
      }
      setShowMicrophoneFailure(cue);
      microphoneFailureTimerRef.current = setTimeout(() => {
        if (!cancelled) {
          microphoneFailureRecordingIdRef.current = 0;
          setShowMicrophoneFailure(null);
        }
        microphoneFailureTimerRef.current = null;
      }, MICROPHONE_FAILURE_FLASH_MS);
    };
    const unlistens: (() => void)[] = [];
    listen<unknown>('recording-initialization-failed', (event) => {
      const recordingId = recordingIdFromPayload(event.payload);
      const cue = microphoneFailureCueFromPayload(event.payload);
      if (recordingId != null && cue != null) flashFailure(recordingId, cue);
    }).then((fn) => {
      if (cancelled) fn(); else unlistens.push(fn);
    }).catch((cause: unknown) => {
      if (!cancelled) {
        flog.warn('overlay', 'recording-initialization-failed listener unavailable', {
          error: String(cause),
        });
      }
    });
    listen<unknown>('recording-interrupted', (event) => {
      const recordingId = recordingIdFromPayload(event.payload);
      if (recordingId != null) flashFailure(recordingId, 'generic');
    }).then((fn) => {
      if (cancelled) fn(); else unlistens.push(fn);
    }).catch((cause: unknown) => {
      if (!cancelled) {
        flog.warn('overlay', 'recording-interrupted listener unavailable', {
          error: String(cause),
        });
      }
    });
    return () => {
      cancelled = true;
      if (microphoneFailureTimerRef.current) {
        clearTimeout(microphoneFailureTimerRef.current);
      }
      microphoneFailureTimerRef.current = null;
      microphoneFailureRecordingIdRef.current = 0;
      unlistens.forEach((fn) => fn());
    };
  }, []);

  // Brief flash when the transform flow refuses a secure/password field
  // (issue #312 PR-C2). Mirrors the recording-cancelled flash mechanism.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    let timeoutId: ReturnType<typeof setTimeout> | null = null;
    listen('transform-secure-field', () => {
      if (timeoutId) clearTimeout(timeoutId);
      setShowSecureField(true);
      timeoutId = setTimeout(() => {
        if (!cancelled) setShowSecureField(false);
        timeoutId = null;
      }, SECURE_FIELD_FLASH_MS);
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => {
      cancelled = true;
      if (timeoutId) clearTimeout(timeoutId);
      unlisten?.();
    };
  }, []);

  // Brief flash when a transform keypress was refused because something else
  // owns the pipeline (issue #329). Mirrors the secure-field flash mechanism.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    let timeoutId: ReturnType<typeof setTimeout> | null = null;
    listen('transform-busy', () => {
      if (timeoutId) clearTimeout(timeoutId);
      setShowTransformBusy(true);
      timeoutId = setTimeout(() => {
        if (!cancelled) setShowTransformBusy(false);
        timeoutId = null;
      }, TRANSFORM_BUSY_FLASH_MS);
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => {
      cancelled = true;
      if (timeoutId) clearTimeout(timeoutId);
      unlisten?.();
    };
  }, []);

  // Subscribe to app-disabled-changed events from Rust
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<boolean>('app-disabled-changed', (event) => {
      flog.info('overlay', 'app-disabled-changed', { disabled: event.payload });
      setDisabled(event.payload);
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, [setDisabled]);

  return {
    showCancelled,
    showSecureField,
    showTransformBusy,
    showMicrophoneFailure,
    showClipboardOnly,
    showHotkeyMiss,
    disabled,
    setDisabled,
    disabledRef,
  };
}
