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
});
