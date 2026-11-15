import { AppSurface } from '../ui/AppSurface';
import { useEffect, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { WindowHeader } from '../ui/WindowHeader';
import {
  DiagnosticsWorkspace,
  isDiagnosticsTab,
  type DiagnosticsTab,
} from './DiagnosticsWorkspace';

const DIAGNOSTICS_TAB_REQUESTED_EVENT = 'diagnostics-tab-requested';
const GET_REQUESTED_TAB_COMMAND = 'get_requested_diagnostics_tab';

interface DiagnosticsTabRequest { tab: string; seq: number }

function isTabRequest(value: unknown): value is DiagnosticsTabRequest {
  if (typeof value !== 'object' || value === null) return false;
  const { tab, seq } = value as Partial<DiagnosticsTabRequest>;
  return typeof tab === 'string' && typeof seq === 'number';
}

/** The window is built when it is first opened and destroyed on close. Its
 * data loads (event history, performance runs, resource samples) wait until
 * the window is shown: a tab request shows it, and a focus covers any other
 * way the window reaches the user. Rust retains the newest request with an
 * increasing `seq`. The page subscribes to the event first and then reads the
 * retained request, so a request made while the page was loading is found,
 * and whichever of the two answers is older never overwrites a newer one. */
export function DiagnosticsWindowApp() {
  const [requestedTab, setRequestedTab] = useState<DiagnosticsTab>('events');
  const [shown, setShown] = useState(false);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let newestSeq = 0;
    const apply = (request: unknown) => {
      if (disposed || !isTabRequest(request) || request.seq <= newestSeq) return;
      newestSeq = request.seq;
      setShown(true);
      if (isDiagnosticsTab(request.tab)) setRequestedTab(request.tab);
    };
    void listen<unknown>(DIAGNOSTICS_TAB_REQUESTED_EVENT, event => apply(event.payload)).then(stop => {
      if (disposed) {
        stop();
        return;
      }
      unlisten = stop;
      if (isTauri()) {
        void invoke<unknown>(GET_REQUESTED_TAB_COMMAND).then(apply).catch(() => {});
      }
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
