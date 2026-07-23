import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/hooks/useEventStore', () => ({
  useEventStore: () => ({ events: [], clear: vi.fn() }),
}));

vi.mock('../../lib/hooks/usePerformanceDiagnostics', () => ({
  usePerformanceDiagnostics: () => ({
    runs: [],
    samples: [],
    runsLoading: false,
    resourcesLoading: false,
    runsError: null,
    resourcesError: null,
    clearError: null,
    cleared: false,
    clearing: false,
    refreshRuns: vi.fn(),
    refreshResources: vi.fn(),
    clear: vi.fn(),
  }),
}));

vi.mock('../../lib/hooks/usePerformanceHealth', () => ({
  usePerformanceHealth: () => ({
    loading: false,
    error: null,
    modelName: 'base.en',
    dictationStatus: 'idle',
    transformStatus: 'idle',
    runtime: null,
    refresh: vi.fn(),
  }),
}));

import { LogViewerApp } from './LogViewerApp';

describe('LogViewerApp shared diagnostics shell', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(async () => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<LogViewerApp />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('replaces Metrics with Events, Performance, and Runs tabs', async () => {
    const tabs = Array.from(container.querySelectorAll('[role="tab"]'));
    expect(tabs.map(tab => tab.textContent)).toEqual(['Events', 'Performance', 'Runs']);
    expect(container.textContent).not.toContain('Metrics');

    await act(async () => (tabs[1] as HTMLButtonElement).click());
    expect(container.querySelector('#diagnostics-panel-performance')).not.toBeNull();
    expect(tabs[1].getAttribute('aria-selected')).toBe('true');

    await act(async () => (tabs[2] as HTMLButtonElement).click());
    expect(container.querySelector('#diagnostics-panel-runs')).not.toBeNull();
    expect(tabs[2].getAttribute('aria-selected')).toBe('true');
  });
});
