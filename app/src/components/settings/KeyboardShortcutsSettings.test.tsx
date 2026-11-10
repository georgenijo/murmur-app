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
  it('shows retained shortcuts without removed rewrite or correction controls', async () => {
    await render(DEFAULT_SETTINGS);
    expect(container.textContent).not.toContain('Voice Query');
    expect(container.textContent).toContain('Paste last or retry delivery');
    expect(container.textContent).not.toMatch(/Selected-text|Correct last dictation/i);
  });

  it('keeps Hold outside the keycap and routes Configure to its owner', async () => {
    await render({ ...DEFAULT_SETTINGS, recordingMode: 'hold_down', doubleTapKey: 'shift_l' });
    const row = container.querySelector('[data-shortcut-id="recording"]')!;
    expect(row.querySelector('.settings-shortcut-hold')?.textContent).toBe('Hold');
    expect(row.querySelector('kbd')?.textContent).toBe('⇧ Left Shift');
    const button = row.querySelector('button')!;
    expect(button.getAttribute('aria-label')).toBe('Configure in Recording');
    await act(async () => button.click());
    expect(onOpenOwner).toHaveBeenCalledWith({ shortcutId: 'recording', page: 'recording', target: 'trigger-key' });
  });

  it('keeps paused and unassigned shortcuts explicit without redundant On badges', async () => {
    await render({ ...DEFAULT_SETTINGS, disabled: true, pasteLastShortcut: null });
    expect(container.querySelector('[data-shortcut-id="recording"] .settings-shortcut-state')?.textContent).toBe('Paused');
    expect(container.querySelector('[data-shortcut-id="paste-last"] .settings-shortcut-state')?.textContent).toBe('Off');
    expect(container.querySelector('[data-shortcut-id="paste-last"] kbd')).toBeNull();
    expect(container.querySelectorAll('ul[aria-label="Main-window shortcuts"] kbd')).toHaveLength(4);
  });

});
