import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_SETTINGS, type Settings } from '../settings';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  emit: vi.fn(async () => {}),
  listen: vi.fn(async (_name: string, _handler: (event: { payload: unknown }) => void) => () => {}),
  loadSettings: vi.fn<() => Settings>(),
  saveSettings: vi.fn<(settings: Settings) => void>(),
  setDisabled: vi.fn(),
  setShowHotkeyMiss: vi.fn(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke, isTauri: () => false }));
vi.mock('@tauri-apps/api/event', () => ({ emit: mocks.emit, listen: mocks.listen }));
vi.mock('../log', () => ({ flog: { error: vi.fn(), warn: vi.fn() } }));
vi.mock('../settings', async (importOriginal) => ({
  ...await importOriginal<typeof import('../settings')>(),
  loadSettings: mocks.loadSettings,
  saveSettings: mocks.saveSettings,
}));

import { useOverlaySettingsMirror } from './useOverlaySettingsMirror';

describe('overlay settings write ownership', () => {
  let root: Root;
  let container: HTMLDivElement;
  let stored: Settings;
  const hotkeyMissFeedbackRef = { current: false };

  function Harness() {
    const mirror = useOverlaySettingsMirror({
      setDisabled: mocks.setDisabled,
      setShowHotkeyMiss: mocks.setShowHotkeyMiss,
      hotkeyMissFeedbackRef,
    });
    return <>
      <output>{mirror.overlayPlacement}|{mirror.overlayProcessingStyle}</output>
      <button onClick={mirror.handleToggleAutoPaste}>Paste</button>
      <button onClick={mirror.handleToggleDisabled}>Disable</button>
      <button onClick={mirror.refresh}>Refresh</button>
    </>;
  }

  beforeEach(async () => {
    vi.clearAllMocks();
    mocks.invoke.mockReset().mockResolvedValue(undefined);
    mocks.listeners.clear();
    mocks.listen.mockImplementation(async (name, handler) => {
      mocks.listeners.set(name, handler); return () => { mocks.listeners.delete(name); };
    });
    stored = {
      ...DEFAULT_SETTINGS,
      autoPaste: false,
      disabled: false,
      smartAutoMicrophoneEnabled: true,
      smartAutoProbeEnabled: true,
      smartAutoApprovedDeviceIds: ['old'],
      smartAutoPreferredDeviceIds: ['old'],
    };
    mocks.loadSettings.mockImplementation(() => stored);
    mocks.saveSettings.mockImplementation((settings) => { stored = settings; });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<Harness />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  function revokeInMain() {
    stored = { ...stored, smartAutoProbeEnabled: false,
      smartAutoApprovedDeviceIds: ['new'], smartAutoPreferredDeviceIds: ['new'] };
  }

  function expectRevocationPreserved() {
    expect(stored.smartAutoProbeEnabled).toBe(false);
    expect(stored.smartAutoApprovedDeviceIds).toEqual(['new']);
    expect(stored.smartAutoPreferredDeviceIds).toEqual(['new']);
  }

  it('does not persist when auto-paste configuration fails after main revokes consent', async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === 'configure_dictation') { revokeInMain(); throw new Error('refused'); }
      if (command === 'load_settings_blob') return JSON.stringify(stored);
    });
    await act(async () => container.querySelector('button')?.click());
    expect(stored.autoPaste).toBe(false);
    expectRevocationPreserved();
    expect(mocks.saveSettings).not.toHaveBeenCalled();
    expect(mocks.invoke.mock.calls.some(([command]) => command === 'patch_overlay_setting')).toBe(false);
    expect(mocks.emit).not.toHaveBeenCalledWith('settings-changed');
  });

  for (const [index, field] of [[0, 'autoPaste'], [1, 'disabled']] as const) {
    it(`preserves committed opt-out/style while a stale refresh is pending during ${field}`, async () => {
      const cached = { ...stored, overlayLiveWords: true };
      stored = cached;
      mocks.invoke.mockResolvedValue(JSON.stringify(cached));
      await act(async () => mocks.listeners.get('overlay-settings-persisted')?.({ payload: null }));
      let finishOld!: (value: string) => void;
      mocks.invoke.mockImplementationOnce(() => new Promise(resolve => { finishOld = resolve; }));
      await act(async () => mocks.listeners.get('overlay-settings-persisted')?.({ payload: null }));
      stored = { ...stored, overlayLiveWords: false, overlayProcessingStyle: 'pulse',
        smartAutoProbeEnabled: false, smartAutoApprovedDeviceIds: ['new'], smartAutoPreferredDeviceIds: ['new'] };
      mocks.invoke.mockImplementation(async (command: string, args?: { field: 'autoPaste' | 'disabled'; value: boolean }) => {
        if (command === 'patch_overlay_setting') stored = { ...stored, [args!.field]: args!.value };
        if (command === 'patch_overlay_setting' || command === 'load_settings_blob') return JSON.stringify(stored);
      });
      await act(async () => container.querySelectorAll('button')[index].click());
      await act(async () => finishOld(JSON.stringify(cached)));
      expect(stored[field]).toBe(true);
      expect(stored.overlayLiveWords).toBe(false);
      expect(stored.overlayProcessingStyle).toBe('pulse');
      expect(container.querySelector('output')?.textContent).toContain('|pulse');
      expectRevocationPreserved();
      expect(mocks.invoke).toHaveBeenCalledWith('patch_overlay_setting', { field, value: true });
      expect(mocks.saveSettings).not.toHaveBeenCalled();
      expect(mocks.emit).not.toHaveBeenCalledWith('settings-changed');
    });
  }

  it('does not write a stale snapshot when disabling fails', async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === 'set_app_disabled') { revokeInMain(); throw new Error('refused'); }
      if (command === 'load_settings_blob') return JSON.stringify(stored);
    });
    await act(async () => container.querySelectorAll('button')[1]?.click());
    expect(stored.disabled).toBe(false);
    expectRevocationPreserved();
    expect(mocks.saveSettings).not.toHaveBeenCalled();
  });

  it('drops an old durable read after a newer placement/style commit', async () => {
    let older!: (blob: string) => void, newer!: (blob: string) => void;
    mocks.invoke.mockImplementationOnce(() => new Promise(resolve => { older = resolve; }));
    await act(async () => mocks.listeners.get('overlay-settings-persisted')?.({ payload: null }));
    mocks.invoke.mockImplementationOnce(() => new Promise(resolve => { newer = resolve; }));
    await act(async () => mocks.listeners.get('overlay-settings-persisted')?.({ payload: null }));
    await act(async () => {
      newer(JSON.stringify({ ...stored, settingsVersion: 6, overlayPlacement: 'below_menu_bar', overlayProcessingStyle: 'pulse' }));
      older(JSON.stringify({ ...stored, settingsVersion: 6, overlayPlacement: 'notch', overlayProcessingStyle: 'sweep' }));
    });
    expect(container.querySelector('output')?.textContent).toBe('below_menu_bar|pulse');
  });

});
