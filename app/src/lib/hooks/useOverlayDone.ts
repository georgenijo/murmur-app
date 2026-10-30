import { useEffect, useRef, useState } from 'react';
import type { DictationStatus } from '../types';

/** Keep the processing plate for its centre-out finish; never delay a restart. */
export function useOverlayDone(status: DictationStatus): boolean {
  const previous = useRef(status);
  const [done, setDone] = useState(false);
  const completing = status === 'idle' && previous.current === 'processing';
  useEffect(() => {
    const completed = previous.current === 'processing' && status === 'idle';
    previous.current = status;
    setDone(completed);
    if (!completed) return;
    const timer = setTimeout(() => setDone(false), 400);
    return () => clearTimeout(timer);
  }, [status]);
  return status === 'idle' && (done || completing);
}
