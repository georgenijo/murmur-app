import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PauseBrowserMediaSetting } from './PauseBrowserMediaSetting';
import { DEFAULT_SETTINGS } from '../../lib/settings';
import { buildConfigureOptions } from '../../lib/dictation';

const mocks = vi.hoisted(() => ({
  get: vi.fn(),
  request: vi.fn(),
  openPreferences: vi.fn(),
}));

vi.mock('../../lib/browserMedia', () => ({
  getBrowserMediaPermission: mocks.get,
  requestBrowserMediaPermission: mocks.request,
}));
vi.mock('../../lib/meetings', () => ({
  openSystemAudioPreferences: mocks.openPreferences,
}));

describe('PauseBrowserMediaSetting', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    mocks.get.mockReset();
    mocks.request.mockReset();
    mocks.openPreferences.mockReset();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  const toggle = () => container.querySelector<HTMLElement>('[role="switch"], button[aria-label]')!;

  it('is off by default and is sent to the backend', () => {
    expect(DEFAULT_SETTINGS.pauseBrowserMedia).toBe(false);
    expect(buildConfigureOptions(DEFAULT_SETTINGS).pauseBrowserMedia).toBe(false);
    expect(buildConfigureOptions({ ...DEFAULT_SETTINGS, pauseBrowserMedia: true }).pauseBrowserMedia).toBe(true);
  });

  it('never asks for permission while off', async () => {
    const onEnabledChange = vi.fn();
    await act(async () => root.render(<PauseBrowserMediaSetting enabled={false} onEnabledChange={onEnabledChange} />));
    expect(mocks.get).not.toHaveBeenCalled();
    expect(mocks.request).not.toHaveBeenCalled();
    expect(container.textContent).not.toContain('System Audio Recording');
  });

  it('requests permission only when the user turns it on', async () => {
    const onEnabledChange = vi.fn();
    mocks.request.mockResolvedValue('denied');
    await act(async () => root.render(<PauseBrowserMediaSetting enabled={false} onEnabledChange={onEnabledChange} />));
    await act(async () => toggle().click());
    expect(onEnabledChange).toHaveBeenCalledWith(true);
    expect(mocks.request).toHaveBeenCalledTimes(1);
    mocks.get.mockResolvedValue('denied');
    await act(async () => root.render(<PauseBrowserMediaSetting enabled onEnabledChange={onEnabledChange} />));
    expect(container.textContent).toContain('Screen & System Audio Recording');
    const open = [...container.querySelectorAll('button')].find((button) => button.textContent === 'Open Settings');
    await act(async () => open!.click());
    expect(mocks.openPreferences).toHaveBeenCalledTimes(1);
  });

  it('reads consent without prompting when already on, and turning off never requests', async () => {
    const onEnabledChange = vi.fn();
    mocks.get.mockResolvedValue('granted');
    await act(async () => root.render(<PauseBrowserMediaSetting enabled onEnabledChange={onEnabledChange} />));
    expect(mocks.get).toHaveBeenCalled();
    expect(container.textContent).toContain('System Audio Recording is allowed.');
    await act(async () => toggle().click());
    expect(onEnabledChange).toHaveBeenCalledWith(false);
    expect(mocks.request).not.toHaveBeenCalled();
  });
});
