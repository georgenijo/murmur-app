import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_SETTINGS, type VocabularyEntry } from '../settings';

const mocks = vi.hoisted(() => ({
  configure: vi.fn(),
  emit: vi.fn<(event: string) => Promise<void>>(async () => {}),
  listen: vi.fn(async () => () => {}),
  invoke: vi.fn<(command?: string, args?: unknown) => Promise<unknown>>(async () => undefined),
  isEnabled: vi.fn(async () => false),
  enable: vi.fn(async () => {}),
  disable: vi.fn(async () => {}),
  isTauri: vi.fn(() => false),
}));

vi.mock('../dictation', () => ({
  configure: mocks.configure,
  buildConfigureOptions: vi.fn((settings) => settings),
}));
vi.mock('@tauri-apps/api/event', () => ({ emit: mocks.emit, listen: mocks.listen }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke, isTauri: mocks.isTauri }));
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
    mocks.isTauri.mockReturnValue(false);
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

  it('does not restore stale microphone approvals from overlay quick controls', async () => {
    await mountHarness();
    await act(async () => current.updateSettings({
      smartAutoMicrophoneEnabled: true,
      smartAutoProbeEnabled: true,
      smartAutoApprovedDeviceIds: ['new-approved'],
      smartAutoPreferredDeviceIds: ['new-approved'],
    }));
    await act(async () => current.applyExternalSettings({
      ...DEFAULT_SETTINGS,
      disabled: true,
      smartAutoApprovedDeviceIds: ['removed-approved'],
    }));
    expect(current.settings.disabled).toBe(true);
    expect(current.settings.smartAutoProbeEnabled).toBe(true);
    expect(current.settings.smartAutoApprovedDeviceIds).toEqual(['new-approved']);
    expect(current.settings.smartAutoPreferredDeviceIds).toEqual(['new-approved']);
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
            schemaVersion: 2, revision: 1, status: 'available', defaultInputId: null, lidState: 'open', errorCode: null, devices: [
            { id: 'raw-coreaudio-built-in', name: 'Built-in Mic', kind: 'builtIn', connected: true, hasInput: true },
            { id: 'raw-coreaudio-studio', name: 'Studio Mic', kind: 'external', connected: true, hasInput: true },
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
            schemaVersion: 2, revision: 1, status: 'available', defaultInputId: null, lidState: 'open', errorCode: null, devices: [
            { id: 'raw-coreaudio-studio-a', name: 'Studio Mic', kind: 'external', connected: true, hasInput: true },
            { id: 'raw-coreaudio-studio-b', name: 'Studio Mic', kind: 'external', connected: true, hasInput: true },
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
            schemaVersion: 2,
            revision: 2,
            status: 'stale',
            devices: [{ id: 'raw-coreaudio-studio', name: 'Studio Mic', kind: 'external', connected: true, hasInput: true }],
            defaultInputId: null,
            lidState: 'open',
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
            schemaVersion: 2,
            revision: 3,
            status: 'available',
            devices: [{ id: 'opaque uid', name: 'Studio Mic', kind: 'external', connected: true, hasInput: true }],
            defaultInputId: 'opaque uid',
            lidState: 'open',
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

  it('notifies the overlay when the recording gesture or function key changes', async () => {
    await mountHarness();
    mocks.emit.mockClear();

    await act(async () => current.updateSettings({ doubleTapKey: 'f7' }));
    await act(async () => current.updateSettings({ recordingMode: 'both' }));

    expect(mocks.emit.mock.calls.filter(([event]) => event === 'settings-changed')).toHaveLength(2);
  });

  it('serializes probe policy writes and reads the latest desired policy after a pending enable', async () => {
    mocks.isTauri.mockReturnValue(true);
    mocks.invoke.mockResolvedValue(1);
    await mountHarness();
    mocks.invoke.mockClear();

    let finishEnable: (() => void) | null = null;
    mocks.invoke.mockImplementation((command?: string) => {
      if (command === 'configure_smart_auto_probe' && finishEnable === null) {
        return new Promise<number>((resolve) => {
          finishEnable = () => resolve(2);
        });
      }
      if (command === 'cancel_audio_initialization') return Promise.resolve(undefined);
      return Promise.resolve(3);
    });

    await act(async () => {
      current.updateSettings({
        smartAutoMicrophoneEnabled: true,
        smartAutoProbeEnabled: true,
        smartAutoApprovedDeviceIds: ['usb-a'],
        smartAutoPreferredDeviceIds: ['usb-a'],
      });
      await Promise.resolve();
    });
    current.updateSettings({
      smartAutoApprovedDeviceIds: ['usb-b'],
      smartAutoPreferredDeviceIds: ['usb-b'],
    });
    current.updateSettings({
      microphone: 'usb-b',
      smartAutoMicrophoneEnabled: false,
      smartAutoProbeEnabled: false,
    });

    expect(mocks.invoke.mock.calls.filter(([command]) => command === 'configure_smart_auto_probe')).toHaveLength(1);
    await act(async () => {
      finishEnable?.();
      await Promise.resolve();
      await Promise.resolve();
    });

    const writes = mocks.invoke.mock.calls.filter(([command]) => command === 'configure_smart_auto_probe');
    expect(writes).toHaveLength(2);
    expect(writes[0]?.[1]).toEqual({
      policy: {
        enabled: true,
        request: {
          approvedDeviceIds: ['usb-a'],
          preferredDeviceIds: ['usb-a'],
          allowContinuity: false,
          requireRecentSignal: true,
        },
      },
    });
    expect(writes[1]?.[1]).toEqual({ policy: { enabled: false } });
    expect(mocks.emit).toHaveBeenCalledWith('settings-changed');
  });

  it('keeps saved disabled consent inert at boot and synchronizes a native re-enable', async () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      disabled: true,
      smartAutoMicrophoneEnabled: true,
      smartAutoProbeEnabled: true,
      smartAutoApprovedDeviceIds: ['usb'],
      smartAutoPreferredDeviceIds: ['usb'],
    }));
    mocks.isTauri.mockReturnValue(true);
    mocks.invoke.mockResolvedValue(1);
    await mountHarness();
    expect(mocks.invoke.mock.calls.filter(([command]) => command === 'configure_smart_auto_probe').map(([, args]) => args))
      .toEqual([{ policy: { enabled: false } }]);
    await act(async () => current.applyExternalSettings({ ...current.settings, disabled: false }));
    expect(mocks.invoke.mock.calls.filter(([command]) => command === 'configure_smart_auto_probe').slice(-1)[0]?.[1])
      .toEqual({ policy: { enabled: true, request: { approvedDeviceIds: ['usb'], preferredDeviceIds: ['usb'], allowContinuity: false, requireRecentSignal: true } } });
    await act(async () => current.updateSettings({ disabled: true }));
    expect(mocks.invoke.mock.calls.filter(([command]) => command === 'configure_smart_auto_probe').slice(-1)[0]?.[1])
      .toEqual({ policy: { enabled: false } });
  });

  it('fails closed and clears consent when probe configuration cannot be enabled', async () => {
    mocks.isTauri.mockReturnValue(true);
    mocks.invoke.mockResolvedValue(1);
    await mountHarness();
    mocks.invoke.mockImplementation((command?: string, args?: unknown) => (
      command === 'configure_smart_auto_probe' && JSON.stringify(args).includes('"enabled":true')
        ? Promise.reject(new Error('invalid probe policy'))
        : Promise.resolve(2)
    ));

    await act(async () => {
      current.updateSettings({
        smartAutoMicrophoneEnabled: true,
        smartAutoProbeEnabled: true,
        smartAutoApprovedDeviceIds: ['usb'],
      });
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(current.settings.smartAutoProbeEnabled).toBe(false);
    expect(current.configureError).toContain('could not be enabled');
    const writes = mocks.invoke.mock.calls.filter(([command]) => command === 'configure_smart_auto_probe');
    expect(writes[writes.length - 1]?.[1]).toEqual({ policy: { enabled: false } });
  });
});
