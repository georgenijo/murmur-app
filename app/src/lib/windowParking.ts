import { useEffect, useState } from 'react';

/** How long the main window must stay hidden before its visible UI unmounts.
 * Long enough that closing and reopening the window right away keeps every
 * view exactly as it was. */
export const WINDOW_PARK_DELAY_MS = 60_000;

const blockers = new Set<symbol>();
const listeners = new Set<() => void>();

function notify() {
  for (const listener of listeners) listener();
}

/** Keep the window's UI mounted while `active`: a download, recording or run
 * whose progress or cleanup lives in a mounted component. */
export function useBlockWindowParking(active: boolean) {
  useEffect(() => {
    if (!active) return undefined;
    const token = Symbol('window-parking-blocker');
    blockers.add(token);
    notify();
    return () => {
      blockers.delete(token);
      notify();
    };
  }, [active]);
}

/** True once the document has been hidden for `delayMs` with nothing blocking.
 * The caller then renders only an empty surface: its hooks (recording,
 * shortcuts, updater) keep running, while the DOM, layers and component state
 * of the visible UI are released. Showing the window renders it again. */
export function useWindowParked(delayMs = WINDOW_PARK_DELAY_MS): boolean {
  const [parked, setParked] = useState(false);
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const evaluate = () => {
      if (timer !== undefined) clearTimeout(timer);
      timer = undefined;
      if (document.visibilityState === 'visible') {
        setParked(false);
        return;
      }
      if (blockers.size > 0) return;
      timer = setTimeout(() => {
        timer = undefined;
        if (document.visibilityState !== 'visible' && blockers.size === 0) setParked(true);
      }, delayMs);
    };
    evaluate();
    document.addEventListener('visibilitychange', evaluate);
    listeners.add(evaluate);
    return () => {
      if (timer !== undefined) clearTimeout(timer);
      document.removeEventListener('visibilitychange', evaluate);
      listeners.delete(evaluate);
    };
  }, [delayMs]);
  return parked;
}
