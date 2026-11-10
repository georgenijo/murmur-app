import { renderToStaticMarkup } from 'react-dom/server';
import { beforeEach, describe, expect, it } from 'vitest';
import { InsightsView } from './InsightsView';
import { loadStats } from '../../lib/stats';

function render() {
  const container = document.createElement('div');
  container.innerHTML = renderToStaticMarkup(<InsightsView statsVersion={0} onBackToHome={() => {}} />);
  return container;
}

beforeEach(() => localStorage.clear());

describe('InsightsView', () => {
  it('renders four lifetime/recent headline figures and a real empty state without secondary statistics', () => {
    const container = render();
    expect(container.querySelectorAll('[data-stats="headline"] dt')).toHaveLength(4);
    expect(container.textContent).toContain('No dictations yet');
    expect(container.textContent).toContain('Dictate something and your activity will show up here.');
    expect(container.querySelector('.insights-more-statistics')).toBeNull();
    expect(container.querySelector('.insights-streak')).toBeNull();
    expect(container.querySelectorAll('.ui-day-chart-bar')).toHaveLength(0);
    expect(container.querySelectorAll('[data-intensity="0"]')).toHaveLength(57);
  });

  it('does not claim there have been no dictations when short recordings have no speed samples', () => {
    localStorage.setItem('dictation-stats', JSON.stringify({ ...loadStats(), totalRecordings: 1, totalWords: 1, totalDurationSeconds: 1 }));
    const container = render();
    expect(container.querySelector('[data-stats="headline"]')?.textContent).toContain('No speed samples yet');
    expect(container.querySelector('.insights-more-statistics')).toBeNull();
  });

  it('uses sanitized retained sample counts, omits zero rows and removes obsolete statistics', () => {
    localStorage.setItem('dictation-stats', JSON.stringify({ ...loadStats(), totalRecordings: 220,
      totalWords: 5168, totalDurationSeconds: 1640, wpmSamples: [184, 192, -1, null, 189, 191] }));
    const container = render();
    expect(container.textContent).toContain('Last 4 dictations');
    expect(container.textContent).toContain('102min');
    expect(container.querySelectorAll('[data-stats="details"] dt')).toHaveLength(1);
    expect(container.textContent).toContain('Total dictation time');
    for (const label of ['Paste last', 'Most-used mode', 'Meetings this month', 'Corrections taught']) {
      expect(container.textContent).not.toContain(label);
    }
    expect(container.querySelector('summary')?.textContent).toBe('More statistics');
    expect(container.querySelector('details')?.open).toBe(false);
  });
});
