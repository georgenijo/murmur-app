import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { DaySummary } from '../../lib/stats';
import { DayChart } from './DayChart';

function day(index: number, overrides: Partial<DaySummary> = {}): DaySummary {
  const date = new Date(2026, 7, 2 + index);
  return {
    key: `2026-08-${String(2 + index).padStart(2, '0')}`,
    date,
    words: index * 100,
    recordings: index,
    recordingSeconds: index * 30,
    wpm: index === 0 ? 0 : 160 + index,
    ...overrides,
  };
}

describe('DayChart', () => {
  let container: HTMLDivElement;
  let root: Root;
  const days = Array.from({ length: 7 }, (_, index) => day(index));

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('keeps bars, tooltip, plot, and seven labels in separate regions', async () => {
    await act(async () => root.render(
      <DayChart kind="bars" metric="words" days={days} ariaLabel="Weekly words" highlightLast />,
    ));

    expect(container.querySelectorAll('.ui-day-chart-bar')).toHaveLength(7);
    expect(container.querySelectorAll('.ui-day-chart-axis span')).toHaveLength(7);
    const last = container.querySelectorAll('.ui-day-chart-bar')[6] as HTMLButtonElement;
    expect(last.dataset.highlighted).toBe('true');
    await act(async () => last.click());
    expect(container.querySelector('.ui-day-chart-tooltip')?.textContent).toContain('600 words');
    expect(last.getAttribute('aria-label')).toContain('6 recordings');
  });

  it('renders a keyboard target and explicit no-data label for every WPM day', async () => {
    await act(async () => root.render(
      <DayChart kind="line" metric="wpm" days={days} ariaLabel="WPM trend" />,
    ));

    const targets = container.querySelectorAll('.ui-day-chart-line-targets button');
    expect(targets).toHaveLength(7);
    expect(targets[0].getAttribute('aria-label')).toContain('No WPM data');
    await act(async () => (targets[1] as HTMLButtonElement).focus());
    expect(container.querySelector('.ui-day-chart-tooltip')?.textContent).toContain('161 WPM');
    await act(async () => targets[1].dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
    expect(container.querySelector('.ui-day-chart-tooltip')?.textContent).toContain('Focus a day');
  });

  it('makes every heatmap cell focusable with bounded intensity and exact values', async () => {
    const weeks = Array.from({ length: 8 }, (_, week) => (
      Array.from({ length: 7 }, (_, weekday) => day(week * 7 + weekday, {
        key: `day-${week}-${weekday}`,
        date: new Date(2026, 6, 5 + week * 7 + weekday),
      }))
    ));
    await act(async () => root.render(
      <DayChart kind="heatmap" metric="words" weeks={weeks} ariaLabel="Activity heatmap" />,
    ));

    const cells = container.querySelectorAll('.ui-day-chart-heatmap button');
    expect(cells).toHaveLength(56);
    expect(Array.from(cells).every((cell) => /^[0-4]$/.test((cell as HTMLElement).dataset.intensity ?? ''))).toBe(true);
    await act(async () => (cells[10] as HTMLButtonElement).click());
    const tooltip = container.querySelector('.ui-day-chart-tooltip')?.textContent ?? '';
    expect(tooltip).toContain('Jul 15, 2026');
    expect(tooltip).toContain('1,000 words');
    expect(tooltip).toContain('10 recordings');
    await act(async () => document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true })));
    expect(container.querySelector('.ui-day-chart-tooltip')?.textContent).toContain('Focus a day');
  });
});
