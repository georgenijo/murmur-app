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
    expect(container.textContent).toContain('Voice Query');
    expect(container.textContent).toContain('Paste last or retry delivery');
    expect(container.textContent).not.toMatch(/Selected-text|Correct last dictation/i);
  });

});
