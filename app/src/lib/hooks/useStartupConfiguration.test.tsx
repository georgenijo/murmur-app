import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { VocabularyEntry } from '../settings';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
  isEnabled: vi.fn(async () => false),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke, isTauri: () => false }));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen, emit: mocks.emit }));
vi.mock('@tauri-apps/plugin-autostart', () => ({
  isEnabled: mocks.isEnabled, enable: vi.fn(), disable: vi.fn(),
}));

import { useSettings } from './useSettings';
import { useInitialization } from './useInitialization';

function deferred<T>() {
  let resolve = (_value: T) => {};
  let reject = (_reason: Error) => {};
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

describe('startup vocabulary configuration', () => {
  let root: Root;
  let container: HTMLDivElement;

  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  it('initializes from the rolled-back settings after a pending invalid alias edit', async () => {
    const init = deferred<{ type: string }>();
    const invalidEdit = deferred<{ type: string }>();
    const writes: Array<{ vocabularyEntries?: VocabularyEntry[] }> = [];
    mocks.invoke.mockImplementation((command: string, args?: { options?: { vocabularyEntries?: VocabularyEntry[] } }) => {
      if (command === 'init_dictation') return init.promise;
      if (command === 'configure_dictation') {
        writes.push(args?.options ?? {});
        return writes.length === 1 ? invalidEdit.promise : Promise.resolve({ type: 'configured' });
      }
      return Promise.resolve(undefined);
    });
    vi.spyOn(console, 'error').mockImplementation(() => {});

    let settingsState!: ReturnType<typeof useSettings>;
    let initialization!: ReturnType<typeof useInitialization>;
    function Harness() {
      settingsState = useSettings();
      initialization = useInitialization(settingsState.getCurrentSettings);
      return null;
    }
    await act(async () => root.render(<Harness />));

    const entry: VocabularyEntry = {
      id: 'term', written: 'Tauri', aliases: ['Tory'], enabled: true,
      scope: { kind: 'global' },
    };
    await act(async () => {
      settingsState.updateSettings({ vocabularyEntries: [entry], customVocabulary: 'Tauri' });
      await Promise.resolve();
    });
    expect(writes).toHaveLength(1);
    expect(writes[0].vocabularyEntries).toEqual([entry]);

    await act(async () => {
      init.resolve({ type: 'initialized' });
      await Promise.resolve();
      invalidEdit.reject(new Error('invalid alias'));
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(settingsState.settings.vocabularyEntries).toEqual([]);
    expect(writes).toHaveLength(2);
    expect(writes[1].vocabularyEntries).toEqual([]);
    expect(initialization.initialized).toBe(true);
    expect(initialization.error).toBe('');
  });
});
