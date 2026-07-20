import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { VocabularyEntry } from '../settings';

const mocks = vi.hoisted(() => ({
  configure: vi.fn(),
  emit: vi.fn(async () => {}),
  listen: vi.fn(async () => () => {}),
  invoke: vi.fn(async () => {}),
  isEnabled: vi.fn(async () => false),
  enable: vi.fn(async () => {}),
  disable: vi.fn(async () => {}),
}));

vi.mock('../dictation', () => ({
  configure: mocks.configure,
  buildConfigureOptions: vi.fn((settings) => settings),
}));
vi.mock('@tauri-apps/api/event', () => ({ emit: mocks.emit, listen: mocks.listen }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/plugin-autostart', () => ({
  isEnabled: mocks.isEnabled,
  enable: mocks.enable,
  disable: mocks.disable,
}));

import { useSettings } from './useSettings';

type SettingsState = ReturnType<typeof useSettings>;

describe('useSettings configure rollback privacy', () => {
  let container: HTMLDivElement;
  let root: Root;
  let current: SettingsState;

  beforeEach(async () => {
    vi.clearAllMocks();
    localStorage.clear();
    mocks.configure.mockResolvedValue(undefined);
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);

    function Harness() {
      current = useSettings();
      return null;
    }

    await act(async () => root.render(<Harness />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  it('restores UI state and never logs alias-bearing backend validation text', async () => {
    const secret = 'private spoken customer alias';
    const entry: VocabularyEntry = {
      id: 'private-entry',
      written: 'PrivateCanonical',
      aliases: [secret],
      enabled: true,
      scope: { kind: 'global' },
    };
    mocks.configure.mockRejectedValueOnce(
      `Spoken alias '${secret}' is a Voice Command phrase.`,
    );
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});

    await act(async () => {
      current.updateSettings({
        customVocabulary: entry.written,
        vocabularyEntries: [entry],
      });
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(current.settings.vocabularyEntries).toEqual([]);
    expect(current.settings.customVocabulary).toBe('');
    expect(current.configureError).toContain('Previous settings were restored');
    expect(current.configureError).not.toContain(secret);
    expect(JSON.stringify(consoleError.mock.calls)).not.toContain(secret);
    expect(localStorage.getItem('dictation-settings')).not.toContain(secret);
  });
});
