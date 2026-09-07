import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { PerformanceRunV1, ProductionRunV1 } from '../../lib/performance';
import { ProductionComparison } from './ProductionComparison';
import { makeRun } from './testFixtures';

function production(
  cohort: Partial<ProductionRunV1['cohort']> = {},
  capture: Partial<ProductionRunV1['capture']> = {},
): ProductionRunV1 {
  return {
    schemaVersion: 1,
    cohort: {
      machineId: '123e4567-e89b-12d3-a456-426614174000',
      osVersion: '15.6.1',
      architecture: 'aarch64',
      buildMode: 'release',
      microphoneSelection: 'systemDefault',
      microphoneKind: 'builtIn',
      configurationKey: 'a'.repeat(64),
      ...cohort,
    },
    capture: {
      helperResolveMs: 4,
      helperSignatureMs: 5,
      helperSpawnMs: 10,
      streamOpenMs: 40,
      firstCallbackWaitMs: 90,
      firstPcmMs: 145,
      readyMs: 180,
      stopToWorkerExitMs: 20,
      backend: 'auhal',
      fallbackAttempted: false,
      fallbackSucceeded: null,
      failureKind: null,
      workerInvariantViolation: false,
      zeroSampleSuccess: false,
      ...capture,
    },
  };
}

function versionRun(
  appVersion: string,
  overrides: Partial<PerformanceRunV1> = {},
): PerformanceRunV1 {
  return makeRun({ appVersion, production: production(), ...overrides });
}

describe('ProductionComparison', () => {
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

  async function render(runs: PerformanceRunV1[]) {
    await act(async () => root.render(<ProductionComparison runs={runs} />));
  }

  it('stays separate from the run list and requires two dictation versions', async () => {
    await render([versionRun('1.0.0')]);

    expect(container.querySelector('details > summary')?.textContent).toContain('Compare app versions');
    expect(container.textContent).toContain(
      'Dictation runs from two app versions are needed before a comparison can be made.',
    );
  });

  it('shows matched latency, preliminary raw values, and version-wide anomalies together', async () => {
    const baseline = versionRun('1.0.0');
    const candidate = versionRun('1.1.0', {
      startedAtMs: baseline.startedAtMs + 1_000,
      production: production({}, { readyMs: 220 }),
    });
    const candidateFailure = versionRun('1.1.0', {
      startedAtMs: baseline.startedAtMs + 2_000,
      outcome: { status: 'timedOut', stage: 'captureFinalization' },
      production: production({}, {
        fallbackAttempted: true,
        fallbackSucceeded: false,
        failureKind: 'first_buffer_timeout',
        workerInvariantViolation: true,
      }),
    });
    await render([baseline, candidate, candidateFailure]);

    expect((container.querySelector('[aria-label="Baseline app version"]') as HTMLSelectElement).value)
      .toBe('1.0.0');
    expect((container.querySelector('[aria-label="Candidate app version"]') as HTMLSelectElement).value)
      .toBe('1.1.0');
    expect(container.textContent).toContain('Version-wide observations');
    expect(container.textContent).toContain('New in candidate');
    expect(container.textContent).toContain('Compatible cohort 1');
    expect(container.textContent).toContain('Capture ready');
    expect(container.textContent).toContain('+40 ms · +22.2%');
    expect(container.textContent).toContain('Insufficient samples');
    expect(container.textContent).toContain('Preliminary raw values');
    expect(container.textContent).toContain('Baseline: 180 ms. Candidate: 220 ms.');
  });

  it('explains historical, development, and exact-cohort exclusions without hiding anomalies', async () => {
    const historical = versionRun('1.0.0', { production: undefined });
    const development = versionRun('1.0.0', {
      production: production({ buildMode: 'development' }),
    });
    const candidate = versionRun('1.1.0', {
      startedAtMs: historical.startedAtMs + 1_000,
      outcome: { status: 'failed', stage: 'inferenceDecode', errorCode: 'inferenceFailed' },
      production: production({ machineId: '223e4567-e89b-12d3-a456-426614174000' }),
    });
    await render([historical, development, candidate]);

    expect(container.textContent).toContain('No exact-compatible latency cohort was found');
    expect(container.textContent).toContain('Historical run without production cohort metadata');
    expect(container.textContent).toContain('Development build');
    expect(container.textContent).toContain('Failed, timed out, or interrupted');
    expect(container.textContent).toContain('New in candidate');
  });
});
