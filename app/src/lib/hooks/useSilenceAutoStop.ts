import { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { flog } from '../log';
import {
  initialSilenceState,
  reduceSilenceSample,
  type SilenceAutoStopState,
} from '../silenceAutoStop';
import { useRecordingOrigin } from './useRecordingOrigin';
import type { DictationStatus } from '../types';

interface UseSilenceAutoStopProps {
  /** Off unless the user enabled it. */
  enabled: boolean;
  status: DictationStatus;
  /** Trailing silence that ends the recording; 0 disables the detector. */
  silenceMs: number;
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
 * lives in `reduceSilenceSample`; this hook only owns subscription, per-recording
 * reset, the origin gate, and the single call out.
 */
export function useSilenceAutoStop({ enabled, status, silenceMs, onAutoStop }: UseSilenceAutoStopProps) {
  const { getOrigin, resetOrigin } = useRecordingOrigin();
  const stateRef = useRef<SilenceAutoStopState>(initialSilenceState());
  const enabledRef = useRef(enabled);
  const statusRef = useRef(status);
  const silenceMsRef = useRef(silenceMs);
  const onAutoStopRef = useRef(onAutoStop);
  useEffect(() => { enabledRef.current = enabled; }, [enabled]);
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
      if (wasRecording || cancelledBeforeReady) resetOrigin();
    }
  }, [status, resetOrigin]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
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
      if (!result.stop) return;
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
