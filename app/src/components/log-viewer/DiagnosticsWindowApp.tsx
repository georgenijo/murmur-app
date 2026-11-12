import { AppSurface } from '../ui/AppSurface';
import { useEffect, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { WindowHeader } from '../ui/WindowHeader';
import {
  DiagnosticsWorkspace,
  isDiagnosticsTab,
  type DiagnosticsTab,
} from './DiagnosticsWorkspace';

const DIAGNOSTICS_TAB_REQUESTED_EVENT = 'diagnostics-tab-requested';

/** The window is created hidden at launch. Its data loads (event history,
 * performance runs, resource samples) wait until the window is first shown:
 * `show_diagnostics_window` emits the tab request, and a focus covers any other
 * way the window reaches the user. Once shown it stays live, so reopening it
 * costs nothing and events keep accumulating. The listeners attach only after
 * settings hydration, so a window that was shown or focused before then (a
 * pop-out right after launch) is detected on mount. */
export function DiagnosticsWindowApp() {
  const [requestedTab, setRequestedTab] = useState<DiagnosticsTab>('events');
  const [shown, setShown] = useState(false);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<string>(DIAGNOSTICS_TAB_REQUESTED_EVENT, event => {
      if (disposed) return;
      setShown(true);
      if (isDiagnosticsTab(event.payload)) setRequestedTab(event.payload);
    }).then(stop => {
      if (disposed) stop();
      else unlisten = stop;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (document.hasFocus()) {
      setShown(true);
      return undefined;
    }
    if (!isTauri()) return undefined;
    let disposed = false;
    void getCurrentWindow().isVisible().then(visible => {
      if (!disposed && visible) setShown(true);
    }).catch(() => {});
    return () => { disposed = true; };
  }, []);

  useEffect(() => {
    if (shown) return undefined;
    const onFocus = () => setShown(true);
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, [shown]);

  return (
    <AppSurface surface="diagnostics" className="flex h-screen min-h-0 flex-col overflow-hidden bg-background text-on-surface">
      <WindowHeader contextLabel="Diagnostics" />
      <main className="min-h-0 flex-1 overflow-hidden">
        <DiagnosticsWorkspace
          active={shown}
          requestedTab={requestedTab}
          storeHealthEnabled
          canArmPrivateCapture={false}
        />
      </main>
    </AppSurface>
  );
}
