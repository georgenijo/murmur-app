import { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { flog } from '../log';
import {
  initialSilenceState,
  latchSilenceStop,
  reduceSilenceSample,
  type SilenceAutoStopState,
} from '../silenceAutoStop';
import { checkSmartTurn, smartTurnCheckDue, smartTurnVerdict } from '../smartTurn';
import { useRecordingOrigin } from './useRecordingOrigin';
import type { DictationStatus } from '../types';

interface UseSilenceAutoStopProps {
  /** Off unless the user enabled it. */
  enabled: boolean;
  status: DictationStatus;
  /** Trailing silence that ends the recording; 0 disables the detector. */
  silenceMs: number;
  /**
   * Smart Auto-Stop: also ask the Smart Turn model at each pause and stop
   * early when it says the turn is complete. `silenceMs` stays the ceiling,
   * so this does nothing while Stop on Silence is Off.
   */
  smartEnabled?: boolean;
  /** Invoked at most once per recording, on the silence transition. */
  onAutoStop: () => void;
}

/**
 * End a hands-free recording after a run of trailing silence.
 *
 * "Hands-free" means any recording that was **not started by holding the
 * trigger key**: double-tap, the main-window button, the overlay click, and
 * locked mode. While a hold-started recording is in flight the detector
 * ignores samples entirely — there the key release owns the stop, and ending
 * a recording while the trigger is still physically held would be wrong.
 *
 * Reuses the `audio-level` RMS stream the overlay waveform already listens to,
 * so no extra audio path or permission is involved. All of the decision logic
 * lives in `reduceSilenceSample` (and, for Smart Auto-Stop, `smartTurnCheckDue`
 * and `smartTurnVerdict`); this hook only owns subscription, per-recording
 * reset, the origin gate, the per-pause Smart Turn check, and the single call
 * out.
 */
export function useSilenceAutoStop({
  enabled,
  status,
  silenceMs,
  smartEnabled = false,
  onAutoStop,
}: UseSilenceAutoStopProps) {
  const { getOrigin, resetOrigin } = useRecordingOrigin();
  const stateRef = useRef<SilenceAutoStopState>(initialSilenceState());
  const enabledRef = useRef(enabled);
  const statusRef = useRef(status);
  const silenceMsRef = useRef(silenceMs);
  const onAutoStopRef = useRef(onAutoStop);
  const smartEnabledRef = useRef(smartEnabled);
  // Bumped per recording so a Smart Turn answer for an earlier one is dropped.
  const generationRef = useRef(0);
  const checkedPauseRef = useRef(0);
  const smartInFlightRef = useRef(false);
  useEffect(() => { enabledRef.current = enabled; }, [enabled]);
  useEffect(() => { smartEnabledRef.current = smartEnabled; }, [smartEnabled]);
  useEffect(() => { silenceMsRef.current = silenceMs; }, [silenceMs]);
  useEffect(() => { onAutoStopRef.current = onAutoStop; }, [onAutoStop]);

  // Every recording starts from a clean state — peak, armed and the silence run
  // are all per-recording, and a latched stop must not leak into the next one.
  // The origin is per-recording too: clearing it exactly on the transition out
  // of 'recording' heals a 'hold' whose stop event was never delivered (Escape
  // cancel, dead rdev thread), while a transition between non-recording states
  // can never wipe a freshly latched hold before its recording starts.
  useEffect(() => {
    if (statusRef.current !== status) {
      const previousStatus = statusRef.current;
      const wasRecording = previousStatus === 'recording';
      const cancelledBeforeReady = status === 'idle'
        && (previousStatus === 'starting' || previousStatus === 'recovering');
      statusRef.current = status;
      stateRef.current = initialSilenceState();
      generationRef.current += 1;
      checkedPauseRef.current = 0;
      if (wasRecording || cancelledBeforeReady) resetOrigin();
    }
  }, [status, resetOrigin]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    // One check per pause, never two at once. Every non-stop answer — a low
    // score, no model, a busy or failed check — leaves the silence timer in
    // charge; Rust logs the content-free outcome of each check.
    const requestSmartTurn = (pause: number) => {
      const generation = generationRef.current;
      checkedPauseRef.current = pause;
      smartInFlightRef.current = true;
      checkSmartTurn()
        .then((answer) => {
          if (cancelled || generationRef.current !== generation) return;
          if (!enabledRef.current || !smartEnabledRef.current || silenceMsRef.current <= 0) return;
          if (statusRef.current !== 'recording' || getOrigin() === 'hold') return;
          if (smartTurnVerdict(stateRef.current, pause, answer) !== 'stop') return;
          stateRef.current = latchSilenceStop(stateRef.current);
          flog.info('recording', 'smart auto-stop fired', {
            probability: answer.outcome === 'scored' ? Math.round(answer.probability * 1000) / 1000 : null,
            inferenceMs: answer.outcome === 'scored' ? answer.inferenceMs : null,
            silenceMs: Math.round(stateRef.current.silenceMs),
            speechMs: Math.round(stateRef.current.speechMs),
          });
          onAutoStopRef.current();
        })
        .catch(() => {})
        .finally(() => { smartInFlightRef.current = false; });
    };
    listen<number>('audio-level', (event) => {
      if (!enabledRef.current || statusRef.current !== 'recording') return;
      if (silenceMsRef.current <= 0) return;
      // Hold-started recordings never accumulate silence, so nothing can fire
      // in the moments between the key release and the status transition.
      if (getOrigin() === 'hold') return;
      const result = reduceSilenceSample(
        stateRef.current,
        { level: event.payload, atMs: Date.now() },
        silenceMsRef.current,
      );
      stateRef.current = result.state;
      if (!result.stop) {
        if (
          smartEnabledRef.current
          && !smartInFlightRef.current
          && smartTurnCheckDue(result.state, checkedPauseRef.current)
        ) {
          requestSmartTurn(result.state.pause);
        }
        return;
      }
      flog.info('recording', 'silence auto-stop fired', {
        silenceMs: silenceMsRef.current,
        speechMs: Math.round(result.state.speechMs),
      });
      onAutoStopRef.current();
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    }).catch(() => {});
    return () => { cancelled = true; unlisten?.(); };
  }, [getOrigin]);
}
