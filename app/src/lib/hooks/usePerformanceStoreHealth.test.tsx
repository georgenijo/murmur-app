import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { PerformanceStoreHealthV1 } from '../performance';

const AVAILABLE: PerformanceStoreHealthV1 = {
  schemaVersion: 1,
  status: 'available',
  skippedRunCount: 0,
  recommendedAction: 'none',
};

const mocks = vi.hoisted(() => ({
  getHealth: vi.fn(),
  recover: vi.fn(),
}));

vi.mock('../performance', async importOriginal => ({
  ...(await importOriginal<typeof import('../performance')>()),
  getPerformanceStoreHealth: mocks.getHealth,
  recoverPerformanceStore: mocks.recover,
}));

import { usePerformanceStoreHealth } from './usePerformanceStoreHealth';

function Probe({ enabled }: { enabled: boolean }) {
  const store = usePerformanceStoreHealth(enabled);
  return (
    <div>
      <output>{store.health?.status ?? 'unknown'}|{store.error}|{store.recoveryError}</output>
      <button type="button" onClick={() => void store.recover()}>Recover</button>
    </div>
  );
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe('usePerformanceStoreHealth', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    mocks.getHealth.mockReset().mockResolvedValue(AVAILABLE);
    mocks.recover.mockReset().mockResolvedValue(AVAILABLE);
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('does no store work while disabled', async () => {
    await act(async () => root.render(<Probe enabled={false} />));
    expect(mocks.getHealth).not.toHaveBeenCalled();
    expect(mocks.recover).not.toHaveBeenCalled();
  });

  it('hydrates typed health and accepts the recovered snapshot as authoritative', async () => {
    const recovered: PerformanceStoreHealthV1 = {
      ...AVAILABLE,
      lastRecovery: {
        action: 'quarantinedAndReinitialized',
        atMs: 1_786_720_000_000,
      },
    };
    mocks.recover.mockResolvedValue(recovered);

    await act(async () => {
      root.render(<Probe enabled />);
      await Promise.resolve();
    });
    expect(container.textContent).toContain('available');

    await act(async () => {
      container.querySelector('button')?.click();
      await Promise.resolve();
    });
    expect(mocks.recover).toHaveBeenCalledOnce();
    expect(mocks.recover).toHaveBeenCalledWith(false);
    expect(container.textContent).toContain('available');
  });

  it('passes explicit destructive intent only for reinitialization', async () => {
    mocks.getHealth.mockResolvedValue({
      ...AVAILABLE,
      status: 'unavailable',
      recommendedAction: 'reinitializeStore',
    });

    await act(async () => {
      root.render(<Probe enabled />);
      await Promise.resolve();
    });
    await act(async () => {
      container.querySelector('button')?.click();
      await Promise.resolve();
    });

    expect(mocks.recover).toHaveBeenCalledWith(true);
  });

  it('does not let an older poll overwrite a completed recovery', async () => {
    const poll = deferred<PerformanceStoreHealthV1>();
    const recovered: PerformanceStoreHealthV1 = {
      ...AVAILABLE,
      lastRecovery: {
        action: 'quarantinedAndReinitialized',
        atMs: 1_786_720_000_000,
      },
    };
    mocks.getHealth.mockReturnValueOnce(poll.promise);
    mocks.recover.mockResolvedValue(recovered);

    await act(async () => root.render(<Probe enabled />));
    await act(async () => {
      container.querySelector('button')?.click();
      await Promise.resolve();
    });
    expect(container.textContent).toContain('available||');

    await act(async () => {
      poll.resolve({
        ...AVAILABLE,
        status: 'unavailable',
        recommendedAction: 'retry',
      });
      await poll.promise;
    });
    expect(container.textContent).toContain('available||');
  });

  it('discards a pending poll failure after recovery and when disabled', async () => {
    const poll = deferred<PerformanceStoreHealthV1>();
    mocks.getHealth.mockReturnValueOnce(poll.promise);

    await act(async () => root.render(<Probe enabled />));
    await act(async () => {
      container.querySelector('button')?.click();
      await Promise.resolve();
      root.render(<Probe enabled={false} />);
    });
    await act(async () => {
      poll.reject(new Error('stale raw database failure'));
      await poll.promise.catch(() => undefined);
    });

    expect(container.textContent).toContain('available||');
    expect(container.textContent).not.toContain('could not verify');
  });

  it('never forwards raw backend failures to presentation state', async () => {
    mocks.getHealth.mockRejectedValue(new Error('SQLITE_IOERR /Users/private/performance.sqlite3'));
    mocks.recover.mockRejectedValue(new Error('SQLITE_CORRUPT at /Users/private'));

    await act(async () => {
      root.render(<Probe enabled />);
      await Promise.resolve();
    });
    expect(container.textContent).toContain('Murmur could not verify the local diagnostics store.');
    expect(container.textContent).not.toContain('SQLITE');
    expect(container.textContent).not.toContain('/Users');

    await act(async () => {
      container.querySelector('button')?.click();
      await Promise.resolve();
    });
    expect(container.textContent).toContain('Diagnostics recovery did not complete.');
    expect(container.textContent).not.toContain('CORRUPT');
  });
});
