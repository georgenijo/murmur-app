import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { UsageDashboard } from './UsageDashboard';

describe('UsageDashboard', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    localStorage.clear();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('uses a daily metric selector and calendar backed by accessible tables', async () => {
    await act(async () => root.render(<UsageDashboard statsVersion={0} displayMode="page" />));

    expect(container.querySelectorAll('.usage-analytics-section')).toHaveLength(2);
    expect(container.querySelectorAll('table')).toHaveLength(2);
    const buttons = container.querySelectorAll('.insights-chart-selector button');
    expect(buttons[0].getAttribute('aria-pressed')).toBe('true');
    await act(async () => (buttons[1] as HTMLButtonElement).click());
    expect(buttons[1].getAttribute('aria-pressed')).toBe('true');
    expect(container.querySelector('.insights-range')?.textContent).toContain('words per minute');
    expect(container.querySelector('figure')?.getAttribute('aria-label')).toContain('no dictations yet');
  });
});
