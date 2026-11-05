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

  it('uses three peer analytics sections', async () => {
    await act(async () => root.render(<UsageDashboard statsVersion={0} displayMode="page" />));

    expect(container.querySelectorAll('.usage-analytics-section')).toHaveLength(3);
  });
});
