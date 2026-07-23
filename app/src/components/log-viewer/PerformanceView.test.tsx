import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { PerformanceHealth } from '../../lib/hooks/usePerformanceHealth';
import { makeResourceSample, unavailable } from './testFixtures';
import { PerformanceView } from './PerformanceView';

const HEALTH: PerformanceHealth = {
  loading: false,
  error: null,
  modelName: 'base.en',
  dictationStatus: 'idle',
  transformStatus: 'idle',
  runtime: {
    generation: 1,
    modelName: 'base.en',
    label: 'Whisper Base (English)',
    size: '~150 MB',
    backend: 'Whisper',
    accelerator: 'Metal GPU',
    capabilities: {
      partialResults: false,
      initialPrompts: true,
      multilingual: false,
      translation: false,
      timestamps: true,
      confidence: false,
      punctuationControl: true,
    },
    supportedPlatforms: ['macos'],
    supported: true,
    unavailableReason: null,
    installState: 'installed',
    lifecycleState: 'ready',
    failurePresent: false,
  },
};

describe('PerformanceView', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('labels every live metric scope and keeps accelerator utilization unavailable', async () => {
    const first = makeResourceSample(Date.UTC(2026, 6, 23, 12));
    const second = makeResourceSample(Date.UTC(2026, 6, 23, 12, 0, 1), {
      sidecarProcess: {
        cpuPercent: unavailable(),
        rssBytes: unavailable(),
      },
    });
    await act(async () => {
      root.render(
        <PerformanceView
          samples={[first, second]}
          loading={false}
          error={null}
          health={HEALTH}
          onRetry={vi.fn()}
        />,
      );
    });

    expect(container.textContent).toContain('Whole-host utilization');
    expect(container.textContent).toContain('Main process · 100% equals one logical core');
    expect(container.textContent).toContain('Local LLM helper process');
    expect(container.textContent).toContain('Accelerator utilization');
    expect(container.textContent).toContain('No production GPU or ANE percentage');
    expect(container.textContent?.match(/No production GPU or ANE percentage/g)).toHaveLength(1);
    expect(container.textContent).not.toMatch(/GPU utilization\s+\d/);
    expect(container.querySelectorAll('svg[role="img"]')).toHaveLength(2);
    expect(container.querySelector('[aria-label="Shared resource chart timeline cursor"]')).not.toBeNull();
  });

  it('distinguishes loading, empty, and error states', async () => {
    await act(async () => {
      root.render(
        <PerformanceView
          samples={[]}
          loading
          error={null}
          health={{ ...HEALTH, loading: true, runtime: null }}
          onRetry={vi.fn()}
        />,
      );
    });
    expect(container.querySelector('[aria-label="Loading resource samples"]')).not.toBeNull();

    await act(async () => {
      root.render(
        <PerformanceView
          samples={[]}
          loading={false}
          error={null}
          health={HEALTH}
          onRetry={vi.fn()}
        />,
      );
    });
    expect(container.textContent).toContain('Waiting for the first resource sample');

    await act(async () => {
      root.render(
        <PerformanceView
          samples={[]}
          loading={false}
          error="store unavailable"
          health={HEALTH}
          onRetry={vi.fn()}
        />,
      );
    });
    expect(container.textContent).toContain('Resource samples unavailable');
    expect(container.querySelector('[role="alert"]')).not.toBeNull();
  });
});
