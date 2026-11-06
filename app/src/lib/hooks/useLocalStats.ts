import { useEffect, useState } from 'react';
import { COMPLETION_CHANGED } from '../completionData';

/** Native sources own updates. Multiple mounted readers cannot add counts. */
export function useLocalStats(): number {
  const [version, setVersion] = useState(0);
  useEffect(() => {
    const changed = () => setVersion(v => v + 1);
    window.addEventListener(COMPLETION_CHANGED, changed);
    return () => window.removeEventListener(COMPLETION_CHANGED, changed);
  }, []);
  return version;
}
