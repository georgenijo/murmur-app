import type { DictationStatus } from '../../lib/types';
import type { MeetingRuntimePhase } from '../../lib/meetings';
import type { MicrophoneFailureCue } from '../../lib/hooks/useOverlayRuntime';

/**
 * Which indicator the top-bar left slot shows. Mirrors the priority chain that
 * used to live inline as a ternary in OverlayWidget's JSX: cancelled beats a
 * hotkey-miss flash, which beats an active status, which beats idle. `dimmed`
 * only applies to the idle mic icon (the global-disable dimming effect).
 */
export type OverlayIndicator =
  | { kind: 'cancelled' }
  | { kind: 'hotkeyMiss' }
  | { kind: 'microphoneFailure'; failure: MicrophoneFailureCue }
  | { kind: 'starting'; slow: boolean }
  | { kind: 'recording' }
  | { kind: 'recovering' }
  | { kind: 'processing' }
  | { kind: 'calibrating' }
  | { kind: 'meeting'; processing: boolean }
  | { kind: 'clipboardOnly' }
  | { kind: 'idle'; dimmed: boolean };

const PRIORITY_CUES = new Set<OverlayIndicator['kind']>([
  'calibrating', 'cancelled', 'microphoneFailure', 'hotkeyMiss',
]);

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
 * Priority: calibration > cancelled > microphone failure > hotkey miss >
 * meeting > starting > recording > recovering > processing > clipboard-only > idle.
 */
export function deriveVisual(
  status: DictationStatus,
  showCancelled: boolean,
  showHotkeyMiss: boolean,
  disabled: boolean,
  showMicrophoneFailure: MicrophoneFailureCue | false | null = false,
  stillConnecting: boolean = false,
  calibrating: boolean = false,
  meetingPhase: MeetingRuntimePhase = 'idle',
  showClipboardOnly: boolean = false,
): OverlayVisual {
  let indicator: OverlayIndicator;
  if (calibrating) {
    indicator = { kind: 'calibrating' };
  } else if (showCancelled) {
    indicator = { kind: 'cancelled' };
  } else if (showMicrophoneFailure) {
    indicator = { kind: 'microphoneFailure', failure: showMicrophoneFailure };
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
  } else if (showClipboardOnly) {
    indicator = { kind: 'clipboardOnly' };
  } else {
    indicator = { kind: 'idle', dimmed: disabled };
  }

  return {
    indicator,
    showTapMissedLabel: showHotkeyMiss,
    // A priority cue owns the bar while it flashes; live levels would bury it.
    waveformVisible: status === 'recording' && !PRIORITY_CUES.has(indicator.kind),
  };
}
