import { invoke } from '@tauri-apps/api/core';
import type { PasteLastShortcut } from './settings';

export interface DeliveryRetryResult {
  kind: 'auto_pasted' | 'clipboard_only' | 'empty' | 'busy' | 'failed';
  message: string;
}

export function isDeliveryRetryResult(value: unknown): value is DeliveryRetryResult {
  if (!value || typeof value !== 'object') return false;
  const candidate = value as Record<string, unknown>;
  return typeof candidate.message === 'string'
    && ['auto_pasted', 'clipboard_only', 'empty', 'busy', 'failed']
      .includes(candidate.kind as string);
}

export function canRetryDelivery(result: Pick<DeliveryRetryResult, 'kind'>): boolean {
  return result.kind === 'clipboard_only' || result.kind === 'busy' || result.kind === 'failed';
}

export async function retryLastDelivery(): Promise<DeliveryRetryResult> {
  return invoke('retry_last_delivery');
}

export async function setPasteLastShortcut(shortcut: PasteLastShortcut | null): Promise<void> {
  await invoke('set_paste_last_shortcut', { shortcut });
}
