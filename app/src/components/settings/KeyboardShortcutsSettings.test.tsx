import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_SETTINGS, type Settings } from '../../lib/settings';
import {
  KeyboardShortcutsSettings,
  type ShortcutOwnerDestination,
} from './KeyboardShortcutsSettings';

describe('KeyboardShortcutsSettings', () => {
  let container: HTMLDivElement;
  let root: Root;
  const onOpenOwner = vi.fn<(destination: ShortcutOwnerDestination) => void>();

  async function render(settings: Settings) {
    await act(async () => root.render(
      <KeyboardShortcutsSettings
        settings={settings}
        activePage="shortcuts"
        onOpenOwner={onOpenOwner}
      />,
    ));
  }

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    onOpenOwner.mockReset();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('renders every global and main-window shortcut with its live state', async () => {
    await render({
      ...DEFAULT_SETTINGS,
      transformHoldKey: 'alt_r',
      queryHotkey: 'ctrl_l',
      pasteLastShortcut: 'command_option_v',
    });

    const globalRows = Array.from(container.querySelectorAll<HTMLLIElement>('[aria-label="Global shortcuts"] > li'));
    expect(globalRows).toHaveLength(5);
    expect(globalRows.map((row) => row.textContent)).toEqual([
      expect.stringMatching(/Recording trigger.*On.*Hold ⇧ Left Shift.*Configure in Recording/),
      expect.stringMatching(/Selected-text transform.*On.*Hold Right Option.*Configure in Selected-Text Rewrite/),
      expect.stringMatching(/Voice Query.*On.*Double-tap Left Control.*Configure in Voice Query/),
      expect.stringMatching(/Paste Last \/ Retry Delivery.*On.*⌘⌥V.*Configure in Delivery/),
      expect.stringMatching(/Correct last dictation.*Off.*⌘⇧E.*Enable in Selected-Text Rewrite/),
    ]);

    const windowRows = Array.from(container.querySelectorAll<HTMLLIElement>('[aria-label="Main-window shortcuts"] > li'));
    expect(windowRows).toHaveLength(4);
    expect(windowRows.map((row) => row.textContent)).toEqual([
      expect.stringMatching(/Command palette.*On.*⌘K/),
      expect.stringMatching(/Search transcripts.*On.*⌘F/),
      expect.stringMatching(/Open Settings.*On.*⌘,/),
      expect.stringMatching(/Open Performance Lab.*On.*⌘L/),
    ]);
  });

  it('reflects a changed transform binding on rerender without remounting', async () => {
    await render({ ...DEFAULT_SETTINGS, transformHoldKey: 'alt_r' });
    const row = container.querySelector('[data-shortcut-id="transform"]') as HTMLLIElement;
    expect(row.textContent).toContain('Hold Right Option');

    await render({ ...DEFAULT_SETTINGS, transformHoldKey: 'shift_r' });
    expect(row.isConnected).toBe(true);
    expect(row.textContent).toContain('Hold Right Shift');
    expect(row.textContent).not.toContain('Right Option');
  });

  it('keeps configured bindings visible and reports native availability while Murmur is disabled', async () => {
    await render({
      ...DEFAULT_SETTINGS,
      disabled: true,
      transformHoldKey: 'alt_r',
      queryHotkey: 'ctrl_l',
      pasteLastShortcut: 'command_option_v',
      correctionShortcutEnabled: true,
    });

    const rowText = (id: string) => container.querySelector(`[data-shortcut-id="${id}"]`)?.textContent;
    expect(rowText('recording')).toMatch(/Paused.*Hold ⇧ Left Shift.*View in Recording/);
    expect(rowText('transform')).toMatch(/Paused.*Hold Right Option.*View in Selected-Text Rewrite/);
    expect(rowText('voice-query')).toMatch(/Paused.*Double-tap Left Control.*View in Voice Query/);
    expect(rowText('paste-last')).toMatch(/On.*⌘⌥V.*Configure in Delivery/);
    expect(rowText('correction')).toMatch(/Paused.*⌘⇧E.*View in Selected-Text Rewrite/);
    expect(container.querySelector('[role="status"]')?.textContent).toContain('Paste Last remains available');
  });

  it('routes each configurable row through its typed owner destination', async () => {
    await render(DEFAULT_SETTINGS);
    const correction = container.querySelector<HTMLButtonElement>('[data-shortcut-id="correction"] button');
    await act(async () => correction?.click());

    expect(onOpenOwner).toHaveBeenCalledWith({
      shortcutId: 'correction',
      page: 'ai-transform',
      target: 'correction-shortcut',
    });
  });
});
