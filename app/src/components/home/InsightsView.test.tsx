import { renderToStaticMarkup } from 'react-dom/server';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { loadStats } from '../../lib/stats';
vi.mock('../../lib/stats', async (original) => {
  const actual = await original<typeof import('../../lib/stats')>();
  return { ...actual, loadStats: vi.fn(actual.loadStats) };
});
import { InsightsView } from './InsightsView';

function tile(label: string) {
  const container = document.createElement('div');
  container.innerHTML = renderToStaticMarkup(<InsightsView statsVersion={0} onBackToHome={() => {}} />);
  const heading = Array.from(container.querySelectorAll('dt')).find((element) => element.textContent === label);
  if (!heading?.parentElement) throw new Error(`Missing tile: ${label}`);
  return heading.parentElement.textContent;
}

beforeEach(() => localStorage.clear());

describe('InsightsView transform tile', () => {

  it('displays the successful Paste last count', () => {
    const stats = loadStats(); stats.activity.pasteLastUses = 2;
    vi.mocked(loadStats).mockReturnValue(stats);
    expect(tile('Paste last')).toContain('2');
    expect(tile('Paste last')).toContain('successful uses');
  });
});
