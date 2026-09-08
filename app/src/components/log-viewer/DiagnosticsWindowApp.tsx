import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { WindowHeader } from '../ui/WindowHeader';
import {
  DiagnosticsWorkspace,
  isDiagnosticsTab,
  type DiagnosticsTab,
} from './DiagnosticsWorkspace';

const DIAGNOSTICS_TAB_REQUESTED_EVENT = 'diagnostics-tab-requested';

export function DiagnosticsWindowApp() {
  const [requestedTab, setRequestedTab] = useState<DiagnosticsTab>('events');

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<string>(DIAGNOSTICS_TAB_REQUESTED_EVENT, event => {
      if (!disposed && isDiagnosticsTab(event.payload)) setRequestedTab(event.payload);
    }).then(stop => {
      if (disposed) stop();
      else unlisten = stop;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  return (
    <div className="flex h-screen min-h-0 flex-col overflow-hidden bg-background text-on-surface font-[-apple-system,BlinkMacSystemFont,'Segoe_UI',Roboto,sans-serif]">
      <WindowHeader contextLabel="Diagnostics" />
      <main className="min-h-0 flex-1 overflow-hidden">
        <DiagnosticsWorkspace
          requestedTab={requestedTab}
          storeHealthEnabled
          canArmPrivateCapture={false}
        />
      </main>
    </div>
  );
}
