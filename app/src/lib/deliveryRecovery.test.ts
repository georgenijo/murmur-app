import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import { retryLastDelivery, setPasteLastShortcut } from './deliveryRecovery';

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

  it('configures the fixed global shortcut explicitly', async () => {
    invoke.mockResolvedValue(undefined);
    await setPasteLastShortcut(true);
    expect(invoke).toHaveBeenCalledWith('set_paste_last_shortcut', { enabled: true });
  });
});
