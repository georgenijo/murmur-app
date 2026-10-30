/**
 * Smart Auto-Stop (#834): ask Pipecat's Smart Turn model whether a pause is
 * the end of the speaker's turn, and stop early when it is.
 *
 * It rides on Stop on Silence. The silence detector decides when a pause has
 * begun; once it has lasted `SMART_TURN_CHECK_MS` the hook asks Rust to score
 * the tail of the recording, once per pause. A score at or over the threshold
 * stops the recording; anything else — a lower score, no model, a busy or
 * failed check — leaves it to the silence timer, which stays the hard ceiling.
 *
 * The decisions are pure so they are unit-testable without timers or IPC.
 */
import { invoke } from '@tauri-apps/api/core';
import type { SilenceAutoStopState } from './silenceAutoStop';

/** Silence after speech before the model is asked (Hex's tuned check point). */
export const SMART_TURN_CHECK_MS = 300;

/** Probability at or over which the turn counts as complete. */
export const SMART_TURN_THRESHOLD = 0.7;

/** Pinned smart-turn-v3.2-cpu.onnx. */
export const SMART_TURN_MODEL_SIZE_LABEL = '~8.7 MB';

export type SmartTurnModelState = 'notDownloaded' | 'downloading' | 'ready';

export interface SmartTurnModelStatus {
  state: SmartTurnModelState;
  sizeBytes: number;
}

/** Mirrors `SmartTurnCheckResult` in `commands/smart_turn.rs`. */
export type SmartTurnCheckResult =
  | { outcome: 'scored'; recordingId: number; probability: number; inferenceMs: number }
  | { outcome: 'notRecording' }
  | { outcome: 'tooShort' }
  | { outcome: 'unavailable'; reason: string };

export function smartTurnModelStatus(): Promise<SmartTurnModelStatus> {
  return invoke('smart_turn_model_status');
}

export function downloadSmartTurnModel(): Promise<void> {
  return invoke('download_smart_turn_model');
}

export function removeSmartTurnModel(): Promise<void> {
  return invoke('remove_smart_turn_model');
}

export function checkSmartTurn(): Promise<SmartTurnCheckResult> {
  return invoke('check_smart_turn');
}

/**
 * Whether the current pause should be scored now: speech has been heard, the
 * pause has lasted the check point, and this pause has not been asked about.
 */
export function smartTurnCheckDue(state: SilenceAutoStopState, lastCheckedPause: number): boolean {
  return state.armed
    && !state.stopped
    && state.silenceMs >= SMART_TURN_CHECK_MS
    && state.pause !== lastCheckedPause;
}

export type SmartTurnVerdict = 'stop' | 'continue' | 'stale';

/**
 * Judge an answer about `askedPause` against the detector state now. An answer
 * is stale when the recording already stopped, speech resumed, or a newer
 * pause began while the model ran — only a still-running pause may end.
 */
export function smartTurnVerdict(
  now: SilenceAutoStopState,
  askedPause: number,
  result: SmartTurnCheckResult,
): SmartTurnVerdict {
  if (now.stopped || now.pause !== askedPause || now.silenceMs <= 0) return 'stale';
  if (result.outcome !== 'scored' || !Number.isFinite(result.probability)) return 'continue';
  return result.probability >= SMART_TURN_THRESHOLD ? 'stop' : 'continue';
}
