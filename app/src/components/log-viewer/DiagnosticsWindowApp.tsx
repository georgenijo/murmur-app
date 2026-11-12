import { AppSurface } from '../ui/AppSurface';
import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
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
 * costs nothing and events keep accumulating. */
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
