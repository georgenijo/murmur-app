import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import {
  canRetryDelivery,
  isDeliveryRetryResult,
  retryLastDelivery,
  setPasteLastShortcut,
} from './deliveryRecovery';

describe('delivery recovery commands', () => {
  beforeEach(() => invoke.mockReset());

  it('preserves the backend empty-history result', async () => {
    invoke.mockResolvedValue({ kind: 'empty', message: 'Nothing to paste yet.' });
    await expect(retryLastDelivery()).resolves.toEqual({
      kind: 'empty',
      message: 'Nothing to paste yet.',
    });
    expect(invoke).toHaveBeenCalledWith('retry_last_delivery');
  });

  it('configures or disables the selected global shortcut explicitly', async () => {
    invoke.mockResolvedValue(undefined);
    await setPasteLastShortcut('command_option_v');
    await setPasteLastShortcut(null);
    expect(invoke).toHaveBeenNthCalledWith(1, 'set_paste_last_shortcut', {
      shortcut: 'command_option_v',
    });
    expect(invoke).toHaveBeenNthCalledWith(2, 'set_paste_last_shortcut', {
      shortcut: null,
    });
  });

  it('validates native feedback and keeps retry available only for recoverable outcomes', () => {
    expect(isDeliveryRetryResult({ kind: 'auto_pasted', message: 'Pasted.' })).toBe(true);
    expect(isDeliveryRetryResult({ kind: 'clipboard_only', message: 'Copied.' })).toBe(true);
    expect(isDeliveryRetryResult({ kind: 'unknown', message: 'Nope.' })).toBe(false);
    expect(isDeliveryRetryResult({ kind: 'failed', message: 7 })).toBe(false);

    expect(canRetryDelivery({ kind: 'clipboard_only' })).toBe(true);
    expect(canRetryDelivery({ kind: 'busy' })).toBe(true);
    expect(canRetryDelivery({ kind: 'failed' })).toBe(true);
    expect(canRetryDelivery({ kind: 'auto_pasted' })).toBe(false);
    expect(canRetryDelivery({ kind: 'empty' })).toBe(false);
  });
});
