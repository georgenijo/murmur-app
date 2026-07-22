import type { DictationStatus } from '../../lib/types';

/**
 * Which indicator the top-bar left slot shows. Mirrors the priority chain that
 * used to live inline as a ternary in OverlayWidget's JSX: cancelled beats a
 * hotkey-miss flash, which beats an active status, which beats idle. `dimmed`
 * only applies to the idle mic icon (the global-disable dimming effect).
 */
export type OverlayIndicator =
  | { kind: 'cancelled' }
  | { kind: 'secureField' }
  | { kind: 'transformBusy' }
  | { kind: 'hotkeyMiss' }
  | { kind: 'recording' }
  | { kind: 'processing' }
  | { kind: 'transforming' }
  | { kind: 'idle'; dimmed: boolean };

export interface OverlayVisual {
  /** Which icon/badge the top-bar left slot renders. */
  indicator: OverlayIndicator;
  /**
   * Hotkey-miss flash is active: the amber border glow lights up and the
   * "Tap missed" label shows in the dropdown row (below notch height — it is
   * too wide for a wing). The `!` badge itself is carried by `indicator`.
   */
  showTapMissedLabel: boolean;
  /** Waveform bars are visible (opacity 1) vs. hidden (opacity 0). */
  waveformVisible: boolean;
}

/**
 * Pure derivation of the overlay's top-bar visual state from status + the two
 * transient flash flags + global-disable. No React, no timers — this only
 * encodes "given these four inputs, what does the top bar look like," exactly
 * as the original ternary chain in OverlayWidget.tsx did:
 *
 *   showCancelled ? X : showHotkeyMiss ? ! : status==='recording' ? dot
 *     : status==='processing' ? spinner : mic (dimmed if disabled)
 *
 * Priority: cancelled > secure-field flash > transform-busy flash >
 * hotkey-miss > recording > processing > transforming > idle. (Since `status`
 * is a single enum value, recording and processing can never both be true, so
 * their relative order does not change behavior — only idle's position at the
 * end, after both, matters.)
 *
 * `transforming` and `showSecureField` (issue #312 PR-C2) are the transform
 * flow's overlay affordances: the "transforming…" indicator shown while
 * the local LLM is thinking, and a brief flash when a password/secure field is
 * refused. `showTransformBusy` (issue #329) flashes when a transform keypress
 * was refused because dictation/benchmark/file-transcription/a mid-flight
 * transform owns the pipeline. All default off so the dictation call sites
 * are unchanged.
 */
export function deriveVisual(
  status: DictationStatus,
  showCancelled: boolean,
  showHotkeyMiss: boolean,
  disabled: boolean,
  transforming: boolean = false,
  showSecureField: boolean = false,
  showTransformBusy: boolean = false,
): OverlayVisual {
  const indicator: OverlayIndicator = showCancelled
    ? { kind: 'cancelled' }
    : showSecureField
      ? { kind: 'secureField' }
      : showTransformBusy
        ? { kind: 'transformBusy' }
        : showHotkeyMiss
          ? { kind: 'hotkeyMiss' }
          : status === 'recording'
            ? { kind: 'recording' }
            : status === 'processing'
              ? { kind: 'processing' }
              : transforming
                ? { kind: 'transforming' }
                : { kind: 'idle', dimmed: disabled };

  return {
    indicator,
    showTapMissedLabel: showHotkeyMiss,
    waveformVisible: status === 'recording',
  };
}
