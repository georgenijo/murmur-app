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

  it('displays the successful Paste last count', () => {
    updateActivityStats({ kind: 'paste_last' });
    updateActivityStats({ kind: 'paste_last' });
    expect(tile('Paste last')).toContain('2');
    expect(tile('Paste last')).toContain('successful uses');
  });
});
