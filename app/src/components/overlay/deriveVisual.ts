import type { DictationStatus } from '../../lib/types';

/**
 * Which indicator the top-bar left slot shows. Mirrors the priority chain that
 * used to live inline as a ternary in OverlayWidget's JSX: cancelled beats a
 * hotkey-miss flash, which beats an active status, which beats idle. `dimmed`
 * only applies to the idle mic icon (the global-disable dimming effect).
 */
export type OverlayIndicator =
  | { kind: 'cancelled' }
  | { kind: 'hotkeyMiss' }
  | { kind: 'recording' }
  | { kind: 'processing' }
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
 * Priority: cancelled > hotkey-miss > recording > processing > idle. (Since
 * `status` is a single enum value, recording and processing can never both be
 * true, so their relative order does not change behavior — only idle's
 * position at the end, after both, matters.)
 */
export function deriveVisual(
  status: DictationStatus,
  showCancelled: boolean,
  showHotkeyMiss: boolean,
  disabled: boolean,
): OverlayVisual {
  const indicator: OverlayIndicator = showCancelled
    ? { kind: 'cancelled' }
    : showHotkeyMiss
      ? { kind: 'hotkeyMiss' }
      : status === 'recording'
        ? { kind: 'recording' }
        : status === 'processing'
          ? { kind: 'processing' }
          : { kind: 'idle', dimmed: disabled };

  return {
    indicator,
    showTapMissedLabel: showHotkeyMiss,
    waveformVisible: status === 'recording',
  };
}
