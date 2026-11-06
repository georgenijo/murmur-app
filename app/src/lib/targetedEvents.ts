import { emitTo as tauriEmitTo, listen as tauriListen, type EventCallback, type EventName, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';

/** Tauri's default Any listener also receives emits addressed to other windows.
 * Bind all clients to their own label so the native sink can enforce routing. */
export function listen<T>(event: EventName, handler: EventCallback<T>): Promise<UnlistenFn> {
  const label = getCurrentWebviewWindow().label;
  return tauriListen(event, handler, { target: { kind: 'WebviewWindow', label } });
}

/** Named frontend notifications have no Rust/native subscribers. Tauri treats
 * a missing label as an empty recipient set; companion destruction is normal.
 * Swallow transport rejection too so a closing/absent window is always quiet. */
export async function sendTo<T>(label: 'main' | 'overlay' | 'dictation-preview' | 'diagnostics', event: EventName, payload?: T): Promise<void> {
  try {
    await tauriEmitTo(label, event, payload);
  } catch { /* A companion may disappear while IPC is in flight. */ }
}
