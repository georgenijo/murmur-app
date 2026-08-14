import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const hookMocks = vi.hoisted(() => ({
  events: [] as Array<{
    timestamp: string;
    stream: 'system';
    level: 'info';
    summary: string;
    data: Record<string, unknown>;
  }>,
  useEventStore: vi.fn(),
  usePerformanceDiagnostics: vi.fn(),
  usePerformanceStoreHealth: vi.fn(),
}));

vi.mock('../../lib/hooks/useEventStore', () => ({
  useEventStore: (active: boolean) => {
    hookMocks.useEventStore(active);
    return { events: hookMocks.events, clear: vi.fn() };
  },
}));

vi.mock('../../lib/hooks/usePerformanceDiagnostics', () => ({
  usePerformanceDiagnostics: (active: boolean) => {
    hookMocks.usePerformanceDiagnostics(active);
    return ({
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
    });
  },
}));

vi.mock('../../lib/hooks/usePerformanceHealth', () => ({
  usePerformanceHealth: () => ({
    loading: false,
    error: null,
    modelName: 'base.en',
    dictationStatus: 'idle',
    transformStatus: 'idle',
    runtime: null,
    capture: {
      status: 'insufficientData',
      sampleCount: 0,
      requiredSamples: 5,
      medianStartupMs: null,
      fallbackCount: 0,
      chronicFallback: false,
      slowStartup: false,
      degradedBackend: null,
    },
    refresh: vi.fn(),
  }),
}));

vi.mock('../../lib/hooks/usePerformanceStoreHealth', () => ({
  usePerformanceStoreHealth: (enabled: boolean) => {
    hookMocks.usePerformanceStoreHealth(enabled);
    return ({
      health: {
        schemaVersion: 1,
        status: 'available',
        skippedRunCount: 0,
        recommendedAction: 'none',
      },
      loading: false,
      error: null,
      recovering: false,
      recoveryError: null,
      refresh: vi.fn(),
      recover: vi.fn(),
    });
  },
}));

vi.mock('../../lib/transformDiagnostics', () => ({
  listTransformAttempts: vi.fn(async () => []),
  listTransformCaptures: vi.fn(async () => []),
  getCaptureArmStatus: vi.fn(async () => ({ armed: false, expiresAtMs: null })),
  armNextTransformCapture: vi.fn(),
  getTransformCapture: vi.fn(),
  deleteTransformCapture: vi.fn(),
}));

import { DiagnosticsWorkspace, type DiagnosticsTab } from './DiagnosticsWorkspace';

describe('DiagnosticsWorkspace shared diagnostics shell', () => {
  let container: HTMLDivElement;
  let root: Root;
  let onPopOut: (tab: DiagnosticsTab) => void;
  let popOutSpy: ReturnType<typeof vi.fn<(tab: DiagnosticsTab) => void>>;

  beforeEach(async () => {
    hookMocks.events = [];
    hookMocks.useEventStore.mockClear();
    hookMocks.usePerformanceDiagnostics.mockClear();
    hookMocks.usePerformanceStoreHealth.mockClear();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    popOutSpy = vi.fn<(tab: DiagnosticsTab) => void>();
    onPopOut = popOutSpy;
    await act(async () => root.render(<DiagnosticsWorkspace onPopOut={onPopOut} />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('renders the six diagnostics tabs in their intended order', async () => {
    const tabs = Array.from(container.querySelectorAll('[role="tab"]'));
    expect(tabs.map(tab => tab.textContent)).toEqual([
      'Events',
      'Runs',
      'Performance',
      'Latency',
      'Compare',
      'Transform',
    ]);
    expect(container.textContent).not.toContain('Metrics');

    await act(async () => (tabs[1] as HTMLButtonElement).click());
    expect(container.querySelector('#diagnostics-panel-runs')).not.toBeNull();
    expect(tabs[1].getAttribute('aria-selected')).toBe('true');

    await act(async () => (tabs[2] as HTMLButtonElement).click());
    expect(container.querySelector('#diagnostics-panel-performance')).not.toBeNull();
    expect(tabs[2].getAttribute('aria-selected')).toBe('true');

    await act(async () => (tabs[3] as HTMLButtonElement).click());
    expect(container.querySelector('#diagnostics-panel-latency')).not.toBeNull();
    expect(tabs[3].getAttribute('aria-selected')).toBe('true');
    expect(container.textContent).toContain('UI latency map');

    await act(async () => (tabs[4] as HTMLButtonElement).click());
    expect(container.querySelector('#diagnostics-panel-reports')).not.toBeNull();
    expect(tabs[4].getAttribute('aria-selected')).toBe('true');
    expect(container.textContent).toContain('Report comparison');

    await act(async () => (tabs[5] as HTMLButtonElement).click());
    expect(container.querySelector('#diagnostics-panel-transforms')).not.toBeNull();
    expect(tabs[5].getAttribute('aria-selected')).toBe('true');
    expect(container.textContent).toContain('Transform diagnostics');
  });

  it('pops out the currently selected diagnostics tab', async () => {
    const tabs = Array.from(container.querySelectorAll('[role="tab"]'));
    await act(async () => (tabs[3] as HTMLButtonElement).click());

    const popOut = container.querySelector(
      'button[aria-label="Open Diagnostics in a separate window"]',
    ) as HTMLButtonElement;
    await act(async () => popOut.click());

    expect(popOutSpy).toHaveBeenCalledWith('latency');
  });

  it('gates live subscriptions when the workspace is inactive', async () => {
    await act(async () => root.render(
      <DiagnosticsWorkspace active={false} onPopOut={onPopOut} />,
    ));
    expect(hookMocks.useEventStore).toHaveBeenLastCalledWith(false);
    expect(hookMocks.usePerformanceDiagnostics).toHaveBeenLastCalledWith(false);
  });

  it('enables store health only when an authorized host opts in', async () => {
    const tabs = Array.from(container.querySelectorAll('[role="tab"]'));
    await act(async () => (tabs[2] as HTMLButtonElement).click());
    expect(hookMocks.usePerformanceStoreHealth).toHaveBeenLastCalledWith(false);
    expect(container.textContent).not.toContain('Diagnostics storage');

    await act(async () => root.render(
      <DiagnosticsWorkspace
        requestedTab="performance"
        storeHealthEnabled
        onPopOut={onPopOut}
      />,
    ));
    expect(hookMocks.usePerformanceStoreHealth).toHaveBeenLastCalledWith(true);
    expect(container.textContent).toContain('Diagnostics storage');
  });

  it('renders only the newest 100 rows while retaining the full filtered count', async () => {
    hookMocks.events = Array.from({ length: 150 }, (_, index) => ({
      timestamp: `2026-08-07T12:00:${String(index).padStart(3, '0')}Z`,
      stream: 'system' as const,
      level: 'info' as const,
      summary: `event ${index}`,
      data: {},
    }));
    await act(async () => root.render(<DiagnosticsWorkspace onPopOut={onPopOut} />));
    expect(container.querySelectorAll('.diagnostic-event-row')).toHaveLength(100);
    expect(container.textContent).toContain('Showing the newest 100 of 150 events');
    expect(container.textContent).not.toContain('event 49');
    expect(container.textContent).toContain('event 50');
    expect(container.textContent).toContain('event 149');
  });
});
