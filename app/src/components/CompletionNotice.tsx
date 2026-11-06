import { useEffect, useState } from 'react';
import { listen } from '../lib/targetedEvents';
import { COMPLETION_CHANGED, COMPLETION_FAILED, completionSnapshot, importCompletionV1 } from '../lib/completionData';

export function CompletionNotice() {
  const [failed, setFailed] = useState(false);
  const [version, setVersion] = useState(0);
  const [retrying, setRetrying] = useState(false);
  useEffect(() => {
    const changed = () => setVersion(v => v + 1);
    window.addEventListener(COMPLETION_CHANGED, changed);
    const failed = () => setFailed(true);
    window.addEventListener(COMPLETION_FAILED, failed);
    const subscription = listen('completion-storage-error', () => setFailed(true));
    return () => { window.removeEventListener(COMPLETION_CHANGED, changed); window.removeEventListener(COMPLETION_FAILED, failed); void subscription.then(fn => fn()); };
  }, []);
  void version;
  if (!failed && completionSnapshot() && !completionSnapshot()?.retryNotice) return null;
  return <div role="alert" className="p-3 ui-text-body">
    Murmur couldn’t finish loading your saved activity. Retry to finish.
    <button disabled={retrying} onClick={() => {
      setRetrying(true);
      void importCompletionV1().then(() => setFailed(false), () => setFailed(true)).finally(() => setRetrying(false));
    }}>Retry</button>
  </div>;
}
