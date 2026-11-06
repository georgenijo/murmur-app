import { listen as tauriListen, type EventCallback, type EventName, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';

/** Tauri's default Any listener also receives emits addressed to other windows.
 * Bind all clients to their own label so the native sink can enforce routing. */
export function listen<T>(event: EventName, handler: EventCallback<T>): Promise<UnlistenFn> {
  const label = getCurrentWebviewWindow().label;
  return tauriListen(event, handler, { target: { kind: 'WebviewWindow', label } });
}
