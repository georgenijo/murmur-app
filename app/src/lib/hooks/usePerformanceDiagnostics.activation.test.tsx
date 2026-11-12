import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const performance = vi.hoisted(() => ({
  listPerformanceRuns: vi.fn(async () => ({ schemaVersion: 1, runs: [] })),
  getPerformanceResourceWindow: vi.fn(async () => []),
}));

vi.mock('../performance', () => ({
  clearPerformanceDiagnostics: vi.fn(),
  getPerformanceResourceWindow: performance.getPerformanceResourceWindow,
  listPerformanceRuns: performance.listPerformanceRuns,
  onPerformanceDiagnosticsCleared: vi.fn(async () => () => {}),
  onPerformanceResourceSample: vi.fn(async () => () => {}),
  onPerformanceRunCompleted: vi.fn(async () => () => {}),
}));

import { usePerformanceDiagnostics } from './usePerformanceDiagnostics';

let latest: ReturnType<typeof usePerformanceDiagnostics> | null = null;

function Probe({ enabled }: { enabled: boolean }) {
  latest = usePerformanceDiagnostics(enabled);
  return null;
}

describe('usePerformanceDiagnostics activation', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    performance.listPerformanceRuns.mockClear();
    performance.getPerformanceResourceWindow.mockClear();
    latest = null;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('loads nothing while inactive and reports a loading state', async () => {
    await act(async () => root.render(<Probe enabled={false} />));
    expect(performance.listPerformanceRuns).not.toHaveBeenCalled();
    expect(performance.getPerformanceResourceWindow).not.toHaveBeenCalled();
    expect(latest?.runsLoading).toBe(true);
    expect(latest?.resourcesLoading).toBe(true);
  });

  it('loads each source exactly once when it becomes active', async () => {
    await act(async () => root.render(<Probe enabled={false} />));
    await act(async () => root.render(<Probe enabled />));
    expect(performance.listPerformanceRuns).toHaveBeenCalledTimes(1);
    expect(performance.getPerformanceResourceWindow).toHaveBeenCalledTimes(1);
    expect(latest?.runsLoading).toBe(false);
    expect(latest?.resourcesLoading).toBe(false);
  });
});
