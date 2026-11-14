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

declare global {
  interface Window { __MURMUR_DIAGNOSTICS_TAB__?: string }
}

/** The tab `show_diagnostics_window` asked for when it built this window. The
 * tab event it also emits can arrive before this page listens. */
export function initialDiagnosticsTab(): DiagnosticsTab | null {
  const tab = typeof window === 'undefined' ? undefined : window.__MURMUR_DIAGNOSTICS_TAB__;
  return typeof tab === 'string' && isDiagnosticsTab(tab) ? tab : null;
}

/** The window is built when it is first opened and destroyed on close. Its
 * data loads (event history, performance runs, resource samples) wait until
 * the window is shown: a build carries the requested tab, a reopen of a live
 * window emits it, and a focus covers any other way the window reaches the
 * user. The listeners attach only after settings hydration, so a window that
 * was shown or focused before then is detected on mount. */
export function DiagnosticsWindowApp() {
  const [requestedTab, setRequestedTab] = useState<DiagnosticsTab>(() => initialDiagnosticsTab() ?? 'events');
  const [shown, setShown] = useState(() => initialDiagnosticsTab() !== null);

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
