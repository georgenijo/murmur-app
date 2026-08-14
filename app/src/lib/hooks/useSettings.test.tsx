import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_SETTINGS, type VocabularyEntry } from '../settings';

const mocks = vi.hoisted(() => ({
  configure: vi.fn(),
  emit: vi.fn(async () => {}),
  listen: vi.fn(async () => () => {}),
  invoke: vi.fn(async (_command?: string): Promise<unknown> => undefined),
  isEnabled: vi.fn(async () => false),
  enable: vi.fn(async () => {}),
  disable: vi.fn(async () => {}),
}));

vi.mock('../dictation', () => ({
  configure: mocks.configure,
  buildConfigureOptions: vi.fn((settings) => settings),
}));
vi.mock('@tauri-apps/api/event', () => ({ emit: mocks.emit, listen: mocks.listen }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke, isTauri: () => false }));
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

  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    mocks.configure.mockResolvedValue(undefined);
    mocks.invoke.mockResolvedValue(undefined);
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  async function mountHarness() {
    function Harness() {
      current = useSettings();
      return null;
    }

    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  it('restores UI state and never logs alias-bearing backend validation text', async () => {
    await mountHarness();
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

  it('migrates a unique legacy microphone name during app settings initialization', async () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      microphone: 'Studio Mic',
      microphoneIdMigrationComplete: false,
    }));
    mocks.invoke.mockImplementation(async (command?: string) => (
      command === 'get_audio_input_inventory'
        ? {
            schemaVersion: 1, revision: 1, status: 'available', defaultInputId: null, errorCode: null, devices: [
            { id: 'raw-coreaudio-built-in', name: 'Built-in Mic' },
            { id: 'raw-coreaudio-studio', name: 'Studio Mic' },
            ],
          }
        : undefined
    ));

    await mountHarness();

    expect(current.settings.microphone).toBe('raw-coreaudio-studio');
    expect(current.settings.microphoneIdMigrationComplete).toBe(true);
    expect(
      JSON.parse(localStorage.getItem('dictation-settings') ?? '{}').microphone,
    ).toBe('raw-coreaudio-studio');
  });

  it('leaves an ambiguous legacy microphone unresolved for explicit reselection', async () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      microphone: 'Studio Mic',
      microphoneIdMigrationComplete: false,
    }));
    mocks.invoke.mockImplementation(async (command?: string) => (
      command === 'get_audio_input_inventory'
        ? {
            schemaVersion: 1, revision: 1, status: 'available', defaultInputId: null, errorCode: null, devices: [
            { id: 'raw-coreaudio-studio-a', name: 'Studio Mic' },
            { id: 'raw-coreaudio-studio-b', name: 'Studio Mic' },
            ],
          }
        : undefined
    ));

    await mountHarness();

    expect(current.settings.microphone).toBe('Studio Mic');
    expect(current.settings.microphoneIdMigrationComplete).toBe(false);
    expect(
      JSON.parse(localStorage.getItem('dictation-settings') ?? '{}').microphone,
    ).toBe('Studio Mic');
  });

  it('does not request inventory for the System Default sentinel', async () => {
    await mountHarness();
    expect(mocks.invoke).not.toHaveBeenCalledWith('get_audio_input_inventory');
  });

  it('does not migrate a legacy display name from stale topology', async () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      microphone: 'Studio Mic',
      microphoneIdMigrationComplete: false,
    }));
    mocks.invoke.mockImplementation(async (command?: string) => (
      command === 'get_audio_input_inventory'
        ? {
            schemaVersion: 1,
            revision: 2,
            status: 'stale',
            devices: [{ id: 'raw-coreaudio-studio', name: 'Studio Mic' }],
            defaultInputId: null,
            errorCode: 'refreshPending',
          }
        : undefined
    ));

    await mountHarness();
    expect(current.settings.microphone).toBe('Studio Mic');
    expect(current.settings.microphoneIdMigrationComplete).toBe(false);
  });

  it('marks a previously stored raw UID complete after exact membership proof', async () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      microphone: 'opaque uid',
      microphoneIdMigrationComplete: false,
    }));
    mocks.invoke.mockImplementation(async (command?: string) => (
      command === 'get_audio_input_inventory'
        ? {
            schemaVersion: 1,
            revision: 3,
            status: 'available',
            devices: [{ id: 'opaque uid', name: 'Studio Mic' }],
            defaultInputId: 'opaque uid',
            errorCode: null,
          }
        : undefined
    ));
    await mountHarness();
    expect(current.settings.microphone).toBe('opaque uid');
    expect(current.settings.microphoneIdMigrationComplete).toBe(true);
  });

  it('pushes mirrorToNotchPill changes to the backend (regression: was missing from configure-trigger list)', async () => {
    await act(async () => {
      current.updateSettings({ mirrorToNotchPill: true });
      await Promise.resolve();
    });

    expect(mocks.configure).toHaveBeenCalled();
    // Indexed rather than `.at(-1)`: this repo's tsconfig target predates it.
    const calls = mocks.configure.mock.calls;
    const lastArg = calls[calls.length - 1]?.[0];
    expect(lastArg).toMatchObject({ mirrorToNotchPill: true });
  });
});
