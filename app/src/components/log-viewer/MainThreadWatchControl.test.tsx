import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { MainThreadWatchStatus } from '../../lib/mainThreadWatch';
import { MainThreadWatchControl } from './MainThreadWatchControl';

const getStatus = vi.hoisted(() => vi.fn<() => Promise<MainThreadWatchStatus>>());
const setArmed = vi.hoisted(() => vi.fn<(armed: boolean) => Promise<MainThreadWatchStatus>>());
const listeners = vi.hoisted(() => new Set<(status: MainThreadWatchStatus) => void>());

vi.mock('../../lib/mainThreadWatch', () => ({
  getMainThreadWatchStatus: getStatus,
  setMainThreadWatchArmed: setArmed,
  listenMainThreadWatchStatus: async (onStatus: (status: MainThreadWatchStatus) => void) => {
    listeners.add(onStatus);
    return () => { listeners.delete(onStatus); };
  },
}));

const OFF: MainThreadWatchStatus = { setting: false, runningThisLaunch: false, forcedByEnv: false, seq: 0 };

/** What Rust does after a change: every owner window hears the new status. */
function broadcast(status: MainThreadWatchStatus) {
  for (const listener of [...listeners]) listener(status);
}

describe('MainThreadWatchControl', () => {
  let container: HTMLDivElement;
  let root: Root;
  /** The native side: one status, `seq` grows with every change. */
  let native: MainThreadWatchStatus;

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    native = OFF;
    listeners.clear();
    getStatus.mockImplementation(async () => native);
    setArmed.mockImplementation(async armed => {
      // Saving never starts or stops the watch: `runningThisLaunch` is fixed.
      native = { ...native, setting: armed, seq: native.seq + 1 };
      broadcast(native);
      return native;
    });
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

  function switches(): HTMLElement[] {
    return Array.from(container.querySelectorAll<HTMLElement>('[role="switch"][aria-label="Main-thread watch"]'));
  }

  function watchSwitch(): HTMLElement {
    const [control] = switches();
    expect(control).toBeDefined();
    return control;
  }

  it('shows one labelled control, off by default, with a one-line cost note', async () => {
    await render();
    expect(container.querySelectorAll('[role="switch"]')).toHaveLength(1);
    expect(container.textContent).toContain('Main-thread watch');
    expect(container.textContent).toContain('Uses extra CPU and log volume while on');
    expect(watchSwitch().getAttribute('aria-checked')).toBe('false');
    expect(watchSwitch().hasAttribute('data-disabled')).toBe(false);
  });

  function restartNote(root: ParentNode = container): string | null {
    return root.querySelector('[data-testid="main-thread-watch-restart-note"]')?.textContent ?? null;
  }

  it('saves the setting and says a restart is needed, in both directions', async () => {
    await render();
    expect(restartNote()).toBeNull();

    await act(async () => { watchSwitch().click(); });
    expect(setArmed).toHaveBeenLastCalledWith(true);
    expect(watchSwitch().getAttribute('aria-checked')).toBe('true');
    expect(restartNote()).toBe('Takes effect after you restart Murmur.');

    // Back to what this launch is doing: nothing left to restart for.
    await act(async () => { watchSwitch().click(); });
    expect(setArmed).toHaveBeenLastCalledWith(false);
    expect(watchSwitch().getAttribute('aria-checked')).toBe('false');
    expect(restartNote()).toBeNull();
  });

  it('turning a running watch off also needs a restart', async () => {
    native = { setting: true, runningThisLaunch: true, forcedByEnv: false, seq: 0 };
    await render();
    expect(watchSwitch().getAttribute('aria-checked')).toBe('true');
    expect(restartNote()).toBeNull();

    await act(async () => { watchSwitch().click(); });
    expect(watchSwitch().getAttribute('aria-checked')).toBe('false');
    expect(restartNote()).toBe('Takes effect after you restart Murmur.');
  });

  it('shows the saved setting, and a switch that is still usable, when the environment runs the watch', async () => {
    native = { setting: false, runningThisLaunch: true, forcedByEnv: true, seq: 0 };
    await render();
    expect(watchSwitch().getAttribute('aria-checked')).toBe('false');
    expect(watchSwitch().hasAttribute('data-disabled')).toBe(false);
    expect(container.textContent).toContain('MURMUR_MAIN_THREAD_WATCH');
    // The environment keeps it running either way, so there is nothing pending.
    expect(restartNote()).toBeNull();
    await act(async () => { watchSwitch().click(); });
    expect(watchSwitch().getAttribute('aria-checked')).toBe('true');
    expect(restartNote()).toBeNull();
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

  describe('two views of the same setting (main window and the pop-out)', () => {
    let second: HTMLDivElement;
    let secondRoot: Root;

    beforeEach(async () => {
      second = document.createElement('div');
      document.body.appendChild(second);
      secondRoot = createRoot(second);
      await render();
      await act(async () => { secondRoot.render(<MainThreadWatchControl />); });
    });

    afterEach(async () => {
      await act(async () => secondRoot.unmount());
      second.remove();
    });

    const other = () => second.querySelector<HTMLElement>('[role="switch"]')!;

    it('shows a change made in one in the other, and sends the right value from the other', async () => {
      expect(watchSwitch().getAttribute('aria-checked')).toBe('false');
      expect(other().getAttribute('aria-checked')).toBe('false');

      expect(restartNote(container)).toBeNull();
      expect(restartNote(second)).toBeNull();

      await act(async () => { other().click(); });
      expect(setArmed).toHaveBeenLastCalledWith(true);
      expect(watchSwitch().getAttribute('aria-checked')).toBe('true');
      expect(other().getAttribute('aria-checked')).toBe('true');
      // Both views show the same state and the same restart note.
      expect(restartNote(container)).toBe('Takes effect after you restart Murmur.');
      expect(restartNote(second)).toBe('Takes effect after you restart Murmur.');

      // The first view was never touched, yet its next click turns the watch
      // off: it acts on the status it heard, not a stale cached one.
      await act(async () => { watchSwitch().click(); });
      expect(setArmed).toHaveBeenLastCalledWith(false);
      expect(watchSwitch().getAttribute('aria-checked')).toBe('false');
      expect(other().getAttribute('aria-checked')).toBe('false');
      expect(restartNote(container)).toBeNull();
      expect(restartNote(second)).toBeNull();
    });

    it('stops listening once a view is unmounted', async () => {
      expect(listeners.size).toBe(2);
      await act(async () => secondRoot.render(null));
      expect(listeners.size).toBe(1);
    });
  });

  it('settles on the change when a read spans it, whichever of read and event lands first', async () => {
    // The read was started before the change and resolves after the change's
    // event; Rust returns the same snapshot (same seq) to both.
    let finishRead: (status: MainThreadWatchStatus) => void = () => {};
    getStatus.mockImplementationOnce(() => new Promise(resolve => { finishRead = resolve; }));
    await render();
    const changed: MainThreadWatchStatus = { ...OFF, setting: true, seq: 1 };
    await act(async () => { broadcast(changed); });
    expect(watchSwitch().getAttribute('aria-checked')).toBe('true');
    expect(restartNote()).toBe('Takes effect after you restart Murmur.');
    await act(async () => { finishRead(changed); });
    expect(watchSwitch().getAttribute('aria-checked')).toBe('true');
    expect(restartNote()).toBe('Takes effect after you restart Murmur.');
    // An event delivered twice changes nothing.
    await act(async () => { broadcast(changed); });
    expect(watchSwitch().getAttribute('aria-checked')).toBe('true');
  });

  it('keeps the newest status when an older one arrives late', async () => {
    // The first read is slow and returns the state from before a change that
    // another window made and broadcast in the meantime.
    let finishRead: (status: MainThreadWatchStatus) => void = () => {};
    getStatus.mockImplementationOnce(() => new Promise(resolve => { finishRead = resolve; }));
    await render();
    await act(async () => {
      broadcast({ setting: true, runningThisLaunch: false, forcedByEnv: false, seq: 5 });
    });
    expect(watchSwitch().getAttribute('aria-checked')).toBe('true');
    await act(async () => { finishRead({ ...OFF, seq: 4 }); });
    expect(watchSwitch().getAttribute('aria-checked')).toBe('true');
    // And an older broadcast is ignored the same way.
    await act(async () => { broadcast({ ...OFF, seq: 3 }); });
    expect(watchSwitch().getAttribute('aria-checked')).toBe('true');
    await act(async () => { broadcast({ ...OFF, seq: 6 }); });
    expect(watchSwitch().getAttribute('aria-checked')).toBe('false');
  });
});
