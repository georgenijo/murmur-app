import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { loadStats, saveStats } from '../lib/stats';
import { UsageDashboard } from './UsageDashboard';

describe('UsageDashboard', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    localStorage.clear();
    const defaults = loadStats();
    saveStats({
      ...defaults,
      query: {
        ...defaults.query,
        queriesRun: 7,
        successfulQueries: 5,
        failedQueries: 2,
        inputTokens: 1_240,
        outputTokens: 380,
        reportedCostUsd: 0.04,
        byProvider: {
          ...defaults.query.byProvider,
          claude: {
            queriesRun: 4,
            successfulQueries: 3,
            failedQueries: 1,
            inputTokens: 800,
            outputTokens: 240,
            reportedCostUsd: 0.025,
          },
          codex: {
            queriesRun: 3,
            successfulQueries: 2,
            failedQueries: 1,
            inputTokens: 440,
            outputTokens: 140,
            reportedCostUsd: 0.015,
          },
        },
        failuresByErrorCode: { timed_out: 2 },
      },
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('uses four peer analytics sections and a stable label/value summary grid', async () => {
    await act(async () => root.render(<UsageDashboard statsVersion={0} displayMode="page" />));

    expect(container.querySelectorAll('.usage-analytics-section')).toHaveLength(4);
    const metricRows = container.querySelectorAll('.usage-query-metrics > div');
    expect(metricRows).toHaveLength(3);
    expect(Array.from(metricRows).every((row) => (
      row.children.length === 2
      && row.children[0].tagName === 'DT'
      && row.children[1].tagName === 'DD'
    ))).toBe(true);
  });

  it('aligns provider values into named columns and isolates notes from metric rows', async () => {
    await act(async () => root.render(<UsageDashboard statsVersion={0} displayMode="page" />));

    const table = container.querySelector('[role="table"][aria-label="Voice Query providers"]')!;
    expect(table).not.toBeNull();
    expect(table.querySelectorAll('[role="columnheader"]')).toHaveLength(4);
    const rows = table.querySelectorAll('[role="row"][data-provider]');
    expect(rows).toHaveLength(2);
    expect(Array.from(rows).every((row) => row.querySelectorAll('[role="cell"]').length === 4)).toBe(true);
    expect(container.querySelector('[data-query-note="failures"]')?.textContent).toContain('timed out 2');
    expect(container.querySelector('[data-query-note="privacy"]')?.textContent).toContain('never stored');
  });
});
