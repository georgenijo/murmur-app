import { runInNewContext } from 'node:vm';
import { describe, expect, it, vi } from 'vitest';
import source from '../../src-tauri/src/qa_clipboard.js?raw';

function install(available = true) {
  const original = { readText: vi.fn(), writeText: vi.fn() };
  const navigator = { clipboard: original as unknown as Clipboard };
  const invoke = vi.fn(async (command: string) => command.endsWith('read_text') ? 'synthetic private text' : undefined);
  runInNewContext(source, {
    navigator,
    window: { __TAURI_INTERNALS__: available ? { invoke } : undefined },
    Blob,
    ClipboardItem: class {
      constructor(readonly items: Record<string, Blob>) {}
    },
  });
  return { navigator, original, invoke };
}

describe('isolated WebKit clipboard bridge', () => {
  it('routes text reads and writes through the private native commands only', async () => {
    const { navigator, original, invoke } = install();
    await navigator.clipboard.writeText('synthetic copy');
    expect(invoke).toHaveBeenCalledWith('plugin:qa-clipboard|write_text', { text: 'synthetic copy' });
    expect(await navigator.clipboard.readText()).toBe('synthetic private text');
    expect(invoke).toHaveBeenCalledWith('plugin:qa-clipboard|read_text', undefined);
    expect(original.readText).not.toHaveBeenCalled();
    expect(original.writeText).not.toHaveBeenCalled();
    expect(Object.getOwnPropertyDescriptor(navigator, 'clipboard')).toMatchObject({ configurable: false, writable: false });
  });

  it('fails closed when the native bridge is unavailable or a non-text item is supplied', async () => {
    const { navigator, original } = install(false);
    await expect(navigator.clipboard.readText()).rejects.toThrow('Private clipboard unavailable');
    await expect(navigator.clipboard.writeText('synthetic')).rejects.toThrow('Private clipboard unavailable');
    await expect(navigator.clipboard.write([{ types: ['image/png'] } as unknown as ClipboardItem])).rejects.toThrow('Private clipboard supports one text item');
    expect(original.readText).not.toHaveBeenCalled();
    expect(original.writeText).not.toHaveBeenCalled();
  });
});
