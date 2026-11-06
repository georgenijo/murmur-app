import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '../targetedEvents';

type WaveformConsumer = 'main-recording' | 'corpus' | 'overlay';
// Serialize interest changes, including StrictMode cleanup/remount, so a late
// activation cannot undo a newer deactivation. Native visibility is independent.
let updates = Promise.resolve();
function interest(consumer: WaveformConsumer, active: boolean) {
  updates = updates.then(() => invoke<void>('set_waveform_subscription', { consumer, active })).catch(() => {});
}
export function useAudioLevel(consumer: WaveformConsumer, active: boolean, accept: (level: number) => void) {
  const acceptRef = useRef(accept);
  acceptRef.current = accept;
  useEffect(() => {
    if (!active) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<number>('audio-level', ({ payload }) => {
      if (!disposed) acceptRef.current(payload);
    }).then(stop => {
      if (disposed) { stop(); return; }
      unlisten = stop;
      interest(consumer, true);
    }).catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
      interest(consumer, false);
    };
  }, [consumer, active]);
}
