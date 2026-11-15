import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { MainThreadWatchStatus } from '../../lib/mainThreadWatch';
import { MainThreadWatchControl } from './MainThreadWatchControl';

const getStatus = vi.hoisted(() => vi.fn<() => Promise<MainThreadWatchStatus>>());
const setArmed = vi.hoisted(() => vi.fn<(armed: boolean) => Promise<MainThreadWatchStatus>>());

vi.mock('../../lib/mainThreadWatch', () => ({
  getMainThreadWatchStatus: getStatus,
  setMainThreadWatchArmed: setArmed,
}));

const OFF: MainThreadWatchStatus = { setting: false, running: false, forcedByEnv: false };

describe('MainThreadWatchControl', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    getStatus.mockResolvedValue(OFF);
    setArmed.mockImplementation(async armed => ({ setting: armed, running: armed, forcedByEnv: false }));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.clearAllMocks();
  });

  async function render() {
    await act(async () => {
      root.render(<MainThreadWatchControl />);
    });
  }

  function watchSwitch(): HTMLElement {
    const control = container.querySelector<HTMLElement>('[role="switch"][aria-label="Main-thread watch"]');
    expect(control).not.toBeNull();
    return control!;
  }

  it('shows one labelled control, off by default, with a one-line cost note', async () => {
    await render();
    expect(container.querySelectorAll('[role="switch"]')).toHaveLength(1);
    expect(container.textContent).toContain('Main-thread watch');
    expect(container.textContent).toContain('Uses extra CPU and log volume while on');
    expect(watchSwitch().getAttribute('aria-checked')).toBe('false');
    expect(watchSwitch().hasAttribute('data-disabled')).toBe(false);
  });

  it('arms and disarms through the native command and shows the running state', async () => {
    await render();
    await act(async () => { watchSwitch().click(); });
    expect(setArmed).toHaveBeenLastCalledWith(true);
    expect(watchSwitch().getAttribute('aria-checked')).toBe('true');

    await act(async () => { watchSwitch().click(); });
    expect(setArmed).toHaveBeenLastCalledWith(false);
    expect(watchSwitch().getAttribute('aria-checked')).toBe('false');
  });

  it('cannot turn off a watch the environment armed', async () => {
    getStatus.mockResolvedValue({ setting: false, running: true, forcedByEnv: true });
    await render();
    expect(watchSwitch().getAttribute('aria-checked')).toBe('true');
    expect(watchSwitch().hasAttribute('data-disabled')).toBe(true);
    expect(container.textContent).toContain('MURMUR_MAIN_THREAD_WATCH');
  });

  it('reports a failed change and stays off', async () => {
    setArmed.mockRejectedValueOnce(new Error('boom'));
    await render();
    await act(async () => { watchSwitch().click(); });
    expect(container.querySelector('[role="alert"]')?.textContent)
      .toBe('Murmur could not change the main-thread watch.');
    expect(watchSwitch().getAttribute('aria-checked')).toBe('false');
  });

  it('stays disabled when the status cannot be read', async () => {
    getStatus.mockRejectedValueOnce(new Error('no'));
    await render();
    expect(watchSwitch().hasAttribute('data-disabled')).toBe(true);
    expect(container.querySelector('[role="alert"]')).not.toBeNull();
  });
});
