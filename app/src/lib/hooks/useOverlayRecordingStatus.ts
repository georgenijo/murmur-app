import { useEffect, useRef, useState } from 'react';
import { subscribeRecordingStatus } from '../recordingStatus';
import { type DictationStatus } from '../types';
import { flog } from '../log';

export function useOverlayRecordingStatus() {
  const [status, setStatus] = useState<DictationStatus>('idle');
  const statusRef = useRef<DictationStatus>('idle');

  useEffect(() => subscribeRecordingStatus(next => {
    statusRef.current = next;
    setStatus(next);
    flog.info('overlay', 'status changed', { status: next });
  }), []);

  return { status, statusRef };
}
