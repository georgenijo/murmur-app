import { useEffect, useState } from 'react';
import { flushSourceState, SOURCE_STORAGE_FAILED, SOURCE_STORAGE_SAVED } from '../lib/webkitLifecycle';
export function SourceStorageNotice() {
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    const fail = () => setFailed(true);
    const saved = () => setFailed(false);
    window.addEventListener(SOURCE_STORAGE_FAILED, fail);
    window.addEventListener(SOURCE_STORAGE_SAVED, saved);
    return () => {
      window.removeEventListener(SOURCE_STORAGE_FAILED, fail);
      window.removeEventListener(SOURCE_STORAGE_SAVED, saved);
    };
  }, []);
  return failed ? <div role="alert">Your changes could not be saved. <button onClick={() => { void flushSourceState().catch(() => {}); }}>Retry</button></div> : null;
}
