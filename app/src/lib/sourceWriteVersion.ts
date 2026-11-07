import { invoke } from '@tauri-apps/api/core';
// A fresh renderer module session owns its own counter. Never persist either
// value: reloads and independent webviews must not reuse a generation namespace.
export const sourceSession = crypto.randomUUID();
let generation = 0;
export function nextSourceWriteVersion(): { sourceSession: string; intentGeneration: number } {
  return { sourceSession, intentGeneration: ++generation };
}

let registration: Promise<unknown> | undefined;
/** One handshake per renderer, shared by every source producer. Native retires
 * the previous session of this window before accepting writes from this one. */
export async function invokeSource<T>(command: string, args: Record<string, unknown>): Promise<T> {
  if (!registration) {
    registration = invoke('register_source_session', { sourceSession }).catch(error => {
      registration = undefined;
      throw error;
    });
  }
  await registration;
  return invoke<T>(command, args);
}
