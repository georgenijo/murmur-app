import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_SETTINGS, type Settings } from '../settings';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  emit: vi.fn(async () => {}),
  listen: vi.fn(async () => () => {}),
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
      <button onClick={mirror.handleToggleAutoPaste}>Paste</button>
      <button onClick={mirror.handleToggleDisabled}>Disable</button>
      <button onClick={mirror.refresh}>Refresh</button>
    </>;
  }

  beforeEach(async () => {
    vi.clearAllMocks();
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

  it('rolls back only auto-paste after main revokes consent during configuration', async () => {
    let fail: (reason: Error) => void = () => { throw new Error('No pending request'); };
    mocks.invoke.mockImplementation(() => new Promise((_, reject) => { fail = reject; }));
    await act(async () => container.querySelector('button')?.click());
    expect(stored.autoPaste).toBe(true);
    revokeInMain();
    await act(async () => fail(new Error('configuration refused')));
    expect(stored.autoPaste).toBe(false);
    expectRevocationPreserved();
    expect(mocks.emit).toHaveBeenCalledWith('settings-changed');
  });

  it('merges disabled success into current settings after a pending backend reply', async () => {
    let finish: () => void = () => { throw new Error('No pending request'); };
    mocks.invoke.mockImplementation(() => new Promise<void>((resolve) => { finish = resolve; }));
    await act(async () => container.querySelectorAll('button')[1]?.click());
    revokeInMain();
    await act(async () => finish());
    expect(stored.disabled).toBe(true);
    expectRevocationPreserved();
  });

  it('does not write a stale snapshot when disabling fails', async () => {
    let fail: (reason: Error) => void = () => { throw new Error('No pending request'); };
    mocks.invoke.mockImplementation(() => new Promise((_, reject) => { fail = reject; }));
    await act(async () => container.querySelectorAll('button')[1]?.click());
    revokeInMain();
    await act(async () => fail(new Error('backend refused')));
    expect(stored.disabled).toBe(false);
    expectRevocationPreserved();
    expect(mocks.saveSettings).not.toHaveBeenCalled();
  });

});
