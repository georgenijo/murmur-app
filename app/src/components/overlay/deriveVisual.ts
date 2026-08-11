import type { DictationStatus } from '../../lib/types';
import type { MeetingRuntimePhase } from '../../lib/meetings';

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
  | { kind: 'microphoneFailure' }
  | { kind: 'starting'; slow: boolean }
  | { kind: 'recording' }
  | { kind: 'recovering' }
  | { kind: 'processing' }
  | { kind: 'transforming' }
  | { kind: 'calibrating' }
  | { kind: 'meeting'; processing: boolean }
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
 * Pure derivation of the overlay's top-bar visual state from status, transient
 * flashes, and global-disable. No React or timers live here.
 *
 * Priority: cancelled > secure-field flash > microphone failure >
 * transform-busy flash > hotkey-miss > starting > recording > recovering >
 * processing > transforming > idle.
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
  showMicrophoneFailure: boolean = false,
  stillConnecting: boolean = false,
  calibrating: boolean = false,
  meetingPhase: MeetingRuntimePhase = 'idle',
): OverlayVisual {
  let indicator: OverlayIndicator;
  if (calibrating) {
    indicator = { kind: 'calibrating' };
  } else if (showCancelled) {
    indicator = { kind: 'cancelled' };
  } else if (showSecureField) {
    indicator = { kind: 'secureField' };
  } else if (showMicrophoneFailure) {
    indicator = { kind: 'microphoneFailure' };
  } else if (showTransformBusy) {
    indicator = { kind: 'transformBusy' };
  } else if (showHotkeyMiss) {
    indicator = { kind: 'hotkeyMiss' };
  } else if (meetingPhase !== 'idle' && meetingPhase !== 'failed') {
    indicator = { kind: 'meeting', processing: meetingPhase === 'processing' };
  } else if (status === 'starting') {
    indicator = { kind: 'starting', slow: stillConnecting };
  } else if (status === 'recording') {
    indicator = { kind: 'recording' };
  } else if (status === 'recovering') {
    indicator = { kind: 'recovering' };
  } else if (status === 'processing') {
    indicator = { kind: 'processing' };
  } else if (transforming) {
    indicator = { kind: 'transforming' };
  } else {
    indicator = { kind: 'idle', dimmed: disabled };
  }

  return {
    indicator,
    showTapMissedLabel: showHotkeyMiss,
    waveformVisible: status === 'recording' && !calibrating,
  };
}
