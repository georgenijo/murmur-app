import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { FooterStats } from './FooterStats';
import { dayKey, loadStats, saveStats } from '../lib/stats';

describe('FooterStats', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    localStorage.clear();
    const defaults = loadStats();
    saveStats({
      ...defaults,
      totalWords: 3344,
      totalRecordings: 125,
      totalDurationSeconds: 1000,
      wpmSamples: [200, 206],
      dailyBuckets: {
        [dayKey()]: { words: 200, recordings: 1, recordingSeconds: 60 },
      },
      query: {
        ...defaults.query,
        queriesRun: 2,
        successfulQueries: 1,
        failedQueries: 1,
        inputTokens: 120,
        outputTokens: 45,
        reportedCostUsd: 0.012,
        byProvider: {
          ...defaults.query.byProvider,
          claude: {
            queriesRun: 2,
            successfulQueries: 1,
            failedQueries: 1,
            inputTokens: 120,
            outputTokens: 45,
            reportedCostUsd: 0.012,
          },
        },
        failuresByErrorCode: { timed_out: 1 },
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

  it('renders compact usage text and opens detailed insights on demand', async () => {
    await act(async () => root.render(<FooterStats statsVersion={0} />));
    expect(container.textContent).toContain(`${(3344).toLocaleString()} words`);
    expect(container.textContent).toContain('203 wpm');
    expect(container.textContent).toContain('125 recordings');
    expect(container.textContent).toContain('1 day streak');

    const insights = Array.from(container.querySelectorAll('button')).find((button) => button.textContent?.includes('Insights')) as HTMLButtonElement;
    await act(async () => insights.click());
    expect(container.textContent).toContain('Usage insights');
    expect(container.textContent).toContain('Voice Query · all time');
    expect(container.textContent).toContain('120');
    expect(container.textContent).toContain('45');
    expect(container.textContent).toContain('Claude');
    expect(container.textContent).toContain('timed out 1');
    expect(container.textContent).toContain('Activity · last 8 weeks');
  });
});
