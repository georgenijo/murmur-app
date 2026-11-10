import { useEffect, useState } from 'react';
import { checkMicrophonePermissionStatus, type MicPermissionStatus } from '../dictation';

/** Query TCC without opening an audio device; refresh when returning from Settings. */
export function useMicrophonePermission(): MicPermissionStatus {
  const [permission, setPermission] = useState<MicPermissionStatus>('unknown');
  useEffect(() => {
    let disposed = false;
    let revision = 0;
    const refresh = async () => {
      const current = ++revision;
      const next = await checkMicrophonePermissionStatus().catch(() => 'unknown' as const);
      if (!disposed && current === revision) setPermission(next);
    };
    void refresh();
    window.addEventListener('focus', refresh);
    return () => { disposed = true; window.removeEventListener('focus', refresh); };
  }, []);
  return permission;
}
