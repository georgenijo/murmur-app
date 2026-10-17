import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { PerformanceRunV1, ResourceSampleV1 } from '../performance';
import { makeResourceSample, makeRun } from '../../components/log-viewer/testFixtures';

const mocks = vi.hoisted(() => ({
  list: vi.fn(),
  resources: vi.fn(),
  clear: vi.fn(),
  run: null as ((run: PerformanceRunV1) => void) | null,
  sample: null as ((sample: ResourceSampleV1) => void) | null,
  cleared: null as (() => void) | null,
  stop: vi.fn(),
}));
vi.mock('../performance', async importOriginal => ({
  ...await importOriginal<typeof import('../performance')>(),
  listPerformanceRuns: mocks.list,
  getPerformanceResourceWindow: mocks.resources,
  clearPerformanceDiagnostics: mocks.clear,
  onPerformanceRunCompleted: async (handler: (run: PerformanceRunV1) => void) => {
    mocks.run = handler;
    return () => mocks.stop('run');
  },
  onPerformanceResourceSample: async (handler: (sample: ResourceSampleV1) => void) => {
    mocks.sample = handler;
    return () => mocks.stop('sample');
  },
  onPerformanceDiagnosticsCleared: async (handler: () => void) => {
    mocks.cleared = handler;
    return () => mocks.stop('cleared');
  },
}));

import { usePerformanceDiagnostics } from './usePerformanceDiagnostics';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}

describe('performance diagnostics visibility lifecycle', () => {
  let container: HTMLDivElement;
  let root: Root;
  let state: ReturnType<typeof usePerformanceDiagnostics>;
  function Probe({ active }: { active: boolean }) {
    state = usePerformanceDiagnostics(active);
    return null;
  }
  const render = async (active: boolean) => {
    await act(async () => root.render(<Probe active={active} />));
  };
  beforeEach(() => {
    mocks.list.mockReset().mockResolvedValue({ runs: [] });
    mocks.resources.mockReset().mockResolvedValue([]);
    mocks.stop.mockClear();
    mocks.run = null;
    mocks.sample = null;
    mocks.cleared = null;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('does no inactive reads, unsubscribes on hide, and ignores queued hidden events', async () => {
    await render(false);
    expect(mocks.list).not.toHaveBeenCalled();
    expect(mocks.resources).not.toHaveBeenCalled();
    expect(mocks.run).toBeNull();
    await render(true);
    expect(mocks.list).toHaveBeenCalledOnce();
    await render(false);
    expect(mocks.stop).toHaveBeenCalledTimes(3);
    await act(async () => {
      mocks.run?.(makeRun());
      mocks.sample?.(makeResourceSample(100));
      await state.refresh();
    });
    expect(state.runs).toEqual([]);
    expect(state.samples).toEqual([]);
    expect(mocks.list).toHaveBeenCalledOnce();
  });

  it('does not resurrect cached rows when another window cleared the backend while hidden', async () => {
    mocks.list.mockResolvedValueOnce({ runs: [makeRun()] });
    mocks.resources.mockResolvedValueOnce([makeResourceSample(100)]);
    await render(true);
    expect(state.runs).toHaveLength(1);
    expect(state.samples).toHaveLength(1);
    await render(false);
    // Keep the previous data mounted until the fresh snapshot arrives.
    expect(state.runs).toHaveLength(1);
    await render(true);
    expect(state.runs).toEqual([]);
    expect(state.samples).toEqual([]);
  });

  it('retains live arrivals during snapshot hydration', async () => {
    const runs = deferred<{ runs: PerformanceRunV1[] }>();
    const samples = deferred<ResourceSampleV1[]>();
    mocks.list.mockReturnValueOnce(runs.promise);
    mocks.resources.mockReturnValueOnce(samples.promise);
    await render(true);
    const live = makeRun({ runId: 'live', finishedAtMs: 200 });
    await act(async () => {
      mocks.run?.(live);
      mocks.sample?.(makeResourceSample(200));
      runs.resolve({ runs: [makeRun({ runId: 'snapshot', finishedAtMs: 100 })] });
      samples.resolve([makeResourceSample(100)]);
    });
    expect(state.runs.map(run => run.runId)).toEqual(['live', 'snapshot']);
    expect(state.samples.map(sample => sample.observedAtMs)).toEqual([100, 200]);
  });

  it('ignores reads from the previous visibility generation after reopening', async () => {
    const old = deferred<{ runs: PerformanceRunV1[] }>();
    mocks.list.mockReturnValueOnce(old.promise);
    await render(true);
    await render(false);
    await render(true);
    await act(async () => old.resolve({ runs: [makeRun({ runId: 'stale' })] }));
    expect(state.runs).toEqual([]);
  });
});
