import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import benchmarkV2 from '../../lib/__fixtures__/diagnostic-reports/benchmark-v2.json';
import evaluationDeterministic from '../../lib/__fixtures__/diagnostic-reports/evaluation-v1-deterministic.json';
import { MAX_DIAGNOSTIC_REPORT_BYTES } from '../../lib/diagnosticReports';
import { ReportCompareView } from './ReportCompareView';

function jsonFile(value: unknown): File {
  const contents = JSON.stringify(value);
  const file = new File([contents], 'private-source-name.json', { type: 'application/json' });
  Object.defineProperty(file, 'text', { value: async () => contents });
  return file;
}

describe('ReportCompareView', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(async () => {
    localStorage.clear();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<ReportCompareView />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    localStorage.clear();
  });

  async function importFile(file: File) {
    const input = container.querySelector('[data-testid="diagnostic-report-input"]') as HTMLInputElement;
    Object.defineProperty(input, 'files', {
      configurable: true,
      value: [file],
    });
    await act(async () => {
      input.dispatchEvent(new Event('change', { bubbles: true }));
      await new Promise(resolve => setTimeout(resolve, 0));
    });
  }

  it('imports compatible benchmark reports and shows side-by-side deltas', async () => {
    const candidate = structuredClone(benchmarkV2);
    candidate.createdAt = '2026-07-21T14:00:00Z';
    candidate.sharedInitMs = 900;

    await importFile(jsonFile(benchmarkV2));
    await importFile(jsonFile(candidate));

    expect(container.textContent).toContain('compatible');
    expect(container.textContent).toContain('Shared initialization');
    expect(container.textContent).toContain('1,200 ms');
    expect(container.textContent).toContain('900 ms');
    expect(container.textContent).toContain('-300 ms');
    expect(container.textContent).toContain('-25%');
    expect(container.textContent).toContain('Eligible.');
    expect(container.textContent).toContain('Fastest');
    expect(container.textContent).not.toContain('private-source-name.json');
  });

  it('imports evaluation reports with the curated-text privacy warning', async () => {
    await importFile(jsonFile(evaluationDeterministic));

    expect(container.textContent).toContain('Evaluation · v1 · fixtures v1');
    expect(container.textContent).toContain('curated fixture transcripts and per-stage text');
    expect(container.textContent).toContain('Fixture-only deterministic run');
  });

  it('fails closed for malformed, oversized, and incomplete reports', async () => {
    const malformedContents = '{"private":"PRIVATE_PARSE_SENTINEL"';
    const malformed = new File([malformedContents], 'secret.json');
    Object.defineProperty(malformed, 'text', { value: async () => malformedContents });
    await importFile(malformed);
    expect(container.querySelector('[role="alert"]')?.textContent)
      .toBe('The selected file is not valid JSON.');
    expect(container.textContent).not.toContain('PRIVATE_PARSE_SENTINEL');
    expect(container.textContent).not.toContain('secret.json');

    const text = vi.fn(async () => {
      throw new Error('oversized files must not be read');
    });
    const oversized = {
      size: MAX_DIAGNOSTIC_REPORT_BYTES + 1,
      text,
    } as unknown as File;
    await importFile(oversized);
    expect(container.querySelector('[role="alert"]')?.textContent)
      .toBe('Diagnostic reports are limited to 8 MiB.');
    expect(text).not.toHaveBeenCalled();

    const incomplete = structuredClone(benchmarkV2);
    incomplete.results[0].warmMedianMs = null as unknown as number;
    await importFile(jsonFile(incomplete));
    expect(container.querySelector('[role="alert"]')?.textContent)
      .toBe('The selected JSON is not a supported Murmur benchmark or evaluation report.');
    expect(container.textContent).toContain('No diagnostic reports available');
  });

  it('blocks deltas and recommendations for a valid failed benchmark result', async () => {
    const failed = JSON.parse(JSON.stringify(benchmarkV2));
    failed.createdAt = '2026-07-21T14:30:00Z';
    failed.results[0].error = 'PRIVATE_FAILURE_SENTINEL';
    failed.results[0].modelLoadMs = null;
    failed.results[0].firstInferenceMs = null;
    failed.results[0].warmMedianMs = null;
    failed.results[0].warmP95Ms = null;
    failed.results[0].realtimeFactor = null;
    failed.results[0].wordErrorRate = null;
    failed.results[0].normalizedWordErrorRate = null;
    failed.results[0].deliveredWordErrorRate = null;
    failed.results[0].deliveredNormalizedWordErrorRate = null;
    failed.results[0].fixtures = [];

    await importFile(jsonFile(benchmarkV2));
    await importFile(jsonFile(failed));

    expect(container.textContent).toContain('One or more benchmark model results are incomplete or failed.');
    expect(container.textContent).toContain('Deltas and recommendations are unavailable');
    expect(container.textContent).toContain('Not eligible.');
    expect(container.textContent).not.toContain('Shared initialization');
    expect(container.textContent).not.toContain('PRIVATE_FAILURE_SENTINEL');
  });

  it('clears imported session state without deleting saved Performance Lab history', async () => {
    localStorage.setItem('murmur-benchmark-reports', JSON.stringify([benchmarkV2]));
    await act(async () => {
      root.unmount();
      root = createRoot(container);
      root.render(<ReportCompareView />);
    });
    await importFile(jsonFile({ ...benchmarkV2, createdAt: '2026-07-21T15:00:00Z' }));

    const clear = Array.from(container.querySelectorAll('button'))
      .find(button => button.textContent === 'Clear imports') as HTMLButtonElement;
    await act(async () => clear.click());

    expect(localStorage.getItem('murmur-benchmark-reports')).not.toBeNull();
    expect(container.textContent).toContain('saved Performance Lab history were not changed');
    const badges = Array.from(container.querySelectorAll('span'))
      .map(element => element.textContent?.trim());
    expect(badges).toContain('local');
    expect(badges).not.toContain('imported');
  });

  it('uses compact responsive grids and a bounded horizontal metric region', async () => {
    const candidate = structuredClone(benchmarkV2);
    candidate.createdAt = '2026-07-21T16:00:00Z';
    await importFile(jsonFile(benchmarkV2));
    await importFile(jsonFile(candidate));

    expect(container.querySelector('[data-testid="report-selection-grid"]')?.className)
      .toContain('grid-cols-1');
    expect(container.querySelector('[data-testid="report-selection-grid"]')?.className)
      .toContain('lg:grid-cols-2');
    expect(container.querySelector('[data-testid="report-summary-grid"]')?.className)
      .toContain('grid-cols-1');
    expect(container.querySelector('[data-testid="report-metrics-scroller"]')?.className)
      .toContain('overflow-x-auto');
  });
});
