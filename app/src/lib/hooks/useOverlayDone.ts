import { useEffect, useRef, useState } from 'react';
import type { DictationStatus } from '../types';
import { OVERLAY_DONE_HOLD_MS } from '../overlayMotion';

/** Keep the processing plate for its brief finish; never delay a restart. */
export function useOverlayDone(status: DictationStatus): boolean {
  const previous = useRef(status);
  const [done, setDone] = useState(false);
  const completing = status === 'idle' && previous.current === 'processing';
  useEffect(() => {
    const completed = previous.current === 'processing' && status === 'idle';
    previous.current = status;
    setDone(completed);
    if (!completed) return;
    const timer = setTimeout(() => setDone(false), OVERLAY_DONE_HOLD_MS);
    return () => clearTimeout(timer);
  }, [status]);
  return status === 'idle' && (done || completing);
}
