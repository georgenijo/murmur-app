import type { DictationStatus } from '../../lib/types';

/**
 * Which indicator the top-bar left slot shows. Mirrors the priority chain that
 * used to live inline as a ternary in OverlayWidget's JSX: cancelled beats a
 * hotkey-miss flash, which beats an active status, which beats idle.
 *
 * `disabled` is its own kind rather than a flag on `idle` because global
 * disable is a distinct state the user must be able to read at a glance, and
 * the renderer owes it a distinct shape — not an alpha applied to the idle mic.
 */
export type OverlayIndicator =
  | { kind: 'cancelled' }
  | { kind: 'hotkeyMiss' }
  | { kind: 'recording' }
  | { kind: 'processing' }
  | { kind: 'disabled' }
  | { kind: 'idle' };

export interface OverlayVisual {
  /** Which icon/badge the top-bar left slot renders. */
  indicator: OverlayIndicator;
  /** Right side shows the "Tap missed" label instead of the waveform. */
  showTapMissedLabel: boolean;
  /** Waveform bars are visible (opacity 1) vs. hidden (opacity 0). */
  waveformVisible: boolean;
  /** Pill is at its active (expanded) width/margin rather than idle. */
  isActive: boolean;
}

/**
 * Pure derivation of the overlay's top-bar visual state from status + the two
 * transient flash flags + global-disable. No React, no timers — this only
 * encodes "given these four inputs, what does the top bar look like," exactly
 * as the original ternary chain in OverlayWidget.tsx did:
 *
 *   showCancelled ? X : showHotkeyMiss ? ! : status==='recording' ? dot
 *     : status==='processing' ? spinner : disabled ? slashed mic : mic
 *
 * Priority: cancelled > hotkey-miss > recording > processing > disabled > idle.
 * (Since `status` is a single enum value, recording and processing can never
 * both be true, so their relative order does not change behavior — only the
 * disabled/idle pair's position at the end, after both, matters.)
 *
 * Global disable sits below the transient flashes and the active statuses on
 * purpose: a cancelled or hotkey-miss flash is a response to something the user
 * just did and must not be swallowed, and a recording already in flight when
 * disable lands should keep showing its true state until it settles.
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
          : disabled
            ? { kind: 'disabled' }
            : { kind: 'idle' };

  return {
    indicator,
    showTapMissedLabel: showHotkeyMiss,
    waveformVisible: status === 'recording',
    isActive: status === 'recording' || status === 'processing' || showCancelled || showHotkeyMiss,
  };
}
