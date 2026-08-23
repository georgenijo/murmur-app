import { invoke } from '@tauri-apps/api/core';
import type { PasteLastShortcut } from './settings';

export interface DeliveryRetryResult {
  kind: 'auto_pasted' | 'clipboard_only' | 'empty' | 'busy' | 'failed';
  message: string;
}

export async function retryLastDelivery(): Promise<DeliveryRetryResult> {
  return invoke('retry_last_delivery');
}

export async function setPasteLastShortcut(shortcut: PasteLastShortcut | null): Promise<void> {
  await invoke('set_paste_last_shortcut', { shortcut });
}
