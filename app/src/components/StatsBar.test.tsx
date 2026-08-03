import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { StatsBar } from './StatsBar';

describe('StatsBar settings visibility', () => {
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

  it('is visible in the main dashboard', async () => {
    await act(async () => root.render(<StatsBar statsVersion={0} />));

    expect((container.querySelector('[data-testid="stats-bar"]') as HTMLDivElement).hidden).toBe(false);
  });

  it('leaves the settings workspace free of usage cards', async () => {
    await act(async () => root.render(<StatsBar statsVersion={0} hidden />));

    expect((container.querySelector('[data-testid="stats-bar"]') as HTMLDivElement).hidden).toBe(true);
  });
});
