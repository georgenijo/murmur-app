import { renderToStaticMarkup } from 'react-dom/server';
import { beforeEach, describe, expect, it } from 'vitest';
import { updateActivityStats } from '../../lib/stats';
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
  it('shows the empty state alongside unchanged Mode and monthly totals', () => {
    expect(tile('Most-used transform')).toContain('None yet');
    expect(tile('Most-used transform')).toContain('0 runs');
    expect(tile('Most-used Mode')).toContain('None yet');
    expect(tile('Transforms this month')).toContain('0% approved');
  });

  it('displays the saved transform name and all-time run count', () => {
    updateActivityStats({ kind: 'transform', outcome: 'runs', presetName: 'Weekly notes' }, '2026-01');
    updateActivityStats({ kind: 'transform', outcome: 'runs', presetName: 'Weekly notes' });
    expect(tile('Most-used transform')).toContain('Weekly notes');
    expect(tile('Most-used transform')).toContain('2 runs');
  });
});
