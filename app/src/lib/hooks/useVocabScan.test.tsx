import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { VocabScanSummary } from '../settings';

type ProgressListener = (event: { payload: Record<string, unknown> }) => void;

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: [] as ProgressListener[],
  unlisteners: [] as ReturnType<typeof vi.fn>[],
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (_event: string, listener: ProgressListener) => {
    const unlisten = vi.fn();
    mocks.listeners.push(listener);
    mocks.unlisteners.push(unlisten);
    return unlisten;
  }),
}));

import { useVocabScan } from './useVocabScan';

type ScanState = ReturnType<typeof useVocabScan>;

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function summary(adopted: boolean, terms = 2): VocabScanSummary {
  return {
    files: 3,
    skipped: 1,
    terms,
    bytes: 42,
    capped: false,
    ms: 7,
    sampleTerms: terms > 0 ? ['fooBar'] : [],
    rankedTerms: terms > 0 ? [{ term: 'fooBar', freq: 2 }] : [],
    whisperCount: terms > 0 ? 1 : 0,
    adopted,
  };
}

describe('useVocabScan correlation and adoption', () => {
  let container: HTMLDivElement;
  let root: Root;
  let current: ScanState;

  beforeEach(async () => {
    vi.clearAllMocks();
    mocks.listeners.length = 0;
    mocks.unlisteners.length = 0;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);

    function Harness() {
      current = useVocabScan();
      return null;
    }

    await act(async () => root.render(<Harness />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('ignores orphaned progress from an overlapping older scan', async () => {
    const first = deferred<VocabScanSummary>();
    const second = deferred<VocabScanSummary>();
    mocks.invoke.mockImplementation((_command: string, args: { folder: string }) =>
      args.folder === '/first' ? first.promise : second.promise,
    );

    let firstRun!: Promise<VocabScanSummary | null>;
    await act(async () => {
      firstRun = current.scan('/first');
      await Promise.resolve();
    });
    const firstScanId = mocks.invoke.mock.calls[0][1].scanId as string;

    let secondRun!: Promise<VocabScanSummary | null>;
    await act(async () => {
      secondRun = current.scan('/second');
      await Promise.resolve();
    });
    const secondScanId = mocks.invoke.mock.calls[1][1].scanId as string;
    expect(secondScanId).not.toBe(firstScanId);

    await act(async () => {
      mocks.listeners[1]({
        payload: {
          scanId: firstScanId,
          currentPath: '/first/stale.ts',
          filesRead: 99,
          dirsSkipped: 8,
          termsSoFar: 77,
          done: false,
          adopted: false,
        },
      });
    });
    expect(current.stats.filesRead).toBe(0);
    expect(current.walker).toHaveLength(0);

    await act(async () => {
      mocks.listeners[1]({
        payload: {
          scanId: secondScanId,
          currentPath: '/second/current.ts',
          filesRead: 1,
          dirsSkipped: 0,
          termsSoFar: 4,
          done: false,
          adopted: false,
        },
      });
    });
    expect(current.stats.filesRead).toBe(1);
    expect(current.walker[0].path).toBe('/second/current.ts');

    second.resolve(summary(true));
    await act(async () => {
      await secondRun;
    });
    expect(current.status).toBe('done');

    first.resolve(summary(false));
    await act(async () => {
      expect(await firstRun).toBeNull();
    });
    expect(current.status).toBe('done');
  });

  it('surfaces a completed but non-adopted scan as superseded', async () => {
    mocks.invoke.mockResolvedValueOnce(summary(false));

    let result!: VocabScanSummary | null;
    await act(async () => {
      result = await current.scan('/changed-during-walk');
    });

    expect(result?.adopted).toBe(false);
    expect(current.status).toBe('superseded');
    expect(current.stats.summary?.adopted).toBe(false);
  });

  it('invalidates the matching backend scan when canceled', async () => {
    const pending = deferred<VocabScanSummary>();
    mocks.invoke.mockImplementation((command: string) =>
      command === 'scan_code_vocab' ? pending.promise : Promise.resolve(true),
    );

    let scanRun!: Promise<VocabScanSummary | null>;
    await act(async () => {
      scanRun = current.scan('/cancel-me');
      await Promise.resolve();
    });
    const scanId = mocks.invoke.mock.calls[0][1].scanId as string;

    await act(async () => current.cancel());
    expect(mocks.invoke).toHaveBeenCalledWith('cancel_code_vocab_scan', { scanId });
    expect(current.status).toBe('idle');

    pending.resolve(summary(false));
    await act(async () => {
      expect(await scanRun).toBeNull();
    });
    expect(current.status).toBe('idle');
  });
});
